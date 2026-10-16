//! Authentication methods against real OpenSSH (key, encrypted key, RSA via the agent, agent) and, for
//! password / keyboard-interactive, against the in-process server (see `support/fake_server.rs`).

use std::fs;
use std::process::Command;

use fogu_core::{EffectiveSsh, SshAuth};
use fogu_session::{PromptKind, PromptReply, SessionError, SessionOutput, SshPaths};

use super::{error_of, no_prompts, secret};
use crate::support::client::Client;
use crate::support::fake_server::{self, FakeServer};
use crate::support::sshd::{TestAgent, TestSshd};
use crate::support::tmp::TempDir;

#[test]
fn unauthorized_key_fails_with_auth_failed() {
    let Some(sshd) = TestSshd::start() else {
        return;
    };
    sshd.trust();
    let stranger = sshd.new_key("stranger", "ed25519", "", false);
    let mut ssh = sshd.profile();
    ssh.private_keys = vec![stranger];
    let mut client = Client::connect(&ssh, sshd.paths(), no_prompts);
    let err = error_of(client.end());
    assert!(matches!(err, SessionError::AuthFailed), "{err:?}");
}

#[test]
fn encrypted_ed25519_key_asks_for_the_passphrase() {
    let Some(sshd) = TestSshd::start() else {
        return;
    };
    sshd.trust();
    let key = sshd.new_key("locked", "ed25519", "s3cret pass", true);
    let mut ssh = sshd.profile();
    ssh.private_keys = vec![key.clone()];
    // A wrong passphrase first: the session asks again.
    let mut tries = 0;
    let mut client = Client::connect(&ssh, sshd.paths(), |kind| {
        assert!(
            matches!(kind, PromptKind::KeyPassphrase { path } if *path == key),
            "{kind:?}"
        );
        tries += 1;
        PromptReply::Secret(secret(if tries == 1 { "wrong" } else { "s3cret pass" }))
    });
    client.wait_shell();
    assert_eq!(client.prompter.asked.len(), 2);
}

#[test]
fn rsa_key_files_are_signed_by_the_agent_with_one_passphrase_prompt() {
    let Some(sshd) = TestSshd::start() else {
        return;
    };
    let Some(agent) = TestAgent::start(&sshd.dir) else {
        return;
    };
    sshd.trust();
    for (name, pass) in [("rsa_plain", ""), ("rsa_locked", "pw")] {
        let key = sshd.new_key(name, "rsa", pass, true);
        let mut ssh = sshd.profile();
        ssh.private_keys = vec![key.clone()];
        ssh.agent_path = Some(agent.socket.clone());
        let mut client = Client::connect(&ssh, sshd.paths(), |kind| {
            assert!(
                matches!(kind, PromptKind::KeyPassphrase { path } if *path == key),
                "{kind:?}"
            );
            PromptReply::Secret(secret(pass))
        });
        client.wait_shell();
        let asked = usize::from(!pass.is_empty());
        assert_eq!(client.prompter.asked.len(), asked, "{name}");
        // The key now sits in the agent: no prompt the second time.
        let mut client = Client::connect(&ssh, sshd.paths(), no_prompts);
        client.wait_shell();
    }
}

const CHILD: &str = "FOGU_TEST_AGENT_CHILD";

/// Agent auth needs `SSH_AUTH_SOCK` in the *process* environment, and the crate forbids the
/// `unsafe` that `set_var` needs; so this test starts the daemon and the agent, then runs
/// [`agent_child`] in a copy of this test binary whose environment points at the agent.
#[test]
fn agent_identities_authenticate() {
    let Some(sshd) = TestSshd::start() else {
        return;
    };
    let Some(agent) = TestAgent::start(&sshd.dir) else {
        return;
    };
    agent.add(&sshd.client_key);
    sshd.trust();
    let out = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "auth::agent_child", "--ignored", "--nocapture"])
        .env("SSH_AUTH_SOCK", &agent.socket)
        .env(
            CHILD,
            format!(
                "{}\n{}\n{}",
                sshd.port,
                sshd.user,
                sshd.dir.path().display()
            ),
        )
        .output()
        .unwrap();
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "child failed:\n{text}");
    // The child leaves this file only after it got a shell, so a child that did not run fails.
    assert!(
        sshd.dir.join("agent-auth-ok").exists(),
        "child did not run:\n{text}"
    );
}

/// Not a test of its own: the child half of [`agent_identities_authenticate`].
#[test]
#[ignore = "run by agent_identities_authenticate"]
fn agent_child() {
    let Ok(spec) = std::env::var(CHILD) else {
        return;
    };
    let mut lines = spec.lines();
    let (port, user, dir) = (
        lines.next().unwrap(),
        lines.next().unwrap(),
        lines.next().unwrap(),
    );
    let mut ssh = EffectiveSsh::new("127.0.0.1");
    ssh.port = port.parse().unwrap();
    user.clone_into(&mut ssh.user);
    ssh.auth = SshAuth::Agent; // no key files configured: only the agent can log in
    ssh.keepalive_interval = None;
    let paths = SshPaths {
        known_hosts: format!("{dir}/known_hosts").into(),
        system_known_hosts: None,
    };
    assert!(
        fs::read_to_string(&paths.known_hosts)
            .unwrap()
            .contains("ssh-ed25519")
    );
    let mut client = Client::connect(&ssh, paths.clone(), no_prompts);
    client.wait_shell();
    // `identities_only` (ssh_config `IdentitiesOnly`): the agent key is offered only when its
    // `.pub` sits next to a configured key file.
    ssh.identities_only = true;
    let mut client = Client::connect(&ssh, paths.clone(), no_prompts);
    assert!(matches!(error_of(client.end()), SessionError::AuthFailed));
    ssh.private_keys = vec![format!("{dir}/client_ed25519").into()];
    let mut client = Client::connect(&ssh, paths, no_prompts);
    client.wait_shell();
    fs::write(format!("{dir}/agent-auth-ok"), "").unwrap();
}

pub(super) fn fake_profile(server: &FakeServer, auth: SshAuth) -> EffectiveSsh {
    let mut ssh = EffectiveSsh::new("127.0.0.1");
    ssh.port = server.port;
    "tester".clone_into(&mut ssh.user);
    ssh.auth = auth;
    ssh.keepalive_interval = None;
    ssh
}

pub(super) fn fake_paths(dir: &TempDir) -> SshPaths {
    SshPaths {
        known_hosts: dir.join("known_hosts"),
        system_known_hosts: None,
    }
}

#[test]
fn password_auth_retries_after_a_wrong_password() {
    let server = FakeServer::start();
    let dir = TempDir::new("pw");
    let mut tries = 0;
    let mut client = Client::connect(
        &fake_profile(&server, SshAuth::Password),
        fake_paths(&dir),
        |kind| match kind {
            PromptKind::HostKeyUnknown { .. } => PromptReply::Accept,
            PromptKind::Password { user, .. } => {
                assert_eq!(user, "tester");
                tries += 1;
                let text = if tries == 1 {
                    "nope"
                } else {
                    fake_server::PASSWORD
                };
                PromptReply::Secret(secret(text))
            }
            other => panic!("unexpected prompt: {other:?}"),
        },
    );
    client.expect_text(fake_server::READY);
    client.send("exit\r");
    assert!(matches!(client.end(), SessionOutput::Exited(_)));
}

#[test]
fn three_wrong_passwords_fail_authentication() {
    let server = FakeServer::start();
    let dir = TempDir::new("pw3");
    let mut client = Client::connect(
        &fake_profile(&server, SshAuth::Password),
        fake_paths(&dir),
        |kind| match kind {
            PromptKind::HostKeyUnknown { .. } => PromptReply::Accept,
            _ => PromptReply::Secret(secret("wrong")),
        },
    );
    let err = error_of(client.end());
    assert!(matches!(err, SessionError::AuthFailed), "{err:?}");
    let passwords = client
        .prompter
        .asked
        .iter()
        .filter(|k| matches!(k, PromptKind::Password { .. }))
        .count();
    assert_eq!(passwords, 3);
}

#[test]
fn keyboard_interactive_shows_sanitized_prompts_and_sends_the_answers() {
    let server = FakeServer::start();
    let dir = TempDir::new("kbd");
    let ssh = fake_profile(&server, SshAuth::KeyboardInteractive);
    let mut client = Client::connect(&ssh, fake_paths(&dir), |kind| match kind {
        PromptKind::HostKeyUnknown { .. } => PromptReply::Accept,
        PromptKind::KeyboardInteractive {
            name,
            instructions,
            prompts,
        } => {
            assert_eq!(name, "Test login");
            assert_eq!(instructions, "Answer both[31mred]0;pwned");
            let shape: Vec<_> = prompts.iter().map(|p| (p.text.as_str(), p.echo)).collect();
            assert_eq!(shape, [("Verification code: ", true), ("Secret: ", false)]);
            PromptReply::Secrets(vec![
                secret(fake_server::KBD_CODE),
                secret(fake_server::KBD_SECRET),
            ])
        }
        other => panic!("unexpected prompt: {other:?}"),
    });
    client.expect_text(fake_server::READY);
}

#[test]
fn an_unanswered_prompt_ends_the_connection_after_the_profiles_prompt_timeout() {
    use std::time::{Duration, Instant};
    let server = FakeServer::start();
    let dir = TempDir::new("prompt-timeout");
    let mut ssh = fake_profile(&server, SshAuth::Password);
    ssh.prompt_timeout = Some(Duration::from_millis(300));
    let client = Client::connect(&ssh, fake_paths(&dir), no_prompts);
    let Client {
        rt, mut session, ..
    } = client;
    let started = Instant::now();
    // Nobody answers the host-key prompt.
    let end = rt.handle().block_on(async {
        let wait = async {
            while let Some(event) = session.events.recv().await {
                if matches!(event, SessionOutput::Error(_) | SessionOutput::Exited(_)) {
                    return Some(event);
                }
            }
            None
        };
        tokio::time::timeout(Duration::from_secs(10), wait)
            .await
            .ok()
            .flatten()
    });
    let err = error_of(end.expect("the session ended"));
    assert!(matches!(err, SessionError::PromptTimeout), "{err:?}");
    assert!(started.elapsed() < Duration::from_secs(5));
}
