#![expect(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "tests"
)]

use super::*;
use crate::profile::{EffectiveProfile, Profile, ProfileDefaults, ProfileKind, resolve};

/// A profile setting every SSH field (plus the common ones), as a user would write it.
const EVERY_FIELD: &str = r#"
id = "ssh:full"
name = "Every field"
type = "ssh"
behavior_on_session_end = "reconnect"
host = "example.com"
port = 2222
user = "alice"
auth = "keyboard_interactive"
private_keys = ["~/.ssh/id_ed25519", "/keys/work.ppk"]
keepalive_interval = 15000
keepalive_count_max = 3
ready_timeout = 9000
skip_banner = true
warn_on_close = false
term = "xterm"
input_backspace = "ctrl-?"
jump_host = "ssh:bastion"
proxy_command = "nc %h %p"
socks_proxy_host = "127.0.0.1"
socks_proxy_port = 9050
http_proxy_host = "proxy.lan"
http_proxy_port = 3128
agent_forward = true
x11 = true
reuse_session = false
cwd = "/srv"
remember_cwd = true
clear_service_messages_on_connect = false
verify_host_keys = false
weight = -5
osc52_write = "deny"

[[forwarded_ports]]
type = "dynamic"
host = "127.0.0.1"
port = 1080
target_address = ""
target_port = 0
description = "socks"

[algorithms]
kex = ["curve25519-sha256"]
cipher = ["chacha20-poly1305@openssh.com"]
hmac = []
server_host_key = ["ssh-ed25519"]
compression = ["none"]

[[scripts]]
expect = "Password:"
send = "x\n"
is_regex = false
optional = true
"#;

#[test]
fn every_field_round_trips_through_toml() {
    let profile: Profile = toml::from_str(EVERY_FIELD).unwrap();
    let ProfileKind::Ssh(opts) = &profile.kind else {
        panic!("expected ssh");
    };
    assert_eq!(opts.auth, Some(SshAuth::KeyboardInteractive));
    assert_eq!(opts.input_backspace, Some(Backspace::CtrlQuestion));
    assert_eq!(
        opts.forwarded_ports.as_ref().unwrap()[0].kind,
        ForwardKind::Dynamic
    );
    assert_eq!(opts.scripts.as_ref().unwrap()[0].expect, "Password:");
    assert_eq!(opts.weight, Some(-5));
    let text = toml::to_string(&profile).unwrap();
    let back: Profile = toml::from_str(&text).unwrap();
    assert_eq!(back, profile);
}

#[test]
fn every_field_resolves_and_is_reported_unsupported() {
    let profile: Profile = toml::from_str(EVERY_FIELD).unwrap();
    let EffectiveProfile::Ssh(ssh) = resolve(&profile, &ProfileDefaults::default()) else {
        panic!("expected ssh");
    };
    assert_eq!(
        (ssh.host.as_str(), ssh.port, ssh.user.as_str()),
        ("example.com", 2222, "alice")
    );
    assert_eq!(ssh.keepalive_interval, Some(Duration::from_secs(15)));
    assert_eq!(ssh.ready_timeout, Some(Duration::from_secs(9)));
    assert_eq!(ssh.socks_proxy, Some(("127.0.0.1".to_owned(), 9050)));
    assert_eq!(ssh.http_proxy, Some(("proxy.lan".to_owned(), 3128)));
    assert!(!ssh.verify_host_keys && !ssh.clear_service_messages_on_connect);
    assert_eq!(
        ssh.unsupported(),
        [
            "input_backspace",
            "jump_host",
            "proxy_command",
            "socks_proxy",
            "http_proxy",
            "forwarded_ports",
            "agent_forward",
            "x11",
            "algorithms",
            "reuse_session = false",
            "scripts",
            "cwd",
            "remember_cwd",
            "warn_on_close",
        ]
    );
}

#[test]
fn builtin_defaults() {
    let ssh = EffectiveSsh::new("h");
    assert_eq!(ssh.port, 22);
    assert_ne!(ssh.user, "", "user defaults to $USER or root");
    assert_eq!(ssh.auth, SshAuth::Auto);
    assert_eq!(ssh.keepalive_interval, Some(Duration::from_millis(5000)));
    assert_eq!(ssh.keepalive_count_max, 10);
    assert_eq!(ssh.ready_timeout, None);
    assert_eq!(ssh.term, "xterm-256color");
    assert!(ssh.reuse_session && ssh.verify_host_keys && ssh.clear_service_messages_on_connect);
    assert!(!ssh.skip_banner && !ssh.agent_forward && !ssh.x11 && !ssh.remember_cwd);
    assert_eq!(ssh.unsupported(), Vec::<&str>::new());
}

#[test]
fn profile_overrides_defaults_and_zero_keepalive_disables() {
    let defaults = SshOpts {
        user: Some("ops".to_owned()),
        keepalive_interval: Some(0),
        term: Some("vt100".to_owned()),
        ..SshOpts::default()
    };
    let profile = SshOpts {
        host: Some("h".to_owned()),
        term: Some("xterm".to_owned()),
        ..SshOpts::default()
    };
    let ssh = resolve_ssh(&profile, &defaults);
    assert_eq!(ssh.user, "ops");
    assert_eq!(ssh.term, "xterm");
    assert_eq!(ssh.keepalive_interval, None);
}
