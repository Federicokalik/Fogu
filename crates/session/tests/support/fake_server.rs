//! A minimal in-process SSH server (russh's server half) for the authentication methods a
//! non-root `sshd` cannot check: password and keyboard-interactive need PAM or `/etc/shadow`,
//! which a hermetic test must not touch. It exercises our *client* against a second SSH
//! implementation, but it is not OpenSSH: PAM's wording and flows were verified by hand.
//!
//! Accepts password [`PASSWORD`], or the keyboard-interactive answers [`KBD_CODE`] (echoed) and
//! [`KBD_SECRET`] (hidden); then offers a one-line "shell" that echoes input and ends on
//! `exit\r`. The host key is a fixed throwaway ed25519 key derived from an all-7 seed.

use std::borrow::Cow;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use russh::keys::PrivateKey;
use russh::keys::ssh_key::private::Ed25519Keypair;
use russh::server::{Auth, Config, Handler, Msg, Response, Server, Session};
use russh::{Channel, ChannelId, Disconnect, MethodKind, MethodSet};
use tokio::net::TcpListener;
use tokio::runtime::{Builder, Runtime};

pub const PASSWORD: &str = "correct horse";
pub const KBD_CODE: &str = "1234";
pub const KBD_SECRET: &str = "hunter2";
/// Printed by the fake shell when it starts.
pub const READY: &str = "fake-shell-ready";

/// The running server (on its own runtime); stops when dropped.
pub struct FakeServer {
    pub port: u16,
    _rt: Runtime,
}

impl FakeServer {
    /// Listens on a free port of 127.0.0.1.
    pub fn start() -> Self {
        Self::start_with(false)
    }

    /// `drop_after_auth`: the server disconnects right after a successful authentication (a
    /// hostile or broken server).
    pub fn start_with(drop_after_auth: bool) -> Self {
        let rt = Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("server runtime");
        let port = rt.block_on(async {
            let socket = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
                .await
                .expect("bind");
            let port = socket.local_addr().expect("addr").port();
            let seed = [7u8; 32];
            let config = Config {
                methods: MethodSet::from(
                    &[MethodKind::Password, MethodKind::KeyboardInteractive][..],
                ),
                keys: vec![PrivateKey::from(Ed25519Keypair::from_seed(&seed))],
                // Rejections are delayed to blunt guessing; tests don't need that.
                auth_rejection_time: Duration::from_millis(10),
                auth_rejection_time_initial: Some(Duration::ZERO),
                ..Config::default()
            };
            let config = Arc::new(config);
            tokio::spawn(async move {
                let mut server = Fake { drop_after_auth };
                let _ended = server.run_on_socket(config, &socket).await;
            });
            port
        });
        Self { port, _rt: rt }
    }
}

#[derive(Clone)]
struct Fake {
    drop_after_auth: bool,
}

impl Server for Fake {
    type Handler = Self;

    fn new_client(&mut self, _peer: Option<std::net::SocketAddr>) -> Self {
        self.clone()
    }
}

fn reject() -> Auth {
    Auth::Reject {
        // Like OpenSSH: the client may try again, with either method.
        proceed_with_methods: Some(MethodSet::from(
            &[MethodKind::Password, MethodKind::KeyboardInteractive][..],
        )),
        partial_success: false,
    }
}

impl Handler for Fake {
    type Error = russh::Error;

    async fn auth_succeeded(&mut self, session: &mut Session) -> Result<(), Self::Error> {
        if self.drop_after_auth {
            // Something a client must survive: the text is attacker-chosen too.
            session.disconnect(Disconnect::ByApplication, "bye\x1b[31m", "en")?;
        }
        Ok(())
    }

    async fn auth_password(&mut self, _user: &str, password: &str) -> Result<Auth, Self::Error> {
        Ok(if password == PASSWORD {
            Auth::Accept
        } else {
            reject()
        })
    }

    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        _user: &str,
        _submethods: &str,
        response: Option<Response<'a>>,
    ) -> Result<Auth, Self::Error> {
        let Some(response) = response else {
            return Ok(Auth::Partial {
                name: Cow::Borrowed("Test login"),
                // An escape sequence the client must not pass on to the user's screen.
                instructions: Cow::Borrowed("Answer both\x1b[31mred\x1b]0;pwned\x07"),
                prompts: Cow::Owned(vec![
                    (Cow::Borrowed("Verification code: "), true),
                    (Cow::Borrowed("Secret: "), false),
                ]),
            });
        };
        let answers: Vec<Vec<u8>> = response.map(|a| a.to_vec()).collect();
        let good = answers == [KBD_CODE.as_bytes(), KBD_SECRET.as_bytes()];
        Ok(if good { Auth::Accept } else { reject() })
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        reply: russh::server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _term: &str,
        _cols: u32,
        _rows: u32,
        _px_w: u32,
        _px_h: u32,
        _modes: &[(russh::Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        session.data(channel, format!("{READY}\r\n").into_bytes())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if data == b"exit\r" {
            session.exit_status_request(channel, 0)?;
            session.eof(channel)?;
            return session.close(channel);
        }
        session.data(channel, data.to_vec())
    }
}
