//! Headless tests of the local-shell session: real `sh` in a real PTY.
#![expect(clippy::unwrap_used, clippy::panic, reason = "tests")]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::sync::atomic::Ordering;
use std::time::Duration;

use bytes::Bytes;
use fogu_core::{EffectiveProfile, LocalProfile, Size};
use fogu_session::{
    ExitInfo, SessionError, SessionHandle, SessionInput, SessionOutput, SessionRuntime, build_env,
    open, supervise,
};
use fogu_term::{TermHandle, WORD_SEPARATORS};
use tokio::sync::mpsc;

const SIZE: Size = Size {
    cols: 80,
    rows: 24,
    px_w: 0,
    px_h: 0,
};

fn profile(program: &str, args: &[&str]) -> LocalProfile {
    LocalProfile {
        program: Some(program.to_owned()),
        args: args.iter().map(|a| (*a).to_owned()).collect(),
        ..LocalProfile::default()
    }
}

/// Opens a session and returns it plus a UI-side view of its terminal.
fn start(rt: &SessionRuntime, profile: &LocalProfile) -> (SessionHandle, TermHandle) {
    let term = TermHandle::new(SIZE.cols, SIZE.rows, 1000, WORD_SEPARATORS);
    let view = term.share();
    let profile = EffectiveProfile::Local(profile.clone());
    (open(rt.handle(), &profile, SIZE, term), view)
}

/// Receives events (clearing `dirty` like the UI does) until `done` returns true or 5 s pass.
fn wait_until(
    rt: &SessionRuntime,
    session: &mut SessionHandle,
    view: &TermHandle,
    mut done: impl FnMut(&SessionOutput, &TermHandle) -> bool,
) -> SessionOutput {
    rt.handle().block_on(async {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let event = session.events.recv().await.unwrap();
                if matches!(event, SessionOutput::Wakeup) {
                    view.dirty().store(false, Ordering::Release);
                }
                if done(&event, view) {
                    return event;
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out; grid:\n{}", view.dump_grid()))
    })
}

#[test]
fn exit_code_and_output_are_reported() {
    let rt = SessionRuntime::new().unwrap();
    let (mut session, view) = start(&rt, &profile("sh", &["-c", "printf 'hello\\n'; exit 3"]));
    let event = wait_until(&rt, &mut session, &view, |e, _| {
        matches!(e, SessionOutput::Exited(_))
    });
    let SessionOutput::Exited(info) = event else {
        unreachable!()
    };
    assert_eq!(
        info,
        ExitInfo {
            code: Some(3),
            signal: None
        }
    );
    assert!(view.dump_grid().contains("hello"), "{}", view.dump_grid());
}

#[test]
fn input_reaches_interactive_shell() {
    let rt = SessionRuntime::new().unwrap();
    let (mut session, view) = start(&rt, &profile("sh", &[]));
    session
        .input
        .try_send(SessionInput::Data(Bytes::from_static(b"echo abc\n")))
        .unwrap();
    // The input may be echoed before the shell prints its prompt, so the output can land after
    // the prompt ("$ abc") instead of on its own line: match any non-echo line ending in "abc".
    wait_until(&rt, &mut session, &view, |_, v| {
        v.dump_grid()
            .lines()
            .any(|line| line.trim_end().ends_with("abc") && !line.contains("echo"))
    });
}

#[test]
fn close_ends_a_running_process_quickly() {
    let rt = SessionRuntime::new().unwrap();
    let (mut session, view) = start(&rt, &profile("sleep", &["100"]));
    session.input.try_send(SessionInput::Close).unwrap();
    let started = std::time::Instant::now();
    wait_until(&rt, &mut session, &view, |e, _| {
        matches!(e, SessionOutput::Exited(_))
    });
    assert!(started.elapsed() < Duration::from_secs(3));
}

/// A shell that ignores SIGHUP (and hands that disposition to `sleep`) must still be gone after
/// the 2 s grace: the SIGKILL-the-process-group fallback, reported as signal 9.
#[test]
fn close_falls_back_to_sigkill_after_the_grace_period() {
    let rt = SessionRuntime::new().unwrap();
    let script = "trap '' HUP; echo READY; exec sleep 100";
    let (mut session, view) = start(&rt, &profile("sh", &["-c", script]));
    // Close only once the trap is installed, or SIGHUP alone would end the shell.
    wait_until(&rt, &mut session, &view, |_, v| {
        v.dump_grid().contains("READY")
    });
    session.input.try_send(SessionInput::Close).unwrap();
    let started = std::time::Instant::now();
    let event = wait_until(&rt, &mut session, &view, |e, _| {
        matches!(e, SessionOutput::Exited(_))
    });
    let elapsed = started.elapsed();
    let SessionOutput::Exited(info) = event else {
        unreachable!()
    };
    assert_eq!(info.signal, Some(9), "{info:?}");
    assert!(
        elapsed >= Duration::from_secs(2) && elapsed < Duration::from_secs(4),
        "{elapsed:?}"
    );
}

#[test]
fn build_env_removes_sets_and_expands() {
    let parent: Vec<(OsString, OsString)> = [
        ("PATH", "/usr/bin"),
        ("HOME", "/home/me"),
        ("APPIMAGE", "/x.AppImage"),
        ("LD_PRELOAD", "libhack.so"),
        ("DESKTOP_STARTUP_ID", "abc"),
        ("RUST_LOG", "debug"),
        ("TERM", "dumb"),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), v.into()))
    .collect();
    let profile_env: BTreeMap<String, String> = [
        ("PATH", "${PATH}:${HOME}/bin"),
        ("EMPTY", "a${NOPE}b"),
        ("OPEN", "x${HOME"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect();
    let env = build_env(parent, &profile_env);
    let get = |k: &str| env.get(&OsString::from(k)).map(|v| v.to_str().unwrap());

    let cases: &[(&str, Option<&str>)] = &[
        ("APPIMAGE", None),
        ("LD_PRELOAD", None),
        ("DESKTOP_STARTUP_ID", None),
        ("RUST_LOG", None),
        ("TERM", Some("xterm-256color")),
        ("COLORTERM", Some("truecolor")),
        ("TERM_PROGRAM", Some("fogu")),
        ("HOME", Some("/home/me")),
        ("PATH", Some("/usr/bin:/home/me/bin")),
        ("EMPTY", Some("ab")),
        ("OPEN", Some("x${HOME")),
    ];
    for (key, want) in cases {
        assert_eq!(get(key), *want, "{key}");
    }

    // Without APPIMAGE, LD_* belong to the user and are kept.
    let env = build_env(
        [(OsString::from("LD_PRELOAD"), OsString::from("mine.so"))],
        &BTreeMap::new(),
    );
    assert!(env.contains_key(&OsString::from("LD_PRELOAD")));
}

#[test]
fn supervise_maps_panic_to_crashed() {
    let rt = SessionRuntime::new().unwrap();
    let (out_tx, mut out_rx) = mpsc::channel(4);
    let task = rt.handle().spawn(async { panic!("deliberate test panic") });
    rt.handle().spawn(supervise(task, out_tx));
    let event = rt.handle().block_on(out_rx.recv()).unwrap();
    assert!(
        matches!(event, SessionOutput::Error(SessionError::Crashed)),
        "{event:?}"
    );
}

#[test]
fn ssh_profiles_fail_until_s18() {
    let rt = SessionRuntime::new().unwrap();
    let term = TermHandle::new(SIZE.cols, SIZE.rows, 1000, WORD_SEPARATORS);
    let profile = EffectiveProfile::Ssh(fogu_core::EffectiveSsh {
        host: "example.org".into(),
        port: 22,
        user: None,
    });
    let mut handle = open(rt.handle(), &profile, SIZE, term);
    let event = rt.handle().block_on(async {
        tokio::time::timeout(Duration::from_secs(5), handle.events.recv()).await
    });
    let Ok(Some(SessionOutput::Error(SessionError::Spawn { program, .. }))) = event else {
        panic!("expected a spawn error, got {event:?}");
    };
    assert_eq!(program, "ssh example.org");
}

#[test]
fn input_backlog_is_bounded_and_reported_once() {
    let rt = SessionRuntime::new().unwrap();
    let (mut session, view) = start(&rt, &profile("sh", &["-c", "sleep 100"]));
    // 10 MiB of "paste" into a program that never reads its tty: the backlog caps at 4 MiB.
    rt.handle().block_on(async {
        for _ in 0..10 {
            session
                .input
                .send(SessionInput::Data(Bytes::from(vec![b'x'; 1024 * 1024])))
                .await
                .unwrap();
        }
    });
    wait_until(&rt, &mut session, &view, |e, _| {
        matches!(e, SessionOutput::InputDropped)
    });
    // Resize and close still flow while the backlog is full.
    assert!(session.input.try_send(SessionInput::Resize(SIZE)).is_ok());
    session.input.try_send(SessionInput::Close).unwrap();
    wait_until(&rt, &mut session, &view, |e, _| {
        matches!(e, SessionOutput::Exited(_))
    });
}
