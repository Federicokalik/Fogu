//! Routes to the host against real OpenSSH: a jump through a second sshd (both host keys
//! checked, each with its own prompt), a proxy command (`socat`/`nc`), and the in-process
//! SOCKS5 / HTTP CONNECT proxies of `support/proxy.rs`.

use fogu_core::{EffectiveSsh, Jump};
use fogu_session::{HostKeyFailure, PromptKind, PromptReply, SessionError};

use super::{error_of, no_prompts};
use crate::support::client::Client;
use crate::support::proxy::{Kind, MockProxy};
use crate::support::sshd::{TestSshd, find_tool, skip};

/// `target` through `jump`.
fn via(mut target: EffectiveSsh, jump: EffectiveSsh) -> EffectiveSsh {
    target.jump_host = Some(fogu_core::ProfileId("jump".into()));
    target.jump = Jump::Via(Box::new(jump));
    target
}

/// Saves every unknown host key; anything else fails the test.
fn accept_and_save(kind: &PromptKind) -> PromptReply {
    match kind {
        PromptKind::HostKeyUnknown { .. } => PromptReply::AcceptAndSave,
        other => panic!("unexpected prompt: {other:?}"),
    }
}

fn shell_works<F: FnMut(&PromptKind) -> PromptReply>(client: &mut Client<F>) {
    client.wait_shell();
    client.send("echo route-$((6*7))\r");
    client.expect_text("route-42");
}

#[test]
fn a_jump_host_checks_both_host_keys_and_reaches_the_shell() {
    let (Some(jump), Some(target)) = (TestSshd::start(), TestSshd::start()) else {
        return;
    };
    let profile = via(target.profile(), jump.profile());
    let mut client = Client::connect(&profile, target.paths(), accept_and_save);
    shell_works(&mut client);
    let ports: Vec<_> = client
        .prompter
        .asked
        .iter()
        .map(|kind| match kind {
            PromptKind::HostKeyUnknown { port, .. } => *port,
            other => panic!("unexpected prompt {other:?}"),
        })
        .collect();
    assert_eq!(
        ports,
        [jump.port, target.port],
        "jump host first, then the target"
    );
    let hop = format!("{}@127.0.0.1:{}", jump.user, jump.port);
    assert_eq!(client.prompter.via, [Some(hop), None]);
    client.close();
    drop(client);

    // Both keys were saved: the second time nobody is asked.
    let saved = std::fs::read_to_string(target.known_hosts()).unwrap();
    assert!(
        saved.contains(&format!("[127.0.0.1]:{}", jump.port)),
        "{saved}"
    );
    assert!(
        saved.contains(&format!("[127.0.0.1]:{}", target.port)),
        "{saved}"
    );
    let mut again = Client::connect(&profile, target.paths(), no_prompts);
    shell_works(&mut again);
}

#[test]
fn a_changed_key_on_the_jump_host_refuses_like_any_host() {
    let (Some(jump), Some(target)) = (TestSshd::start(), TestSshd::start()) else {
        return;
    };
    // The jump host's record holds the target's key: "changed".
    let stale = format!("[127.0.0.1]:{} {}\n", jump.port, target.host_pub);
    std::fs::write(target.known_hosts(), stale).unwrap();
    let profile = via(target.profile(), jump.profile());
    let mut client = Client::connect(&profile, target.paths(), |kind| match kind {
        // A plain Accept is never enough for a changed key, on a hop as anywhere.
        PromptKind::HostKeyChanged { .. } => PromptReply::Accept,
        other => panic!("unexpected prompt: {other:?}"),
    });
    let err = error_of(client.end());
    let SessionError::Jump { hop, error } = &err else {
        panic!("expected a jump host error, got {err:?}");
    };
    assert_eq!(hop, &format!("{}@127.0.0.1:{}", jump.user, jump.port));
    assert!(
        matches!(**error, SessionError::HostKey(HostKeyFailure::Changed)),
        "{error:?}"
    );
    assert_eq!(
        client.prompter.asked.len(),
        1,
        "the target was never reached"
    );
}

#[test]
fn a_failed_login_on_the_jump_host_names_the_hop() {
    let (Some(jump), Some(target)) = (TestSshd::start(), TestSshd::start()) else {
        return;
    };
    jump.trust();
    let mut hop = jump.profile();
    // The target's key is not authorized on the jump host.
    hop.private_keys = vec![target.client_key.clone()];
    let mut client = Client::connect(&via(target.profile(), hop), jump.paths(), no_prompts);
    let err = error_of(client.end());
    assert!(
        matches!(&err, SessionError::Jump { error, .. } if matches!(**error, SessionError::AuthFailed)),
        "{err:?}"
    );
    assert!(err.to_string().starts_with("jump host "), "{err}");
}

#[test]
fn a_proxy_command_carries_the_connection() {
    let Some(sshd) = TestSshd::start() else {
        return;
    };
    let command = if find_tool("socat").is_some() {
        "socat - TCP:%h:%p"
    } else if find_tool("nc").is_some() {
        "nc %h %p"
    } else {
        skip("neither socat nor nc is installed");
        return;
    };
    sshd.trust();
    let mut profile = sshd.profile();
    profile.proxy_command = Some(command.into());
    let mut client = Client::connect(&profile, sshd.paths(), no_prompts);
    shell_works(&mut client);
}

#[test]
fn socks5_sends_the_name_by_default_and_the_address_when_asked() {
    let Some(sshd) = TestSshd::start() else {
        return;
    };
    let proxy = MockProxy::start(Kind::Socks5);
    let mut profile = sshd.profile();
    profile.host = "localhost".into();
    profile.socks_proxy = Some(("127.0.0.1".into(), proxy.port));
    // `localhost` has no record: accept its key once.
    let accept_once = |kind: &PromptKind| match kind {
        PromptKind::HostKeyUnknown { host, .. } if host == "localhost" => PromptReply::Accept,
        other => panic!("unexpected prompt: {other:?}"),
    };
    let mut client = Client::connect(&profile, sshd.paths(), accept_once);
    shell_works(&mut client);
    client.close();
    drop(client);
    assert_eq!(
        proxy.requests(),
        [format!("domain localhost:{}", sshd.port)]
    );

    profile.socks_remote_dns = false;
    let mut client = Client::connect(&profile, sshd.paths(), accept_once);
    shell_works(&mut client);
    let requests = proxy.requests();
    let last = requests.last().unwrap();
    assert!(
        last == &format!("ipv4 127.0.0.1:{}", sshd.port)
            || last == &format!("ipv6 ::1:{}", sshd.port),
        "{requests:?}"
    );
}

#[test]
fn an_http_connect_proxy_carries_the_connection() {
    let Some(sshd) = TestSshd::start() else {
        return;
    };
    sshd.trust();
    let proxy = MockProxy::start(Kind::Http);
    let mut profile = sshd.profile();
    profile.http_proxy = Some(("127.0.0.1".into(), proxy.port));
    let mut client = Client::connect(&profile, sshd.paths(), no_prompts);
    shell_works(&mut client);
    assert_eq!(
        proxy.requests(),
        [format!("CONNECT 127.0.0.1:{} HTTP/1.1", sshd.port)]
    );
}
