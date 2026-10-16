//! Transport (`transport.rs`) + SSH handshake with strict host-key verification: an unknown or
//! changed key never connects without the user's explicit approval.
//!
//! russh calls [`Client::check_server_key`] inside its connection future; the handler forwards
//! the key to [`connect`], which drives that future and answers from `known_hosts` or a prompt
//! (awaited on the session's own input channel, never blocking a worker). After the first accepted
//! key, re-key exchanges must present the same key or are refused without asking.

use std::borrow::Cow;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use fogu_core::EffectiveSsh;
use russh::client;
use russh::keys::{Algorithm, PublicKey, PublicKeyOrCertificate};
use tokio::sync::{mpsc, oneshot};
use tokio::time::{Instant, sleep_until};

use super::hostkeys::{self, Lookup};
use super::transport::{self, Route};
use super::{SshPaths, Stop, Ui};
use crate::{HostKeyFailure, PromptKind, PromptReply, SessionError, SessionOutput, SessionWarning};

/// Messages from the russh handler (running inside russh's session task) to our task.
pub(crate) enum HandlerEvent {
    /// Decide on this server key; answer `true` to accept.
    HostKey(Box<PublicKey>, oneshot::Sender<bool>),
    /// The server's auth banner (raw; sanitized when shown).
    Banner(String),
}

/// Our russh client handler.
pub(crate) struct Client {
    events: mpsc::Sender<HandlerEvent>,
    /// The key accepted for this connection; re-keys must match it.
    accepted: Option<PublicKey>,
}

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let PublicKeyOrCertificate::PublicKey { key, .. } = key else {
            // We never advertise certificate host-key algorithms; refuse one if sent anyway.
            tracing::warn!("server presented a host certificate; refused");
            return Ok(false);
        };
        if let Some(accepted) = &self.accepted {
            let same = accepted.key_data() == key.key_data();
            if !same {
                tracing::warn!("host key changed during re-key; disconnecting");
            }
            return Ok(same);
        }
        let (tx, rx) = oneshot::channel();
        if self
            .events
            .send(HandlerEvent::HostKey(Box::new(key.clone()), tx))
            .await
            .is_err()
        {
            return Ok(false);
        }
        let ok = rx.await.unwrap_or(false);
        if ok {
            self.accepted = Some(key.clone());
        }
        Ok(ok)
    }

    fn auth_banner(
        &mut self,
        banner: &str,
        _session: &mut client::Session,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        // Dropped if our task is busy and the queue is full: a banner is informational.
        let _full = self
            .events
            .try_send(HandlerEvent::Banner(banner.to_owned()));
        std::future::ready(Ok(()))
    }
}

/// An established, not yet authenticated connection.
pub(crate) struct Connection {
    pub(crate) handle: client::Handle<Client>,
    /// Banners (and refused late host-key checks) arriving after the handshake.
    pub(crate) events: mpsc::Receiver<HandlerEvent>,
    /// The jump host connection / proxy command under this one; declared last, so it is
    /// dropped (closed) after `handle`.
    pub(crate) route: Route,
}

impl Connection {
    /// Shows queued banners (unless `skip_banner`); answers any late host-key check with "no".
    pub(crate) async fn drain_events(
        &mut self,
        ssh: &EffectiveSsh,
        ui: &mut Ui,
    ) -> Result<(), Stop> {
        while let Ok(event) = self.events.try_recv() {
            handle_late_event(event, ssh, ui).await?;
        }
        Ok(())
    }
}

async fn handle_late_event(
    event: HandlerEvent,
    ssh: &EffectiveSsh,
    ui: &mut Ui,
) -> Result<(), Stop> {
    match event {
        HandlerEvent::Banner(text) if !ssh.skip_banner => ui.banner(&text).await?,
        HandlerEvent::Banner(_) => {}
        HandlerEvent::HostKey(_, reply) => {
            let _closed = reply.send(false);
        }
    }
    Ok(())
}

/// Connects to `ssh.host:ssh.port` along the profile's route, verifying the host key.
/// `ready` bounds each network step (time spent waiting for the user's answers is not
/// counted). `depth` = jump hosts already on the way.
pub(crate) async fn connect(
    ssh: &EffectiveSsh,
    paths: &SshPaths,
    ui: &mut Ui,
    ready: Duration,
    depth: usize,
) -> Result<Connection, Stop> {
    if ssh.host.is_empty() {
        return Err(SessionError::NoHost.into());
    }
    let problems = ssh.route_problems();
    if !problems.is_empty() {
        let text = crate::sanitize_remote(&problems.join("; "), 300, false);
        return Err(SessionError::Route(text).into());
    }
    ui.service(&format!(
        "Connecting to {}:{}{}…",
        ssh.host,
        ssh.port,
        transport::describe(ssh)
    ))
    .await?;
    let (stream, mut route) = transport::open(ssh, paths, ui, ready, depth).await?;
    let files = paths.files();
    let config = Arc::new(config(
        ssh,
        &hostkeys::known_algorithms(&files, &ssh.host, ssh.port),
    ));
    let (tx, mut events) = mpsc::channel(8);
    let handler = Client {
        events: tx,
        accepted: None,
    };
    let fut = client::connect_stream(config, stream, handler);
    tokio::pin!(fut);
    let mut deadline = Instant::now() + ready;
    let mut refused: Option<HostKeyFailure> = None;
    loop {
        tokio::select! {
            result = &mut fut => {
                return match result {
                    Ok(handle) => Ok(Connection { handle, events, route }),
                    Err(err) => {
                        let proxy = match refused {
                            None => route.proxy_failure().await,
                            Some(_) => None,
                        };
                        Err(proxy.unwrap_or_else(|| connect_error(err, refused, ssh)).into())
                    }
                };
            }
            Some(event) = events.recv() => match event {
                HandlerEvent::HostKey(key, reply) => {
                    let started = Instant::now();
                    let verdict = verify(ssh, paths, ui, &key).await;
                    deadline += started.elapsed();
                    match verdict {
                        Ok(()) => {
                            let _closed = reply.send(true);
                        }
                        Err(Stop::Failed(SessionError::HostKey(failure))) => {
                            refused = Some(failure);
                            let _closed = reply.send(false);
                        }
                        Err(stop) => return Err(stop),
                    }
                }
                banner @ HandlerEvent::Banner(_) => handle_late_event(banner, ssh, ui).await?,
            },
            input = ui.in_rx.recv() => ui.idle_input(input.as_ref())?,
            () = sleep_until(deadline) => {
                let proxy = route.proxy_failure().await;
                return Err(proxy.unwrap_or(SessionError::Timeout).into());
            }
        }
    }
}

/// [`connect`] boxed, for the jump-host recursion (`transport` → `connect` → `transport`).
pub(crate) fn connect_boxed<'a>(
    ssh: &'a EffectiveSsh,
    paths: &'a SshPaths,
    ui: &'a mut Ui,
    ready: Duration,
    depth: usize,
) -> Pin<Box<dyn Future<Output = Result<Connection, Stop>> + Send + 'a>> {
    Box::pin(connect(ssh, paths, ui, ready, depth))
}

/// The transport config: keepalives from the profile, `TCP_NODELAY`, and the host-key
/// algorithms we already have records for moved to the front (OpenSSH does the same), so a
/// server with several host keys presents the one we know instead of a false "changed".
fn config(ssh: &EffectiveSsh, known: &[Algorithm]) -> client::Config {
    let mut preferred = russh::Preferred::default();
    if !known.is_empty() {
        let is_known = |alg: &Algorithm| {
            known.iter().any(|k| {
                k == alg || matches!((k, alg), (Algorithm::Rsa { .. }, Algorithm::Rsa { .. }))
            })
        };
        let mut keys = preferred.key.to_vec();
        keys.sort_by_key(|alg| !is_known(alg));
        preferred.key = Cow::Owned(keys);
    }
    client::Config {
        keepalive_interval: ssh.keepalive_interval,
        keepalive_max: ssh.keepalive_count_max as usize,
        nodelay: true,
        preferred,
        ..client::Config::default()
    }
}

/// Maps a failed handshake: a refused key reports why we refused it, an ended proxy command
/// what it said.
fn connect_error(
    err: russh::Error,
    refused: Option<HostKeyFailure>,
    ssh: &EffectiveSsh,
) -> SessionError {
    match (err, refused) {
        (_, Some(failure)) => SessionError::HostKey(failure),
        (russh::Error::IO(source), None) => SessionError::Connect {
            target: crate::sanitize_remote(&format!("{}:{}", ssh.host, ssh.port), 300, false),
            source: Arc::new(source),
        },
        (russh::Error::UnknownKey, None) => SessionError::HostKey(HostKeyFailure::Certificate),
        (err, None) => ssh_error(&err),
    }
}

/// Any other russh error as sanitized text (some variants quote server-sent names).
pub(crate) fn ssh_error(err: &dyn std::fmt::Display) -> SessionError {
    SessionError::Ssh(crate::sanitize_remote(&err.to_string(), 300, false))
}

/// Decides on `key`: `known_hosts`, else a prompt. `Err(HostKey(..))` = refuse.
async fn verify(
    ssh: &EffectiveSsh,
    paths: &SshPaths,
    ui: &mut Ui,
    key: &PublicKey,
) -> Result<(), Stop> {
    let fingerprint = hostkeys::fingerprint(key);
    let key_type = key.algorithm().as_str().to_owned();
    if !ssh.verify_host_keys {
        tracing::warn!(%fingerprint, "host key NOT verified (verify_host_keys = false)");
        ui.service(&format!(
            "Host key NOT verified (verify_host_keys = false): {key_type} {fingerprint}"
        ))
        .await?;
        ui.pump
            .emit(SessionOutput::Warning(SessionWarning::HostKeyNotVerified))
            .await?;
        return Ok(());
    }
    let refuse = |failure| Err(Stop::Failed(SessionError::HostKey(failure)));
    match hostkeys::lookup(&paths.files(), &ssh.host, ssh.port, key) {
        Lookup::Known => Ok(()),
        Lookup::Revoked { path, line } => {
            tracing::warn!(path = %path.display(), line, "revoked host key offered");
            refuse(HostKeyFailure::Revoked)
        }
        Lookup::Unknown => {
            let kind = PromptKind::HostKeyUnknown {
                host: ssh.host.clone(),
                port: ssh.port,
                key_type,
                fingerprint,
            };
            match ui.ask(kind).await? {
                PromptReply::Accept => Ok(()),
                PromptReply::AcceptAndSave => {
                    if let Err(err) = hostkeys::learn(&paths.known_hosts, &ssh.host, ssh.port, key)
                    {
                        tracing::warn!(error = %err, "could not save the host key");
                        ui.service(&format!("Could not save the host key: {err}"))
                            .await?;
                    }
                    Ok(())
                }
                _ => refuse(HostKeyFailure::Rejected),
            }
        }
        Lookup::Changed { path, line, old } => {
            tracing::warn!(path = %path.display(), line, %fingerprint, "HOST KEY CHANGED");
            ui.service(&format!(
                "WARNING: the host key of {}:{} has CHANGED ({}:{line}). Someone could be \
                 eavesdropping on you (man-in-the-middle attack).",
                ssh.host,
                ssh.port,
                path.display()
            ))
            .await?;
            let kind = PromptKind::HostKeyChanged {
                host: ssh.host.clone(),
                port: ssh.port,
                key_type,
                fingerprint,
                old_fingerprint: hostkeys::fingerprint(&old),
                known_hosts_path: path,
                line,
            };
            // Only the explicit replace flow continues; a plain Accept is a refusal.
            match ui.ask(kind).await? {
                PromptReply::AcceptAndSave => {
                    match hostkeys::replace(&paths.known_hosts, &ssh.host, ssh.port, key) {
                        Ok(()) => Ok(()),
                        Err(err) => {
                            tracing::warn!(error = %err, "could not replace the host key");
                            ui.service(&format!("Could not replace the host key: {err}"))
                                .await?;
                            refuse(HostKeyFailure::Changed)
                        }
                    }
                }
                _ => refuse(HostKeyFailure::Changed),
            }
        }
    }
}
