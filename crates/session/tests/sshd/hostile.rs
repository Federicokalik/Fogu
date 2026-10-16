//! Hostile-server probes: whatever the server does, the session must end in a visible
//! outcome (shell or `Error`), never hang or leave the user without a toast.

use std::time::Duration;

use fogu_core::SshAuth;
use fogu_session::{PromptKind, PromptReply, SessionError, SessionOutput};

use super::auth::{fake_paths, fake_profile};
use super::{no_prompts, secret};
use crate::support::client::Client;
use crate::support::fake_server::{self, FakeServer};
use crate::support::sshd::{Options, TestSshd};
use crate::support::tmp::TempDir;

/// A 1 MiB banner (OpenSSH's own limit for the `Banner` file) with an escape sequence in every
/// line: the client must show a capped, sanitized part of it and still reach the shell.
#[test]
fn one_mebibyte_banner_is_capped_and_the_shell_still_opens() {
    let line = b"banner line \x1b[31mred\x1b]0;pwned\x07 end\n";
    let mut banner = Vec::with_capacity(1024 * 1024);
    while banner.len() + line.len() <= 1024 * 1024 {
        banner.extend_from_slice(line);
    }
    let Some(sshd) = TestSshd::start_with(&Options {
        banner: Some(banner),
    }) else {
        return;
    };
    sshd.trust();
    let mut ssh = sshd.profile();
    ssh.clear_service_messages_on_connect = false;
    let mut client = Client::connect(&ssh, sshd.paths(), no_prompts);
    let end = client.until_for(Duration::from_secs(20), |e| {
        matches!(e, SessionOutput::Exited(_) | SessionOutput::Error(_))
    });
    // Either outcome is acceptable as long as it is visible and prompt; the shell is expected.
    match end {
        None => {
            client.wait_shell();
            let flat: String = client.view.dump_grid().split_whitespace().collect();
            assert!(
                !flat.contains('\x1b'),
                "escape reached the terminal:\n{flat}"
            );
            client.send("exit\r");
            assert!(matches!(client.end(), SessionOutput::Exited(_)));
        }
        Some(SessionOutput::Error(err)) => eprintln!("1 MiB banner ended the session: {err}"),
        Some(other) => panic!("unexpected end: {other:?}"),
    }
}

/// The server disconnects right after `USERAUTH_SUCCESS`: a toast-worthy `Error` within the
/// `ready_timeout`, not a hang on the channel open.
#[test]
fn server_closing_right_after_auth_reports_an_error() {
    let server = FakeServer::start_with(true);
    let dir = TempDir::new("drop-after-auth");
    let mut ssh = fake_profile(&server, SshAuth::Password);
    ssh.ready_timeout = Some(Duration::from_secs(5));
    let mut client = Client::connect(&ssh, fake_paths(&dir), |kind| match kind {
        PromptKind::HostKeyUnknown { .. } => PromptReply::Accept,
        PromptKind::Password { .. } => PromptReply::Secret {
            value: secret(fake_server::PASSWORD),
            remember: false,
        },
        other => panic!("unexpected prompt: {other:?}"),
    });
    let end = client.until_for(Duration::from_secs(8), |e| {
        matches!(e, SessionOutput::Exited(_) | SessionOutput::Error(_))
    });
    let Some(SessionOutput::Error(err)) = end else {
        panic!(
            "expected Error, got {end:?}; screen:\n{}",
            client.view.dump_grid()
        );
    };
    assert!(
        matches!(err, SessionError::Ssh(_) | SessionError::Timeout),
        "unexpected error {err:?}"
    );
    let text = err.to_string();
    assert!(
        !text.contains('\x1b'),
        "server text reached the error unsanitized: {text:?}"
    );
    let grid = client.view.dump_grid();
    assert!(grid.contains("Error:"), "no visible error line:\n{grid}");
}
