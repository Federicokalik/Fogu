//! Headless SSH probe: connects with `run_ssh`, answers prompts from stdin, runs one command in
//! the remote shell, prints the terminal and how the session ended.
//!
//! ```text
//! cargo run -p fogu-session --example ssh_probe -- user@host[:port] [-i KEY]... \
//!     [--auth auto|password|public_key|agent|keyboard_interactive] [--no-verify] [--no-clear]
//!     [--cmd CMD]
//! ```
//! Host-key prompts read `a` (accept once), `s` (accept and save) or anything else (reject);
//! secrets are read as one line each (echoed: this is a test tool). Our `known_hosts` is
//! `$XDG_CONFIG_HOME/fogu/known_hosts`.
#![expect(clippy::print_stdout, reason = "command-line example")]

use std::io::{BufRead as _, Write as _};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

use bytes::Bytes;
use fogu_core::{EffectiveSsh, QuickConnect, Size, SshAuth};
use fogu_session::{
    PromptKind, PromptReply, SessionInput, SessionOutput, SessionRuntime, SshPaths, open_ssh,
};
use fogu_term::{TermHandle, WORD_SEPARATORS};
use secrecy::SecretString;

/// No `anyhow` dependency for one example: a boxed error is enough.
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const SIZE: Size = Size {
    cols: 100,
    rows: 30,
    px_w: 0,
    px_h: 0,
};

fn main() -> Result<()> {
    let (ssh, cmd) = parse_args()?;
    let rt = SessionRuntime::new()?;
    let term = TermHandle::new(SIZE.cols, SIZE.rows, 1000, WORD_SEPARATORS);
    let view = term.share();
    let mut session = open_ssh(rt.handle(), &ssh, SshPaths::default_for_user(), SIZE, term);
    let mut sent = false;
    let end = rt.handle().block_on(async {
        loop {
            let event = tokio::time::timeout(Duration::from_secs(150), session.events.recv())
                .await
                .ok()
                .flatten();
            match event {
                None => return "no more events (timeout or session gone)".to_owned(),
                Some(SessionOutput::Wakeup) => {
                    view.dirty().store(false, Ordering::Release);
                    // Send the command once a shell prompt (`$`/`#` ending the last line) shows.
                    let grid = view.dump_grid();
                    let last = grid.lines().rev().find(|l| !l.trim().is_empty());
                    let at_prompt = last.is_some_and(|l| l.trim_end().ends_with(['$', '#']));
                    if !sent && at_prompt {
                        sent = true;
                        let line = Bytes::from(format!("{cmd}\nexit\n"));
                        let _closed = session.input.send(SessionInput::Data(line)).await;
                    }
                }
                Some(SessionOutput::Prompt(id, kind, _via)) => {
                    let reply = answer(&kind);
                    let _closed = session
                        .input
                        .send(SessionInput::PromptReply(id, reply))
                        .await;
                }
                Some(SessionOutput::Warning(w)) => println!("[warning] {w:?}"),
                Some(SessionOutput::Exited(info)) => return format!("exited: {info:?}"),
                Some(SessionOutput::Error(err)) => return format!("error: {err}"),
                Some(_) => {}
            }
        }
    });
    println!(
        "----- terminal -----\n{}\n--------------------\n{end}",
        view.dump_grid()
    );
    Ok(())
}

fn parse_args() -> Result<(EffectiveSsh, String)> {
    let mut args = std::env::args().skip(1);
    let target = args
        .next()
        .ok_or("usage: ssh_probe user@host[:port] [options]")?;
    let qc = QuickConnect::parse(&target)?;
    let mut ssh = EffectiveSsh::new(qc.host);
    if let Some(user) = qc.user {
        ssh.user = user;
    }
    ssh.port = qc.port.unwrap_or(22);
    let mut cmd = "echo PROBE_OK $(whoami)".to_owned();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-i" => ssh
                .private_keys
                .push(PathBuf::from(args.next().ok_or("-i needs a path")?)),
            "--no-verify" => ssh.verify_host_keys = false,
            "--no-clear" => ssh.clear_service_messages_on_connect = false,
            "--cmd" => cmd = args.next().ok_or("--cmd needs a command")?,
            "--auth" => {
                ssh.auth = match args.next().as_deref() {
                    Some("password") => SshAuth::Password,
                    Some("public_key") => SshAuth::PublicKey,
                    Some("agent") => SshAuth::Agent,
                    Some("keyboard_interactive") => SshAuth::KeyboardInteractive,
                    _ => SshAuth::Auto,
                };
            }
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
    Ok((ssh, cmd))
}

fn read_line(question: &str) -> String {
    print!("{question} ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let _ = std::io::stdin().lock().read_line(&mut line);
    line.trim_end_matches(['\r', '\n']).to_owned()
}

fn answer(kind: &PromptKind) -> PromptReply {
    match kind {
        PromptKind::HostKeyUnknown {
            host,
            port,
            key_type,
            fingerprint,
        } => {
            let a = read_line(&format!(
                "[prompt] unknown host key {host}:{port} {key_type} {fingerprint} [a/s/r]?"
            ));
            host_key_reply(&a)
        }
        PromptKind::HostKeyChanged {
            host,
            port,
            fingerprint,
            old_fingerprint,
            known_hosts_path,
            line,
            ..
        } => {
            let a = read_line(&format!(
                "[prompt] HOST KEY CHANGED {host}:{port} new {fingerprint} old {old_fingerprint} ({}:{line}) [s=replace/r]?",
                known_hosts_path.display()
            ));
            host_key_reply(&a)
        }
        PromptKind::Password { user, host } => PromptReply::Secret(SecretString::from(read_line(
            &format!("[prompt] password for {user}@{host}:"),
        ))),
        PromptKind::KeyPassphrase { path } => PromptReply::Secret(SecretString::from(read_line(
            &format!("[prompt] passphrase for {}:", path.display()),
        ))),
        PromptKind::KeyboardInteractive {
            name,
            instructions,
            prompts,
        } => {
            println!("[prompt] keyboard-interactive {name:?} {instructions:?}");
            PromptReply::Secrets(
                prompts
                    .iter()
                    .map(|p| {
                        SecretString::from(read_line(&format!(
                            "[prompt]   {} (echo={})",
                            p.text, p.echo
                        )))
                    })
                    .collect(),
            )
        }
    }
}

fn host_key_reply(answer: &str) -> PromptReply {
    match answer {
        "a" => PromptReply::Accept,
        "s" => PromptReply::AcceptAndSave,
        _ => PromptReply::Reject,
    }
}
