//! Authentication order:
//! `none` → agent identities (those whose `.pub` sits next to a configured key first) → private
//! key files (passphrase prompt for encrypted ones; OpenSSH/PEM/PKCS#8/PuTTY) → password prompt →
//! keyboard-interactive (prompts forwarded with their echo flags), filtered by the profile's
//! `auth`. Methods the server no longer lists are skipped; a failing method is logged and the
//! next one tried; all failed → `SessionError::AuthFailed`.
//!
//! Secrets: passwords, passphrases, answers and key-file text are `SecretString`s, exposed only
//! as the argument of the russh / key-decoding call. russh copies a password into its own
//! `String` for the packet (not zeroized; outside our control).

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use fogu_core::{EffectiveSsh, SshAuth};
use russh::client::KeyboardInteractiveAuthResponse as Kbd;
use russh::keys::agent::AgentIdentity;
use russh::keys::agent::client::AgentClient;
use russh::keys::{Algorithm, HashAlg, PrivateKey, PrivateKeyWithHashAlg, PublicKey};
use russh::{MethodKind, MethodSet, client::AuthResult};
use secrecy::{ExposeSecret as _, SecretString};

use super::connect::{Connection, ssh_error};
use super::hostkeys::fingerprint;
use super::{Stop, Ui};
use crate::prompt::{
    KbdPrompt, PROMPT_INSTRUCTIONS_MAX_CHARS, PROMPT_MAX_COUNT, PROMPT_TEXT_MAX_CHARS,
    sanitize_remote,
};
use crate::{PromptKind, PromptReply, SessionError};

/// One step of the plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Method {
    None,
    Agent,
    PublicKey,
    Password,
    KeyboardInteractive,
}

/// Password prompts per connection (OpenSSH's `NumberOfPasswordPrompts`).
const PASSWORD_ATTEMPTS: usize = 3;
/// Passphrase prompts per key file.
const PASSPHRASE_ATTEMPTS: usize = 3;
/// Keyboard-interactive rounds before giving up on the method (a hostile server could loop).
const KBD_MAX_ROUNDS: usize = 10;
/// Private key files larger than this are not read.
const MAX_KEY_FILE_BYTES: u64 = 1024 * 1024;

/// The methods tried for `auth`, in order.
pub(crate) fn plan(auth: SshAuth) -> &'static [Method] {
    use Method::{Agent, KeyboardInteractive, None, Password, PublicKey};
    match auth {
        SshAuth::Auto => &[None, Agent, PublicKey, Password, KeyboardInteractive],
        SshAuth::Password => &[None, Password],
        SshAuth::PublicKey => &[None, PublicKey],
        SshAuth::Agent => &[None, Agent],
        SshAuth::KeyboardInteractive => &[None, KeyboardInteractive],
    }
}

/// The protocol method behind a step (`None` is always attempted).
fn kind(method: Method) -> Option<MethodKind> {
    match method {
        Method::None => None,
        Method::Agent | Method::PublicKey => Some(MethodKind::PublicKey),
        Method::Password => Some(MethodKind::Password),
        Method::KeyboardInteractive => Some(MethodKind::KeyboardInteractive),
    }
}

/// `false` when the server's last failure response no longer lists `method`.
pub(crate) fn server_allows(allowed: Option<&MethodSet>, method: Method) -> bool {
    match (allowed, kind(method)) {
        (Some(set), Some(kind)) if !set.is_empty() => set.contains(&kind),
        _ => true,
    }
}

enum Outcome {
    Success,
    /// Failed; the server's remaining methods (may be empty).
    Failure(MethodSet),
    /// Not attempted (no agent, no keys, user declined).
    Skipped,
}

impl From<AuthResult> for Outcome {
    fn from(result: AuthResult) -> Self {
        match result {
            AuthResult::Success => Self::Success,
            AuthResult::Failure {
                remaining_methods, ..
            } => Self::Failure(remaining_methods),
        }
    }
}

/// Context for one authentication run.
struct Auth<'a> {
    conn: &'a mut Connection,
    ssh: &'a EffectiveSsh,
    ui: &'a mut Ui,
    ready: Duration,
}

/// Runs the plan until one method succeeds.
pub(crate) async fn authenticate(
    conn: &mut Connection,
    ssh: &EffectiveSsh,
    ui: &mut Ui,
    ready: Duration,
) -> Result<(), Stop> {
    let mut auth = Auth {
        conn,
        ssh,
        ui,
        ready,
    };
    let mut allowed: Option<MethodSet> = None;
    for &method in plan(ssh.auth) {
        if !server_allows(allowed.as_ref(), method) {
            tracing::debug!(?method, "skipped: not offered by the server");
            continue;
        }
        let outcome = match method {
            Method::None => auth.none().await?,
            Method::Agent => auth.agent().await?,
            Method::PublicKey => auth.keys().await?,
            Method::Password => auth.password(&mut allowed).await?,
            Method::KeyboardInteractive => auth.keyboard_interactive().await?,
        };
        auth.conn.drain_events(ssh, auth.ui).await?;
        match outcome {
            Outcome::Success => {
                tracing::info!(?method, "authenticated");
                return Ok(());
            }
            Outcome::Failure(remaining) => {
                tracing::info!(?method, "authentication method failed");
                if !remaining.is_empty() {
                    allowed = Some(remaining);
                }
            }
            Outcome::Skipped => {}
        }
    }
    Err(SessionError::AuthFailed.into())
}

/// Awaits a russh call under `ready_timeout`.
async fn call<T, E: std::fmt::Display>(
    ready: Duration,
    fut: impl Future<Output = Result<T, E>>,
) -> Result<T, Stop> {
    match tokio::time::timeout(ready, fut).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(err)) => Err(ssh_error(&err).into()),
        Err(_elapsed) => Err(SessionError::Timeout.into()),
    }
}

impl Auth<'_> {
    async fn none(&mut self) -> Result<Outcome, Stop> {
        let user = self.ssh.user.clone();
        let fut = self.conn.handle.authenticate_none(user);
        Ok(call(self.ready, fut).await?.into())
    }

    /// The RSA signature hash the server accepts (`None` = legacy `ssh-rsa`).
    /// RSA client keys are refused visibly: the `rsa` crate is not built in (RUSTSEC-2023-0071,
    /// non-constant-time private-key operations, no fixed release).
    async fn refuse_rsa(&mut self, path: &Path) -> Result<Option<PrivateKey>, Stop> {
        tracing::warn!(path = %path.display(), "RSA private key refused (RUSTSEC-2023-0071)");
        self.ui
            .service(&format!(
                "Skipping RSA key {}: RSA private keys are not supported (RUSTSEC-2023-0071); \
                 use an ed25519 or ecdsa key",
                path.display()
            ))
            .await?;
        Ok(None)
    }

    async fn rsa_hash(&self, algorithm: &Algorithm) -> Result<Option<HashAlg>, Stop> {
        if !matches!(algorithm, Algorithm::Rsa { .. }) {
            return Ok(None);
        }
        Ok(call(self.ready, self.conn.handle.best_supported_rsa_hash())
            .await?
            .flatten())
    }

    async fn agent(&mut self) -> Result<Outcome, Stop> {
        let connected = match &self.ssh.agent_path {
            Some(path) => AgentClient::connect_uds(expand_home(path)).await,
            None => AgentClient::connect_env().await,
        };
        let mut agent = match connected {
            Ok(agent) => agent,
            Err(err) => {
                tracing::debug!(error = %err, "no SSH agent");
                return Ok(Outcome::Skipped);
            }
        };
        let identities = match call(self.ready, agent.request_identities()).await {
            Ok(ids) => ids,
            Err(Stop::Failed(err)) => {
                tracing::warn!(error = %err, "SSH agent: listing identities failed");
                return Ok(Outcome::Skipped);
            }
            Err(stop) => return Err(stop),
        };
        let preferred = configured_public_keys(&self.ssh.private_keys);
        let mut keys: Vec<PublicKey> = identities
            .into_iter()
            .filter_map(|id| match id {
                AgentIdentity::PublicKey { key, .. } => Some(key),
                AgentIdentity::Certificate { .. } => None,
            })
            .collect();
        let is_configured =
            |key: &PublicKey| preferred.iter().any(|p| p.key_data() == key.key_data());
        if self.ssh.identities_only {
            keys.retain(is_configured);
        }
        keys.sort_by_key(|key| !is_configured(key));
        let mut last = Outcome::Skipped;
        for key in keys {
            self.ui
                .service(&format!("Trying agent key {}", fingerprint(&key)))
                .await?;
            let hash = self.rsa_hash(&key.algorithm()).await?;
            let user = self.ssh.user.clone();
            let fut = self
                .conn
                .handle
                .authenticate_publickey_with(user, key, hash, &mut agent);
            match tokio::time::timeout(self.ready, fut).await {
                Ok(Ok(result)) => match Outcome::from(result) {
                    Outcome::Success => return Ok(Outcome::Success),
                    failure => last = failure,
                },
                Ok(Err(err)) => {
                    tracing::warn!(error = %ssh_error(&err), "SSH agent signing failed");
                }
                Err(_elapsed) => return Err(SessionError::Timeout.into()),
            }
        }
        Ok(last)
    }

    async fn keys(&mut self) -> Result<Outcome, Stop> {
        let mut last = Outcome::Skipped;
        for path in self.ssh.private_keys.clone() {
            let path = expand_home(&path);
            let Some(key) = self.load_key(&path).await? else {
                continue;
            };
            self.ui
                .service(&format!("Trying private key {}", path.display()))
                .await?;
            let hash = self.rsa_hash(&key.algorithm()).await?;
            let key = PrivateKeyWithHashAlg::new(Arc::new(key), hash);
            let user = self.ssh.user.clone();
            let result = call(
                self.ready,
                self.conn.handle.authenticate_publickey(user, key),
            )
            .await?;
            match Outcome::from(result) {
                Outcome::Success => return Ok(Outcome::Success),
                failure => last = failure,
            }
        }
        Ok(last)
    }

    /// Reads and decodes a key file, prompting for its passphrase when it is encrypted.
    /// `None` = skip this key (unreadable, wrong passphrase, declined).
    async fn load_key(&mut self, path: &Path) -> Result<Option<PrivateKey>, Stop> {
        let text = match read_key_file(path) {
            Ok(text) => text,
            Err(err) => {
                tracing::warn!(path = %path.display(), error = %err, "private key unreadable");
                self.ui
                    .service(&format!(
                        "Cannot read private key {}: {err}",
                        path.display()
                    ))
                    .await?;
                return Ok(None);
            }
        };
        // OpenSSH files carry the public half unencrypted: refuse RSA before asking a passphrase.
        if russh::keys::ssh_key::PrivateKey::from_openssh(text.expose_secret())
            .is_ok_and(|key| is_rsa(&key.algorithm()))
        {
            return self.refuse_rsa(path).await;
        }
        let first = russh::keys::decode_secret_key(text.expose_secret(), None);
        let err = match first {
            Ok(key) if is_rsa(&key.algorithm()) => return self.refuse_rsa(path).await,
            Ok(key) => return Ok(Some(key)),
            Err(err) if needs_passphrase(text.expose_secret(), &err) => err,
            Err(err) => {
                self.ui
                    .service(&format!(
                        "Cannot load private key {}: {err}",
                        path.display()
                    ))
                    .await?;
                return Ok(None);
            }
        };
        tracing::debug!(path = %path.display(), error = %err, "key is encrypted");
        for _ in 0..PASSPHRASE_ATTEMPTS {
            let kind = PromptKind::KeyPassphrase {
                path: path.to_path_buf(),
            };
            let PromptReply::Secret(passphrase) = self.ui.ask(kind).await? else {
                return Ok(None);
            };
            match russh::keys::decode_secret_key(
                text.expose_secret(),
                Some(passphrase.expose_secret()),
            ) {
                Ok(key) if is_rsa(&key.algorithm()) => return self.refuse_rsa(path).await,
                Ok(key) => return Ok(Some(key)),
                Err(_) => self.ui.service("Wrong passphrase").await?,
            }
        }
        Ok(None)
    }

    async fn password(&mut self, allowed: &mut Option<MethodSet>) -> Result<Outcome, Stop> {
        let mut last = Outcome::Skipped;
        for _ in 0..PASSWORD_ATTEMPTS {
            if !server_allows(allowed.as_ref(), Method::Password) {
                break;
            }
            let kind = PromptKind::Password {
                user: self.ssh.user.clone(),
                host: self.ssh.host.clone(),
            };
            let PromptReply::Secret(password) = self.ui.ask(kind).await? else {
                return Ok(Outcome::Skipped);
            };
            let user = self.ssh.user.clone();
            let fut = self
                .conn
                .handle
                .authenticate_password(user, password.expose_secret());
            match Outcome::from(call(self.ready, fut).await?) {
                Outcome::Success => return Ok(Outcome::Success),
                Outcome::Failure(remaining) => {
                    self.ui.service("Password rejected").await?;
                    if !remaining.is_empty() {
                        *allowed = Some(remaining.clone());
                    }
                    last = Outcome::Failure(remaining);
                }
                Outcome::Skipped => {}
            }
        }
        Ok(last)
    }

    async fn keyboard_interactive(&mut self) -> Result<Outcome, Stop> {
        let user = self.ssh.user.clone();
        let fut = self
            .conn
            .handle
            .authenticate_keyboard_interactive_start(user, None);
        let mut response = call(self.ready, fut).await?;
        for _ in 0..KBD_MAX_ROUNDS {
            let (name, instructions, prompts) = match response {
                Kbd::Success => return Ok(Outcome::Success),
                Kbd::Failure {
                    remaining_methods, ..
                } => return Ok(Outcome::Failure(remaining_methods)),
                Kbd::InfoRequest {
                    name,
                    instructions,
                    prompts,
                } => (name, instructions, prompts),
            };
            if prompts.len() > PROMPT_MAX_COUNT {
                tracing::warn!(
                    count = prompts.len(),
                    "too many keyboard-interactive prompts"
                );
                return Ok(Outcome::Failure(MethodSet::empty()));
            }
            let answers: Vec<String> = if prompts.is_empty() {
                Vec::new()
            } else {
                let kind = kbd_prompt(&name, &instructions, &prompts);
                match self.ui.ask(kind).await? {
                    PromptReply::Secrets(answers) if answers.len() == prompts.len() => answers
                        .iter()
                        .map(|a| a.expose_secret().to_owned())
                        .collect(),
                    _ => return Ok(Outcome::Skipped),
                }
            };
            let fut = self
                .conn
                .handle
                .authenticate_keyboard_interactive_respond(answers);
            response = call(self.ready, fut).await?;
        }
        Ok(Outcome::Failure(MethodSet::empty()))
    }
}

/// The UI prompt for one keyboard-interactive round, server texts sanitized and capped.
fn kbd_prompt(name: &str, instructions: &str, prompts: &[russh::client::Prompt]) -> PromptKind {
    PromptKind::KeyboardInteractive {
        name: sanitize_remote(name, PROMPT_TEXT_MAX_CHARS, false),
        instructions: sanitize_remote(instructions, PROMPT_INSTRUCTIONS_MAX_CHARS, true),
        prompts: prompts
            .iter()
            .map(|p| KbdPrompt {
                text: sanitize_remote(&p.prompt, PROMPT_TEXT_MAX_CHARS, false),
                echo: p.echo,
            })
            .collect(),
    }
}

/// `~/x` → `$HOME/x`.
fn expand_home(path: &Path) -> PathBuf {
    match (path.strip_prefix("~"), std::env::var_os("HOME")) {
        (Ok(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => path.to_path_buf(),
    }
}

/// Public halves (`<key>.pub`) of the configured keys, for agent identity ordering.
fn configured_public_keys(keys: &[PathBuf]) -> Vec<PublicKey> {
    keys.iter()
        .filter_map(|path| {
            let path = expand_home(path);
            let pub_path = if path.extension().is_some_and(|e| e == "pub") {
                path
            } else {
                let mut name = path.into_os_string();
                name.push(".pub");
                PathBuf::from(name)
            };
            russh::keys::load_public_key(&pub_path).ok()
        })
        .collect()
}

/// A key file's text, as a secret (it may be an unencrypted private key).
fn read_key_file(path: &Path) -> std::io::Result<SecretString> {
    let file = std::fs::File::open(path)?;
    let mut text = String::new();
    file.take(MAX_KEY_FILE_BYTES).read_to_string(&mut text)?;
    Ok(SecretString::from(text))
}

/// Whether a failed passphrase-less decode means "encrypted": russh says so for OpenSSH/PEM;
/// encrypted PKCS#8 and `PuTTY` files are recognised by their headers.
fn is_rsa(algorithm: &Algorithm) -> bool {
    matches!(algorithm, Algorithm::Rsa { .. })
}

fn needs_passphrase(text: &str, err: &russh::keys::Error) -> bool {
    matches!(err, russh::keys::Error::KeyIsEncrypted)
        || text.contains("-----BEGIN ENCRYPTED PRIVATE KEY-----")
        || (text.trim_start().starts_with("PuTTY-User-Key-File-")
            && text
                .lines()
                .any(|l| l.starts_with("Encryption:") && !l.ends_with("none")))
}

#[cfg(test)]
#[path = "auth_tests.rs"]
mod tests;
