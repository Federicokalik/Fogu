//! SSH profile options: the stored partial form [`SshOpts`] and the resolved [`EffectiveSsh`].
//!
//! Stored fields are `Option<T>` like every other profile option: a profile inherits unset
//! fields from `profile_defaults.ssh`, which inherits from the builtin defaults below (`user`
//! defaults to `$USER`).
//! Options whose behaviour is not implemented yet are accepted and listed by
//! [`EffectiveSsh::unsupported`], which the session warns about once at connect.

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::profile::{Osc52Policy, ProfileId};

/// Default SSH port.
pub const DEFAULT_SSH_PORT: u16 = 22;
/// Default keepalive interval, in milliseconds.
pub const DEFAULT_KEEPALIVE_INTERVAL_MS: u64 = 5000;
/// Default number of unanswered keepalives before the connection is dropped.
pub const DEFAULT_KEEPALIVE_COUNT_MAX: u32 = 10;
/// Default PTY `TERM`.
pub const DEFAULT_TERM: &str = "xterm-256color";

/// Which authentication methods to try.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SshAuth {
    /// The full order: none → agent → private keys → password → keyboard-interactive.
    #[default]
    Auto,
    /// Password only.
    Password,
    /// Private key files only.
    PublicKey,
    /// SSH agent identities only.
    Agent,
    /// Keyboard-interactive only.
    KeyboardInteractive,
}

/// What the Backspace key sends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Backspace {
    /// The terminal's normal encoding (DEL, `^?`).
    #[default]
    Backspace,
    /// `^H`.
    CtrlH,
    /// `^?`.
    #[serde(rename = "ctrl-?")]
    CtrlQuestion,
    /// The Delete key sequence (`ESC [3~`).
    Delete,
}

/// Port forward direction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ForwardKind {
    /// `-L`: listen locally, connect from the server.
    #[default]
    Local,
    /// `-R`: listen on the server, connect locally.
    Remote,
    /// `-D`: local SOCKS5 server.
    Dynamic,
}

/// One auto-started port forward.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PortForward {
    /// Direction.
    #[serde(rename = "type")]
    pub kind: ForwardKind,
    /// Listen address.
    pub host: String,
    /// Listen port.
    pub port: u16,
    /// Target address (unused for `Dynamic`).
    pub target_address: String,
    /// Target port (unused for `Dynamic`).
    pub target_port: u16,
    /// Free text shown in the forwards dialog.
    pub description: String,
}

/// Per-profile algorithm allow-lists; empty list = russh's defaults.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Algorithms {
    /// Key exchange.
    pub kex: Vec<String>,
    /// Ciphers.
    pub cipher: Vec<String>,
    /// MACs.
    pub hmac: Vec<String>,
    /// Host key algorithms.
    pub server_host_key: Vec<String>,
    /// Compression.
    pub compression: Vec<String>,
}

impl Algorithms {
    /// `true` when no list overrides russh's defaults.
    #[must_use]
    pub fn is_default(&self) -> bool {
        [
            &self.kex,
            &self.cipher,
            &self.hmac,
            &self.server_host_key,
            &self.compression,
        ]
        .iter()
        .all(|list| list.is_empty())
    }
}

/// One login-script rule: when output matches `expect`, send `send`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LoginScript {
    /// Text (or regex) to wait for.
    pub expect: String,
    /// Text to send (escapes like `\n` are processed).
    pub send: String,
    /// `expect` is a regular expression.
    pub is_regex: bool,
    /// Skip this rule if it never matches.
    pub optional: bool,
}

/// Stored (partial) SSH options; see the module docs. Durations are milliseconds.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SshOpts {
    /// Hostname or address; required to connect (no builtin default).
    pub host: Option<String>,
    /// TCP port (22).
    pub port: Option<u16>,
    /// Remote user (`$USER` at resolve time).
    pub user: Option<String>,
    /// Auth method filter (`auto`).
    pub auth: Option<SshAuth>,
    /// Private key files, tried in order (OpenSSH, PKCS#8, PEM, `PuTTY` `.ppk`).
    pub private_keys: Option<Vec<PathBuf>>,
    /// Keepalive interval in ms (5000); 0 disables keepalives.
    pub keepalive_interval: Option<u64>,
    /// Unanswered keepalives before disconnecting (10).
    pub keepalive_count_max: Option<u32>,
    /// Connect + handshake + per-request auth timeout in ms (unset = 20 s).
    pub ready_timeout: Option<u64>,
    /// Don't print the server's auth banner.
    pub skip_banner: Option<bool>,
    /// Confirm before closing a live SSH tab or pane (unset = `true`).
    pub warn_on_close: Option<bool>,
    /// PTY `TERM` (`xterm-256color`).
    pub term: Option<String>,
    /// Backspace encoding.
    pub input_backspace: Option<Backspace>,
    /// Profile to jump through.
    pub jump_host: Option<ProfileId>,
    /// Command whose stdio is the transport, `%h`/`%p` substituted.
    pub proxy_command: Option<String>,
    /// SOCKS5 proxy host.
    pub socks_proxy_host: Option<String>,
    /// SOCKS5 proxy port.
    pub socks_proxy_port: Option<u16>,
    /// HTTP CONNECT proxy host.
    pub http_proxy_host: Option<String>,
    /// HTTP CONNECT proxy port.
    pub http_proxy_port: Option<u16>,
    /// Auto-started port forwards.
    pub forwarded_ports: Option<Vec<PortForward>>,
    /// Agent forwarding.
    pub agent_forward: Option<bool>,
    /// X11 forwarding.
    pub x11: Option<bool>,
    /// Algorithm allow-lists.
    pub algorithms: Option<Algorithms>,
    /// Open new tabs of this profile on the existing connection (`true`).
    pub reuse_session: Option<bool>,
    /// Login scripts.
    pub scripts: Option<Vec<LoginScript>>,
    /// Remote directory to `cd` into after login.
    pub cwd: Option<String>,
    /// Track the remote cwd and reuse it on reconnect/duplicate.
    pub remember_cwd: Option<bool>,
    /// Clear the "Connecting…" service lines once the shell starts (`true`).
    pub clear_service_messages_on_connect: Option<bool>,
    /// Verify host keys against `known_hosts` (`true`); `false` shows a warning badge.
    pub verify_host_keys: Option<bool>,
    /// Manual ordering in the profile list.
    pub weight: Option<i32>,
    /// Per-profile override of `terminal.osc52_write`.
    pub osc52_write: Option<Osc52Policy>,
    /// Offer only the agent identities whose `.pub` sits next to a configured key (OpenSSH
    /// `IdentitiesOnly`; `false`).
    pub identities_only: Option<bool>,
    /// `~/.ssh/config` options that apply to this host but are not used (unknown to Fogu,
    /// unusable values, or never obeyed like `StrictHostKeyChecking no`); listed in the
    /// connect-time warning. Only `ssh_config` profiles have them; never stored.
    #[serde(skip)]
    pub ignored_ssh_config: Vec<String>,
}

/// A fully resolved SSH profile, ready for `fogu_session::open`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "mirrors the flat SSH profile options one to one"
)]
pub struct EffectiveSsh {
    /// Hostname or address.
    pub host: String,
    /// TCP port.
    pub port: u16,
    /// Remote user.
    pub user: String,
    /// See [`SshOpts::auth`].
    pub auth: SshAuth,
    /// See [`SshOpts::private_keys`].
    pub private_keys: Vec<PathBuf>,
    /// `None` = keepalives disabled.
    pub keepalive_interval: Option<Duration>,
    /// See [`SshOpts::keepalive_count_max`].
    pub keepalive_count_max: u32,
    /// `None` = the session's default.
    pub ready_timeout: Option<Duration>,
    /// See [`SshOpts::skip_banner`].
    pub skip_banner: bool,
    /// See [`SshOpts::warn_on_close`].
    pub warn_on_close: bool,
    /// See [`SshOpts::term`].
    pub term: String,
    /// See [`SshOpts::input_backspace`].
    pub input_backspace: Backspace,
    /// See [`SshOpts::jump_host`].
    pub jump_host: Option<ProfileId>,
    /// See [`SshOpts::proxy_command`].
    pub proxy_command: Option<String>,
    /// SOCKS5 proxy `(host, port)`; port defaults to 1080.
    pub socks_proxy: Option<(String, u16)>,
    /// HTTP proxy `(host, port)`; port defaults to 8080.
    pub http_proxy: Option<(String, u16)>,
    /// See [`SshOpts::forwarded_ports`].
    pub forwarded_ports: Vec<PortForward>,
    /// See [`SshOpts::agent_forward`].
    pub agent_forward: bool,
    /// See [`SshOpts::x11`].
    pub x11: bool,
    /// See [`SshOpts::algorithms`].
    pub algorithms: Algorithms,
    /// See [`SshOpts::reuse_session`].
    pub reuse_session: bool,
    /// See [`SshOpts::scripts`].
    pub scripts: Vec<LoginScript>,
    /// See [`SshOpts::cwd`].
    pub cwd: Option<String>,
    /// See [`SshOpts::remember_cwd`].
    pub remember_cwd: bool,
    /// See [`SshOpts::clear_service_messages_on_connect`].
    pub clear_service_messages_on_connect: bool,
    /// See [`SshOpts::verify_host_keys`].
    pub verify_host_keys: bool,
    /// See [`SshOpts::weight`].
    pub weight: i32,
    /// See [`SshOpts::identities_only`].
    pub identities_only: bool,
    /// See [`SshOpts::ignored_ssh_config`].
    pub ignored_ssh_config: Vec<String>,
}

impl EffectiveSsh {
    /// A profile for `host` with every other option at its builtin default.
    #[must_use]
    pub fn new(host: impl Into<String>) -> Self {
        let mut ssh = resolve_ssh(&SshOpts::default(), &SshOpts::default());
        ssh.host = host.into();
        ssh
    }

    /// Set options that change *how the host is reached* (jump host, proxy command, SOCKS/HTTP
    /// proxy, an unresolvable `~/.ssh/config` `ProxyJump`). Not implemented yet, and connecting
    /// directly instead could reach a different machine, so the session refuses to connect.
    #[must_use]
    pub fn unsupported_route(&self) -> Vec<String> {
        let checks = [
            (self.jump_host.is_some(), "jump_host"),
            (self.proxy_command.is_some(), "proxy_command"),
            (self.socks_proxy.is_some(), "socks_proxy"),
            (self.http_proxy.is_some(), "http_proxy"),
        ];
        let from_ssh_config = self.ignored_ssh_config.iter().filter(|o| {
            let key = o.split_whitespace().next().unwrap_or_default();
            key.eq_ignore_ascii_case("ProxyJump") || key.eq_ignore_ascii_case("ProxyCommand")
        });
        checks
            .into_iter()
            .filter(|(set, _)| *set)
            .map(|(_, name)| name.to_owned())
            .chain(from_ssh_config.cloned())
            .collect()
    }

    /// Options that are set but whose behaviour is not implemented yet, then the ignored
    /// `~/.ssh/config` options; the session warns about each once at connect ("not supported
    /// yet: x11"). Never silently ignored.
    #[must_use]
    pub fn unsupported(&self) -> Vec<String> {
        let checks = [
            (
                self.input_backspace != Backspace::Backspace,
                "input_backspace",
            ),
            (self.jump_host.is_some(), "jump_host"),
            (self.proxy_command.is_some(), "proxy_command"),
            (self.socks_proxy.is_some(), "socks_proxy"),
            (self.http_proxy.is_some(), "http_proxy"),
            (!self.forwarded_ports.is_empty(), "forwarded_ports"),
            (self.agent_forward, "agent_forward"),
            (self.x11, "x11"),
            (!self.algorithms.is_default(), "algorithms"),
            (!self.reuse_session, "reuse_session = false"),
            (!self.scripts.is_empty(), "scripts"),
            (self.cwd.is_some(), "cwd"),
            (self.remember_cwd, "remember_cwd"),
        ];
        checks
            .into_iter()
            .filter(|(set, _)| *set)
            .map(|(_, name)| name.to_owned())
            .chain(self.ignored_ssh_config.iter().cloned())
            .collect()
    }
}

/// `profile.or(defaults).or(builtin)` for every SSH option (the SSH half of `resolve`).
#[must_use]
pub fn resolve_ssh(p: &SshOpts, d: &SshOpts) -> EffectiveSsh {
    // `profile.field` if set, else `defaults.field`.
    macro_rules! pick {
        ($field:ident) => {
            p.$field.clone().or_else(|| d.$field.clone())
        };
    }
    let ms = Duration::from_millis;
    let keepalive = pick!(keepalive_interval).unwrap_or(DEFAULT_KEEPALIVE_INTERVAL_MS);
    EffectiveSsh {
        host: pick!(host).unwrap_or_default(),
        port: pick!(port).unwrap_or(DEFAULT_SSH_PORT),
        user: pick!(user).unwrap_or_else(local_user),
        auth: pick!(auth).unwrap_or_default(),
        private_keys: pick!(private_keys).unwrap_or_default(),
        keepalive_interval: (keepalive > 0).then(|| ms(keepalive)),
        keepalive_count_max: pick!(keepalive_count_max).unwrap_or(DEFAULT_KEEPALIVE_COUNT_MAX),
        ready_timeout: pick!(ready_timeout).map(ms),
        skip_banner: pick!(skip_banner).unwrap_or(false),
        warn_on_close: pick!(warn_on_close).unwrap_or(true),
        term: pick!(term).unwrap_or_else(|| DEFAULT_TERM.to_owned()),
        input_backspace: pick!(input_backspace).unwrap_or_default(),
        jump_host: pick!(jump_host),
        proxy_command: pick!(proxy_command),
        socks_proxy: pick!(socks_proxy_host)
            .map(|host| (host, pick!(socks_proxy_port).unwrap_or(1080))),
        http_proxy: pick!(http_proxy_host)
            .map(|host| (host, pick!(http_proxy_port).unwrap_or(8080))),
        forwarded_ports: pick!(forwarded_ports).unwrap_or_default(),
        agent_forward: pick!(agent_forward).unwrap_or(false),
        x11: pick!(x11).unwrap_or(false),
        algorithms: pick!(algorithms).unwrap_or_default(),
        reuse_session: pick!(reuse_session).unwrap_or(true),
        scripts: pick!(scripts).unwrap_or_default(),
        cwd: pick!(cwd),
        remember_cwd: pick!(remember_cwd).unwrap_or(false),
        clear_service_messages_on_connect: pick!(clear_service_messages_on_connect).unwrap_or(true),
        verify_host_keys: pick!(verify_host_keys).unwrap_or(true),
        weight: pick!(weight).unwrap_or(0),
        identities_only: pick!(identities_only).unwrap_or(false),
        ignored_ssh_config: p.ignored_ssh_config.clone(),
    }
}

/// `$USER`, else `root`.
pub(crate) fn local_user() -> String {
    std::env::var("USER")
        .ok()
        .filter(|user| !user.is_empty())
        .unwrap_or_else(|| "root".to_owned())
}

#[cfg(test)]
#[path = "ssh_opts_tests.rs"]
mod tests;
