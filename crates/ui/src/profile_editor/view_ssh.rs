//! The SSH sections of the editor: Connection, Authentication, Network, Algorithms, Login
//! scripts, Terminal. Empty fields show the inherited value as placeholder; checkboxes show the
//! effective value (Save drops whatever equals the inherited value).

use fogu_core::config::ssh_opts::{
    DEFAULT_KEEPALIVE_COUNT_MAX, DEFAULT_KEEPALIVE_INTERVAL_MS, DEFAULT_PROMPT_TIMEOUT_MS,
    DEFAULT_SSH_PORT, DEFAULT_TERM, resolve_ssh,
};
use fogu_core::{Config, EffectiveSsh, SshOpts};

use super::ProfileEditor;
use super::fields::{AUTH, BACKSPACE, FieldId};
use super::view::{Row, index_of, labels, shown, with_notes};
use super::view_lists::{algorithms, forwards, line, scripts};
use crate::forms::{self, ERROR, WARN};
use crate::sftp::panel::Look;

/// Shown at the top of Network while jump hosts and proxies are not implemented.
const ROUTE_BANNER: &str = "Jump hosts and proxies are not supported yet. A profile that sets \
     one refuses to connect (it never connects directly instead, which could reach another \
     machine).";
const NO_PASSWORDS: &str = "Stored passwords are not available yet: you are asked when connecting.";

pub(super) fn sections<'a>(
    ed: &'a ProfileEditor,
    ssh: &'a SshOpts,
    config: &Config,
    look: Look,
) -> Vec<Row<'a>> {
    let d = &config.profile_defaults.ssh;
    let eff = resolve_ssh(ssh, d);
    vec![
        connection(ed, ssh, d, &eff, look),
        authentication(ed, ssh, d, &eff, look),
        network(ed, ssh, &eff, look),
        algorithms(ed, ssh, look),
        scripts(ed, ssh, look),
        terminal(ed, ssh, &eff, look),
    ]
}

/// A number field with its notes; `key` is its config name.
fn number_row<'a>(
    ed: &ProfileEditor,
    look: Look,
    (id, key): (FieldId, &str),
    label: &str,
    value: Option<impl std::fmt::Display>,
    placeholder: &dyn std::fmt::Display,
) -> Row<'a> {
    let field = forms::number(
        look,
        id,
        label,
        &shown(ed, id, value),
        &placeholder.to_string(),
    );
    with_notes(ed, id, key, field)
}

fn text_row<'a>(
    ed: &ProfileEditor,
    look: Look,
    (id, key): (FieldId, &str),
    label: &str,
    value: Option<&str>,
    placeholder: &str,
) -> Row<'a> {
    let field = forms::text_field(look, id, label, value.unwrap_or_default(), placeholder);
    with_notes(ed, id, key, field)
}

fn toggle_row<'a>(
    ed: &ProfileEditor,
    look: Look,
    (id, key): (FieldId, &str),
    label: &str,
    on: bool,
) -> Row<'a> {
    with_notes(ed, id, key, forms::toggle(look, id, label, on))
}

fn connection<'a>(
    ed: &'a ProfileEditor,
    ssh: &'a SshOpts,
    d: &SshOpts,
    eff: &EffectiveSsh,
    look: Look,
) -> Row<'a> {
    let inherited = resolve_ssh(&SshOpts::default(), d);
    let mut verify = toggle_row(
        ed,
        look,
        (FieldId::VerifyHostKeys, "verify_host_keys"),
        "Verify host keys (known_hosts)",
        eff.verify_host_keys,
    );
    let off = "Off: any server key is accepted, so a machine in between can read everything.";
    verify = forms::note_maybe(verify, (!eff.verify_host_keys).then_some((off, ERROR)));
    let mut rows = vec![
        text_row(
            ed,
            look,
            (FieldId::Host, "host"),
            "Host",
            ssh.host.as_deref(),
            d.host.as_deref().unwrap_or("hostname or address"),
        ),
        number_row(
            ed,
            look,
            (FieldId::Port, "port"),
            "Port",
            ssh.port,
            &d.port.unwrap_or(DEFAULT_SSH_PORT),
        ),
        text_row(
            ed,
            look,
            (FieldId::User, "user"),
            "User",
            ssh.user.as_deref(),
            &inherited.user,
        ),
        text_row(
            ed,
            look,
            (FieldId::Term, "term"),
            "TERM",
            ssh.term.as_deref(),
            d.term.as_deref().unwrap_or(DEFAULT_TERM),
        ),
        toggle_row(
            ed,
            look,
            (FieldId::SkipBanner, "skip_banner"),
            "Hide the server's login banner",
            eff.skip_banner,
        ),
        toggle_row(
            ed,
            look,
            (
                FieldId::ClearServiceMessages,
                "clear_service_messages_on_connect",
            ),
            "Clear \"Connecting…\" lines once connected",
            eff.clear_service_messages_on_connect,
        ),
        verify,
        toggle_row(
            ed,
            look,
            (FieldId::WarnOnClose, "warn_on_close"),
            "Ask before closing a connected tab",
            eff.warn_on_close,
        ),
        toggle_row(
            ed,
            look,
            (FieldId::ReuseSession, "reuse_session"),
            "Reuse the connection for new tabs",
            eff.reuse_session,
        ),
        number_row(
            ed,
            look,
            (FieldId::Weight, "weight"),
            "Sort weight",
            ssh.weight,
            &d.weight.unwrap_or(0),
        ),
    ];
    rows.splice(4..4, timing_rows(ed, ssh, d, look));
    forms::section(look, "Connection", rows)
}

/// Keepalive and timeouts (milliseconds, like the config file).
fn timing_rows<'a>(ed: &ProfileEditor, ssh: &SshOpts, d: &SshOpts, look: Look) -> Vec<Row<'a>> {
    vec![
        number_row(
            ed,
            look,
            (FieldId::KeepaliveInterval, "keepalive_interval"),
            "Keepalive interval (ms, 0 = off)",
            ssh.keepalive_interval,
            &d.keepalive_interval
                .unwrap_or(DEFAULT_KEEPALIVE_INTERVAL_MS),
        ),
        number_row(
            ed,
            look,
            (FieldId::KeepaliveCountMax, "keepalive_count_max"),
            "Unanswered keepalives before disconnecting",
            ssh.keepalive_count_max,
            &d.keepalive_count_max.unwrap_or(DEFAULT_KEEPALIVE_COUNT_MAX),
        ),
        number_row(
            ed,
            look,
            (FieldId::ReadyTimeout, "ready_timeout"),
            "Connect timeout (ms)",
            ssh.ready_timeout,
            &d.ready_timeout.unwrap_or(20_000),
        ),
        number_row(
            ed,
            look,
            (FieldId::PromptTimeout, "prompt_timeout"),
            "Prompt timeout (ms, 0 = no limit)",
            ssh.prompt_timeout,
            &d.prompt_timeout.unwrap_or(DEFAULT_PROMPT_TIMEOUT_MS),
        ),
    ]
}

fn authentication<'a>(
    ed: &'a ProfileEditor,
    ssh: &'a SshOpts,
    d: &SshOpts,
    eff: &EffectiveSsh,
    look: Look,
) -> Row<'a> {
    let keys: Vec<String> = ssh
        .private_keys
        .iter()
        .flatten()
        .map(|k| k.to_string_lossy().into_owned())
        .collect();
    let agent = ssh
        .agent_path
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned());
    let agent_hint = d.agent_path.as_ref().map_or_else(
        || "$SSH_AUTH_SOCK".to_owned(),
        |p| p.to_string_lossy().into_owned(),
    );
    let rows = vec![
        forms::choice(
            look,
            FieldId::Auth,
            "Method",
            labels(&AUTH),
            index_of(&AUTH, &eff.auth),
        ),
        forms::note(
            forms::list(
                look,
                FieldId::PrivateKeys,
                "Private keys (tried in order)",
                &keys,
                "~/.ssh/id_ed25519",
                Some(ed.picker),
            ),
            "OpenSSH, PEM or PuTTY keys; RSA keys are refused.",
            look.dim,
        ),
        forms::secret_hint(
            look,
            FieldId::Password,
            "Password",
            "not stored",
            Some(NO_PASSWORDS),
        ),
        forms::path(
            look,
            FieldId::AgentPath,
            "SSH agent socket",
            agent.as_deref().unwrap_or_default(),
            &agent_hint,
            ed.picker,
        ),
        toggle_row(
            ed,
            look,
            (FieldId::IdentitiesOnly, "identities_only"),
            "Only agent keys matching the private keys above",
            eff.identities_only,
        ),
    ];
    forms::section(look, "Authentication", rows)
}

fn network<'a>(ed: &'a ProfileEditor, ssh: &'a SshOpts, eff: &EffectiveSsh, look: Look) -> Row<'a> {
    let jump = ed
        .jumps
        .iter()
        .position(|(j, _)| j == &ssh.jump_host)
        .unwrap_or(0);
    let jump_row = with_notes(
        ed,
        FieldId::JumpHost,
        "jump_host",
        forms::choice(
            look,
            FieldId::JumpHost,
            "Jump host",
            ed.jumps.iter().map(|(_, l)| l.clone()).collect(),
            jump,
        ),
    );
    let proxy = forms::note(
        text_row(
            ed,
            look,
            (FieldId::ProxyCommand, "proxy_command"),
            "Proxy command",
            ssh.proxy_command.as_deref(),
            "e.g. nc %h %p",
        ),
        "Runs on this computer with your rights.",
        look.dim,
    );
    let agent = forms::note(
        toggle_row(
            ed,
            look,
            (FieldId::AgentForward, "agent_forward"),
            "Forward the SSH agent",
            eff.agent_forward,
        ),
        "The server can use your agent's keys while you are connected.",
        look.dim,
    );
    let x11 = forms::note(
        toggle_row(ed, look, (FieldId::X11, "x11"), "X11 forwarding", eff.x11),
        "Remote programs can read your keyboard and screen.",
        look.dim,
    );
    let rows = vec![
        line(WARN, ROUTE_BANNER),
        jump_row,
        proxy,
        text_row(
            ed,
            look,
            (FieldId::SocksHost, "socks_proxy_host"),
            "SOCKS5 proxy host",
            ssh.socks_proxy_host.as_deref(),
            "proxy host",
        ),
        number_row(
            ed,
            look,
            (FieldId::SocksPort, "socks_proxy_port"),
            "SOCKS5 proxy port",
            ssh.socks_proxy_port,
            &1080,
        ),
        text_row(
            ed,
            look,
            (FieldId::HttpHost, "http_proxy_host"),
            "HTTP proxy host",
            ssh.http_proxy_host.as_deref(),
            "proxy host",
        ),
        number_row(
            ed,
            look,
            (FieldId::HttpPort, "http_proxy_port"),
            "HTTP proxy port",
            ssh.http_proxy_port,
            &8080,
        ),
        agent,
        x11,
        forwards(ed, ssh, look),
    ];
    forms::section(look, "Network", rows)
}

fn terminal<'a>(
    ed: &'a ProfileEditor,
    ssh: &'a SshOpts,
    eff: &EffectiveSsh,
    look: Look,
) -> Row<'a> {
    let rows = vec![
        text_row(
            ed,
            look,
            (FieldId::RemoteCwd, "cwd"),
            "Remote start directory",
            ssh.cwd.as_deref(),
            "home directory",
        ),
        toggle_row(
            ed,
            look,
            (FieldId::RememberCwd, "remember_cwd"),
            "Reopen in the last directory",
            eff.remember_cwd,
        ),
        with_notes(
            ed,
            FieldId::Backspace,
            "input_backspace",
            forms::choice(
                look,
                FieldId::Backspace,
                "Backspace sends",
                labels(&BACKSPACE),
                index_of(&BACKSPACE, &eff.input_backspace),
            ),
        ),
    ];
    forms::section(look, "Terminal", rows)
}
