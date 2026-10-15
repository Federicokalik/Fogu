//! Host key verification against real OpenSSH.

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::process::Command;

use fogu_session::{HostKeyFailure, PromptKind, PromptReply, SessionError};

use super::{error_of, no_prompts};
use crate::support::client::Client;
use crate::support::sshd::{TestSshd, pub_without_comment};

fn unknown_prompt(sshd: &TestSshd, kind: &PromptKind) {
    let PromptKind::HostKeyUnknown {
        host,
        port,
        key_type,
        fingerprint,
    } = kind
    else {
        panic!("expected HostKeyUnknown, got {kind:?}");
    };
    assert_eq!((host.as_str(), *port), ("127.0.0.1", sshd.port));
    assert_eq!(key_type, "ssh-ed25519");
    // Computed by ssh-keygen, independently of our code.
    assert_eq!(*fingerprint, sshd.host_key_fingerprint());
}

#[test]
fn unknown_key_accept_and_save_writes_a_0600_line_and_the_next_connect_is_known() {
    let Some(sshd) = TestSshd::start() else {
        return;
    };
    let mut client = Client::connect(&sshd.profile(), sshd.paths(), |kind| {
        unknown_prompt(&sshd, kind);
        PromptReply::AcceptAndSave
    });
    client.wait_shell();
    assert_eq!(client.prompter.asked.len(), 1);
    client.close();
    drop(client);

    let file = sshd.known_hosts();
    assert_eq!(fs::read_to_string(&file).unwrap(), sshd.known_hosts_line());
    let mode = fs::metadata(&file).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "known_hosts mode");

    let mut again = Client::connect(&sshd.profile(), sshd.paths(), no_prompts);
    again.wait_shell();
    assert_eq!(again.prompter.asked.len(), 0);
}

#[test]
fn accept_once_writes_nothing_and_reject_refuses() {
    let Some(sshd) = TestSshd::start() else {
        return;
    };
    let mut once = Client::connect(&sshd.profile(), sshd.paths(), |_| PromptReply::Accept);
    once.wait_shell();
    drop(once);
    assert!(!sshd.known_hosts().exists(), "accept-once must not write");

    let mut rejected = Client::connect(&sshd.profile(), sshd.paths(), |_| PromptReply::Reject);
    let err = error_of(rejected.end());
    assert!(
        matches!(err, SessionError::HostKey(HostKeyFailure::Rejected)),
        "{err:?}"
    );
    assert!(!sshd.known_hosts().exists());
}

/// Records a different key for the daemon's address, plus an unrelated host that must survive.
fn record_other_key(sshd: &TestSshd) -> String {
    let other = sshd.new_key("other_host", "ed25519", "", false);
    let other_pub = pub_without_comment(&sshd.dir.join("other_host.pub"));
    drop(other);
    let text = format!(
        "example.invalid {other_pub}\n[127.0.0.1]:{} {other_pub}\n",
        sshd.port
    );
    fs::write(sshd.known_hosts(), &text).unwrap();
    text
}

#[test]
fn changed_key_is_refused_unless_explicitly_replaced() {
    let Some(sshd) = TestSshd::start() else {
        return;
    };
    let before = record_other_key(&sshd);

    // Reject, and a plain Accept, must both refuse and leave the file alone.
    for reply in [PromptReply::Reject, PromptReply::Accept] {
        let mut reply = Some(reply);
        let mut client = Client::connect(&sshd.profile(), sshd.paths(), |kind| {
            let PromptKind::HostKeyChanged {
                fingerprint, line, ..
            } = kind
            else {
                panic!("expected HostKeyChanged, got {kind:?}");
            };
            assert_eq!(*fingerprint, sshd.host_key_fingerprint());
            assert_eq!(*line, 2);
            reply.take().expect("one prompt")
        });
        let err = error_of(client.end());
        assert!(
            matches!(err, SessionError::HostKey(HostKeyFailure::Changed)),
            "{err:?}"
        );
        assert_eq!(fs::read_to_string(sshd.known_hosts()).unwrap(), before);
    }

    // The explicit replace flow connects and rewrites only the stale line.
    let mut client = Client::connect(&sshd.profile(), sshd.paths(), |_| {
        PromptReply::AcceptAndSave
    });
    client.wait_shell();
    drop(client);
    let after = fs::read_to_string(sshd.known_hosts()).unwrap();
    assert!(after.contains(&sshd.known_hosts_line()), "{after}");
    assert!(
        after.starts_with(before.lines().next().unwrap()),
        "other host kept: {after}"
    );
    assert_eq!(after.lines().count(), 2, "{after}");

    let mut known = Client::connect(&sshd.profile(), sshd.paths(), no_prompts);
    known.wait_shell();
}

#[test]
fn hashed_known_hosts_entry_matches() {
    let Some(sshd) = TestSshd::start() else {
        return;
    };
    sshd.trust();
    // `ssh-keygen -H` hashes the host names, like `ssh-keyscan -H` does.
    let status = Command::new(sshd.keygen())
        .arg("-H")
        .arg("-f")
        .arg(sshd.known_hosts())
        .output()
        .unwrap();
    assert!(status.status.success(), "{status:?}");
    let text = fs::read_to_string(sshd.known_hosts()).unwrap();
    assert!(text.starts_with("|1|"), "{text}");
    let mut client = Client::connect(&sshd.profile(), sshd.paths(), no_prompts);
    client.wait_shell();
}
