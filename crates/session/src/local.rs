//! `run_local`: a local shell in a PTY, one task per tab.
//!
//! One `select!` loop: input (never waits behind output), pending writes in ≤ 4 KiB chunks, one
//! PTY read of ≤ 64 KiB per iteration (backpressure: we only read as fast as we parse), the
//! synchronized-update timer, and child exit.

use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::process::ExitStatusExt as _;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use bytes::{Buf as _, BytesMut};
use fogu_core::{LocalProfile, Size};
use fogu_term::{OscEvent, OscScanner, TermEvent, TermHandle};
use pty_process::{OwnedReadPty, OwnedWritePty};
use rustix::process::{Pid, Signal};
use rustix::termios::Winsize;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::process::Child;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::time::{Instant, sleep_until, timeout, timeout_at};

use crate::env::build_env;
use crate::{ClipboardRequest, ExitInfo, SessionError, SessionInput, SessionOutput};

/// PTY read buffer.
const READ_CHUNK: usize = 64 * 1024;
/// Largest single PTY write.
const WRITE_CHUNK: usize = 4 * 1024;
/// Terminal replies (`PtyWrite`) are dropped while this much is already waiting to be written,
/// so a program flooding queries without reading its input cannot grow our memory.
const MAX_PENDING_FOR_REPLIES: usize = 1024 * 1024;
/// Time between SIGHUP (closing the master) and SIGKILL of the process group.
const CLOSE_GRACE: Duration = Duration::from_secs(2);
/// Upper bound for draining output after the child exited (a background job may keep the slave).
const DRAIN_TIMEOUT: Duration = Duration::from_millis(500);

/// The UI dropped its event receiver; stop quietly.
struct Gone;

/// Why the main loop stopped.
enum End {
    Exited(io::Result<ExitStatus>),
    Close,
}

/// Feeds bytes into the terminal and routes the resulting events.
struct Pump {
    term: TermHandle,
    dirty: Arc<AtomicBool>,
    out_tx: Sender<SessionOutput>,
    /// Bytes waiting to be written to the PTY (user input + terminal replies).
    pending: BytesMut,
    /// Side-channel scanner for OSC codes `Term`/`vte` drop.
    osc: OscScanner,
    /// This machine's hostname, used to tell an OSC 7 report of a local path from one made on a
    /// remote host reached by a plain `ssh` inside this shell.
    local_hostname: String,
}

impl Pump {
    async fn feed(&mut self, bytes: &[u8]) -> Result<(), Gone> {
        // Non-consuming: `bytes` is fed to the terminal unchanged below.
        let osc_events = self.osc.scan(bytes);
        self.term.feed(bytes);
        for event in osc_events {
            if let Some(out) = self.map_osc(event) {
                self.emit(out).await?;
            }
        }
        for event in self.term.drain_events() {
            let out = match event {
                TermEvent::PtyWrite(reply) => {
                    if self.pending.len() < MAX_PENDING_FOR_REPLIES {
                        self.pending.extend_from_slice(&reply);
                    }
                    continue;
                }
                TermEvent::Title(title) => SessionOutput::Title(title),
                TermEvent::ResetTitle => SessionOutput::Title(String::new()),
                TermEvent::Bell => SessionOutput::Bell,
                TermEvent::ClipboardStore {
                    target,
                    payload,
                    len,
                } => SessionOutput::ClipboardStore(ClipboardRequest {
                    target,
                    payload,
                    len,
                }),
                TermEvent::CursorBlinkingChange => continue,
            };
            self.emit(out).await?;
        }
        if !self.dirty.swap(true, Ordering::AcqRel) {
            self.emit(SessionOutput::Wakeup).await?;
        }
        Ok(())
    }

    async fn emit(&self, out: SessionOutput) -> Result<(), Gone> {
        self.out_tx.send(out).await.map_err(|_| Gone)
    }

    /// Maps a scanner event to a `SessionOutput`, logging the ones the UI does not use
    /// yet.
    fn map_osc(&self, event: OscEvent) -> Option<SessionOutput> {
        Some(match event {
            OscEvent::Cwd { host, path } => {
                let remote = !is_local_host(&host, &self.local_hostname);
                let path = if remote {
                    Some(path)
                } else {
                    canonical_dir(&path)
                }?;
                tracing::debug!(path = %path.display(), remote, "cwd");
                SessionOutput::Cwd { path, remote }
            }
            OscEvent::Progress(pct) => {
                tracing::debug!(?pct, "progress");
                SessionOutput::Progress(pct)
            }
            OscEvent::Notify { title, body } => {
                tracing::debug!(?title, %body, "notify (no UI yet)");
                SessionOutput::Notify { title, body }
            }
            OscEvent::Prompt(mark) => {
                tracing::debug!(?mark, "prompt mark (unused)");
                SessionOutput::PromptMark(mark)
            }
        })
    }
}

/// `true` when `host` (an OSC 7 host field) names this machine: empty (no host given) or
/// matching `local_hostname` case-insensitively.
fn is_local_host(host: &str, local_hostname: &str) -> bool {
    host.is_empty()
        || host.eq_ignore_ascii_case("localhost")
        || (!local_hostname.is_empty() && host.eq_ignore_ascii_case(local_hostname))
}

/// Resolves and validates a reported local path; `None` if it doesn't canonicalize to an
/// existing directory.
fn canonical_dir(path: &Path) -> Option<PathBuf> {
    let canon = path.canonicalize().ok()?;
    canon.is_dir().then_some(canon)
}

/// This machine's hostname (`/proc/sys/kernel/hostname`), for [`is_local_host`]. Empty on
/// failure, which only ever widens what counts as "local" by one fewer alias, never narrows it.
fn local_hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|s| s.trim().to_owned())
        .unwrap_or_default()
}

/// On-demand cwd lookup (fallback when the shell reports no OSC 7): the foreground process group's
/// directory, falling back to the shell's own. `None` if neither resolves to a live directory.
fn current_cwd(master: &OwnedFd, shell_pid: Option<u32>) -> Option<PathBuf> {
    let fg_pid = rustix::termios::tcgetpgrp(master)
        .ok()
        .and_then(|pgid| u32::try_from(pgid.as_raw_nonzero().get()).ok());
    fg_pid.into_iter().chain(shell_pid).find_map(|pid| {
        let cwd = std::fs::read_link(format!("/proc/{pid}/cwd")).ok()?;
        canonical_dir(&cwd)
    })
}

/// A spawned shell and our ends of its PTY.
struct Local {
    reader: OwnedReadPty,
    writer: OwnedWritePty,
    /// Dup of the master, kept for `tcsetwinsize` (owned halves don't implement `AsFd`).
    master: OwnedFd,
    child: Child,
    /// The PTY read side hit EOF/EIO.
    eof: bool,
}

/// Session task for a local shell. Errors are reported as `SessionOutput::Error`.
pub(crate) async fn run_local(
    profile: LocalProfile,
    size: Size,
    term: TermHandle,
    mut in_rx: Receiver<SessionInput>,
    out_tx: Sender<SessionOutput>,
) {
    let mut pump = Pump {
        dirty: term.dirty(),
        term,
        out_tx,
        pending: BytesMut::new(),
        osc: OscScanner::new(),
        local_hostname: local_hostname(),
    };
    let mut local = match spawn(&profile, size) {
        Ok(local) => local,
        Err(err) => {
            tracing::warn!(error = %err, "local shell failed to start");
            let _gone = pump.emit(SessionOutput::Error(err)).await;
            return;
        }
    };
    let mut buf = vec![0_u8; READ_CHUNK];
    let status = match run_loop(&mut local, &mut pump, &mut in_rx, &mut buf).await {
        Ok(End::Exited(status)) => {
            if !local.eof && drain(&mut local.reader, &mut pump, &mut buf).await.is_err() {
                return;
            }
            status
        }
        Ok(End::Close) => close(local).await,
        Err(Gone) => {
            close(local).await.ok();
            return;
        }
    };
    let out = match status {
        Ok(status) => SessionOutput::Exited(ExitInfo {
            code: status.code(),
            signal: status.signal(),
        }),
        Err(err) => SessionOutput::Error(SessionError::Io(Arc::new(err))),
    };
    let _gone = pump.emit(out).await;
}

async fn run_loop(
    local: &mut Local,
    pump: &mut Pump,
    in_rx: &mut Receiver<SessionInput>,
    buf: &mut [u8],
) -> Result<End, Gone> {
    loop {
        let sync_deadline = pump.term.sync_timeout();
        let chunk = pump.pending.len().min(WRITE_CHUNK);
        tokio::select! {
            biased;
            input = in_rx.recv() => match input {
                Some(SessionInput::Data(data)) => pump.pending.extend_from_slice(&data),
                Some(SessionInput::Resize(size)) => resize(&local.master, size),
                Some(SessionInput::PromptReply(_, reply)) => match reply {},
                Some(SessionInput::QueryCwd) => {
                    if let Some(path) = current_cwd(&local.master, local.child.id()) {
                        pump.emit(SessionOutput::Cwd { path, remote: false }).await?;
                    }
                }
                Some(SessionInput::Close) | None => return Ok(End::Close),
            },
            written = local.writer.write(pump.pending.get(..chunk).unwrap_or_default()),
                if chunk > 0 => match written {
                Ok(n) => pump.pending.advance(n),
                Err(err) => {
                    tracing::debug!(error = %err, "PTY write failed; dropping pending input");
                    pump.pending.clear();
                }
            },
            read = local.reader.read(buf), if !local.eof => match read {
                Ok(0) => local.eof = true,
                Ok(n) => pump.feed(buf.get(..n).unwrap_or_default()).await?,
                Err(err) => {
                    if !is_eof(&err) {
                        tracing::warn!(error = %err, "PTY read failed");
                    }
                    local.eof = true;
                }
            },
            () = sleep_until(sync_deadline.map_or_else(Instant::now, Instant::from_std)),
                if sync_deadline.is_some() => pump.feed(&[]).await?,
            status = local.child.wait() => return Ok(End::Exited(status)),
        }
    }
}

/// Reads what the child left in the PTY before it exited, until EOF/EIO or [`DRAIN_TIMEOUT`].
async fn drain(reader: &mut OwnedReadPty, pump: &mut Pump, buf: &mut [u8]) -> Result<(), Gone> {
    let deadline = Instant::now() + DRAIN_TIMEOUT;
    while let Ok(Ok(n)) = timeout_at(deadline, reader.read(buf)).await {
        if n == 0 {
            break;
        }
        pump.feed(buf.get(..n).unwrap_or_default()).await?;
    }
    Ok(())
}

/// Close: drop every master fd (kernel sends SIGHUP), wait ≤ 2 s, SIGKILL the process group,
/// always reap.
async fn close(local: Local) -> io::Result<ExitStatus> {
    let Local {
        reader,
        writer,
        master,
        mut child,
        ..
    } = local;
    drop((reader, writer, master));
    if let Ok(status) = timeout(CLOSE_GRACE, child.wait()).await {
        return status;
    }
    // The child is a session leader (setsid), so its pid is also its process-group id.
    if let Some(pid) = child
        .id()
        .and_then(|id| i32::try_from(id).ok())
        .and_then(Pid::from_raw)
        && let Err(err) = rustix::process::kill_process_group(pid, Signal::KILL)
    {
        tracing::warn!(error = %err, "SIGKILL of the shell's process group failed");
    }
    child.wait().await
}

/// On Linux, reading the master after the last slave fd closed returns `EIO`: that is EOF.
fn is_eof(err: &io::Error) -> bool {
    err.raw_os_error() == Some(rustix::io::Errno::IO.raw_os_error())
}

fn winsize(size: Size) -> Winsize {
    // Never hand the kernel a 0×0 size (hidden/minimized panes): clamp to ≥ 2×1.
    Winsize {
        ws_row: size.rows.max(1),
        ws_col: size.cols.max(2),
        ws_xpixel: size.px_w,
        ws_ypixel: size.px_h,
    }
}

fn resize(master: &OwnedFd, size: Size) {
    if let Err(err) = rustix::termios::tcsetwinsize(master, winsize(size)) {
        tracing::warn!(error = %err, "PTY resize failed");
    }
}

fn spawn(profile: &LocalProfile, size: Size) -> Result<Local, SessionError> {
    let program = profile.program.clone().unwrap_or_else(default_shell);
    let spawn_err = |source: io::Error| SessionError::Spawn {
        program: program.clone(),
        source: Arc::new(source),
    };
    let (pty, pts) = pty_process::open().map_err(|e| spawn_err(pty_io(e)))?;
    let ws = winsize(size);
    pty.resize(pty_process::Size::new_with_pixel(
        ws.ws_row,
        ws.ws_col,
        ws.ws_xpixel,
        ws.ws_ypixel,
    ))
    .map_err(|e| spawn_err(pty_io(e)))?;
    let cwd = profile
        .cwd
        .clone()
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/"));
    let mut cmd = pty_process::Command::new(&program)
        .args(&profile.args)
        .current_dir(cwd)
        .env_clear()
        .envs(build_env(std::env::vars_os(), &profile.env));
    if profile.login {
        cmd = cmd.arg0(login_argv0(&program));
    }
    // `spawn` consumes `pts`, so our copy of the slave is gone and EOF/EIO arrives on exit.
    let child = cmd.spawn(pts).map_err(|e| spawn_err(pty_io(e)))?;
    let master = rustix::io::dup(&pty).map_err(|e| spawn_err(e.into()))?;
    let (reader, writer) = pty.into_split();
    Ok(Local {
        reader,
        writer,
        master,
        child,
        eof: false,
    })
}

fn pty_io(err: pty_process::Error) -> io::Error {
    match err {
        pty_process::Error::Io(err) => err,
        pty_process::Error::Rustix(errno) => errno.into(),
        err @ pty_process::Error::Unsplit(..) => io::Error::other(err.to_string()),
    }
}

/// `$SHELL` → the passwd entry of the current uid → `/bin/sh`.
fn default_shell() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|shell| !shell.is_empty())
        .or_else(|| {
            let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
            shell_from_passwd(&passwd, rustix::process::getuid().as_raw())
        })
        .unwrap_or_else(|| "/bin/sh".to_owned())
}

/// Login shell field of `uid`'s line in an `/etc/passwd`-format file.
fn shell_from_passwd(passwd: &str, uid: u32) -> Option<String> {
    passwd.lines().find_map(|line| {
        let mut fields = line.split(':');
        let line_uid = fields.nth(2)?.parse::<u32>().ok()?;
        let shell = fields.nth(3)?;
        (line_uid == uid && !shell.is_empty()).then(|| shell.to_owned())
    })
}

/// `"/usr/bin/zsh"` → `"-zsh"`.
fn login_argv0(program: &str) -> String {
    let name = Path::new(program)
        .file_name()
        .map_or_else(|| program.into(), |name| name.to_string_lossy());
    format!("-{name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwd_lookup_by_uid() {
        let passwd = "root:x:0:0::/root:/bin/bash\n\
                      broken line\n\
                      me:x:1000:1000:Me:/home/me:/usr/bin/zsh\n\
                      noshell:x:1001:1001::/home/n:\n";
        assert_eq!(
            shell_from_passwd(passwd, 1000).as_deref(),
            Some("/usr/bin/zsh")
        );
        assert_eq!(shell_from_passwd(passwd, 0).as_deref(), Some("/bin/bash"));
        assert_eq!(shell_from_passwd(passwd, 1001), None);
        assert_eq!(shell_from_passwd(passwd, 4242), None);
    }

    #[test]
    fn login_argv0_uses_basename() {
        assert_eq!(login_argv0("/usr/bin/zsh"), "-zsh");
        assert_eq!(login_argv0("bash"), "-bash");
    }

    #[test]
    fn winsize_never_zero() {
        let ws = winsize(Size::default());
        assert_eq!((ws.ws_col, ws.ws_row), (2, 1));
    }
}
