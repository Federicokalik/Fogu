//! The shell channel against real OpenSSH: commands, resize, close, banner.

use std::time::{Duration, Instant};

use fogu_session::SessionOutput;

use super::no_prompts;
use crate::support::client::Client;
use crate::support::sshd::{Options, TestSshd};

#[test]
fn key_auth_runs_a_command_and_exit_ends_the_session() {
    let Some(sshd) = TestSshd::start() else {
        return;
    };
    sshd.trust();
    let mut client = Client::connect(&sshd.profile(), sshd.paths(), no_prompts);
    client.wait_shell();
    client.send("echo marker-$((6*7))-$$\r");
    client.expect_text("marker-42-");
    client.send("exit 3\r");
    let SessionOutput::Exited(info) = client.end() else {
        panic!("expected Exited; screen:\n{}", client.view.dump_grid());
    };
    assert_eq!(info.code, Some(3));
}

#[test]
fn resize_reaches_the_remote_pty() {
    let Some(sshd) = TestSshd::start() else {
        return;
    };
    sshd.trust();
    let mut client = Client::connect(&sshd.profile(), sshd.paths(), no_prompts);
    client.wait_shell();
    client.send("stty size\r");
    client.expect_text("24 80");
    client.resize(100, 30);
    // The session debounces window-change; ask until the new size shows.
    let mut seen = false;
    for _ in 0..20 {
        client.send("stty size\r");
        if client.wait_text_for(Duration::from_millis(400), "30 100") {
            seen = true;
            break;
        }
    }
    assert!(
        seen,
        "window-change never arrived; screen:\n{}",
        client.view.dump_grid()
    );
}

#[test]
fn close_ends_the_session_quickly() {
    let Some(sshd) = TestSshd::start() else {
        return;
    };
    sshd.trust();
    let mut client = Client::connect(&sshd.profile(), sshd.paths(), no_prompts);
    client.wait_shell();
    let started = Instant::now();
    client.close();
    let end = client.until_for(Duration::from_secs(3), |e| {
        matches!(e, SessionOutput::Exited(_) | SessionOutput::Error(_))
    });
    assert!(end.is_some(), "no end event within 3 s");
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn banner_escape_sequences_arrive_sanitized() {
    let options = Options {
        banner: Some(b"Welcome\x1b[31mRED\x1b]0;pwned\x07 end\n".to_vec()),
    };
    let Some(sshd) = TestSshd::start_with(&options) else {
        return;
    };
    sshd.trust();
    let mut ssh = sshd.profile();
    ssh.clear_service_messages_on_connect = false;
    let mut client = Client::connect(&ssh, sshd.paths(), no_prompts);
    client.wait_shell();
    // ESC and BEL are dropped, so the rest of the sequences shows up as plain text: had the
    // terminal interpreted them, neither "[31m" nor "]0;pwned" would be on the screen.
    let grid = client.view.dump_grid();
    assert!(grid.contains("Welcome[31mRED]0;pwned end"), "{grid}");
}

#[test]
fn cwd_and_login_scripts_run_once_the_shell_starts() {
    let Some(sshd) = TestSshd::start() else {
        return;
    };
    sshd.trust();
    let mut ssh = sshd.profile();
    ssh.cwd = Some("/tmp".to_owned());
    ssh.scripts = vec![
        fogu_core::LoginScript {
            expect: String::new(),
            send: r"echo script-\x41-$((6*7))".to_owned(),
            is_regex: false,
            optional: false,
        },
        fogu_core::LoginScript {
            expect: "script-A-42".to_owned(),
            send: "echo second-$(pwd)".to_owned(),
            is_regex: false,
            optional: false,
        },
    ];
    let mut client = Client::connect(&ssh, sshd.paths(), no_prompts);
    client.wait_shell();
    // The `cd` goes first, so the second script sees /tmp.
    client.expect_text("second-/tmp");
}
