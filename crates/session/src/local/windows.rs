//! `run_local`: a local shell in a `ConPTY`, one task per tab.
//!
//! `portable-pty` exposes blocking readers and writers even on Windows. Two small blocking
//! tasks bridge them to the async session loop: one reads PTY output, one writes queued input.
//! The loop itself keeps the same shape as the Unix implementation (input first, one write and
//! one read per turn, synchronized-update timer, child exit).

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use fogu_core::{LocalProfile, Size};
use fogu_term::TermHandle;
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtyPair, PtySize, native_pty_system};
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::time::{Instant, sleep_until, timeout};

use crate::env::build_env;
use crate::pipeline::{Gone, Pump, WRITE_CHUNK, canonical_dir};
use crate::{ExitInfo, SessionError, SessionInput, SessionOutput};

/// PTY read buffer.
const READ_CHUNK: usize = 64 * 1024;
/// Time between closing the master and killing the child.
const CLOSE_GRACE: Duration = Duration::from_secs(2);
/// Upper bound for draining output after the child exited.
/// How often a watched foreground program is checked (kept for API parity; Windows has no
/// portable foreground-process query).
const WATCH_EVERY: Duration = Duration::from_secs(1);

/// A spawned shell and our ends of its `ConPTY`.
struct Local {
    master: Box<dyn MasterPty + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    /// Output from the blocking reader task.
    read_rx: Receiver<io::Result<Vec<u8>>>,
    /// Queue to the blocking writer task.
    write_tx: Sender<Vec<u8>>,
    /// Written-byte counts / errors from the writer task.
    write_rx: Receiver<io::Result<usize>>,
    /// The child's exit status, from the blocking wait task.
    wait_rx: tokio::task::JoinHandle<io::Result<i32>>,
    /// The PTY read side hit EOF.
    eof: bool,
}

impl Local {
    fn resize(&self, size: Size) {
        let pty_size = pty_size(size);
        if let Err(err) = self.master.resize(pty_size) {
            tracing::warn!(error = %err, "PTY resize failed");
        }
    }
}

/// Session task for a local shell. Errors are reported as `SessionOutput::Error`.
pub(crate) async fn run_local(
    profile: LocalProfile,
    size: Size,
    term: TermHandle,
    mut in_rx: Receiver<SessionInput>,
    out_tx: Sender<SessionOutput>,
) {
    let mut pump = Pump::new(term, out_tx, false);
    let mut local = match spawn(&profile, size) {
        Ok(local) => local,
        Err(err) => {
            tracing::warn!(error = %err, "local shell failed to start");
            let _gone = pump.emit(SessionOutput::Error(err)).await;
            return;
        }
    };
    let Ok(status) = run_loop(&mut local, &mut pump, &mut in_rx).await else {
        let _status = close(&mut local).await;
        return;
    };
    let out = match status {
        Ok(code) => SessionOutput::Exited(ExitInfo {
            code: Some(code),
            signal: None,
        }),
        Err(err) => SessionOutput::Error(SessionError::Io(Arc::new(err))),
    };
    let _gone = pump.emit(out).await;
}

async fn run_loop(
    local: &mut Local,
    pump: &mut Pump,
    in_rx: &mut Receiver<SessionInput>,
) -> Result<io::Result<i32>, Gone> {
    let mut watching: Option<Option<String>> = None;
    let mut watch_tick = tokio::time::interval(WATCH_EVERY);
    let mut write_in_flight = false;
    loop {
        let sync_deadline = pump.term.sync_timeout();
        tokio::select! {
            biased;
            input = in_rx.recv() => match input {
                Some(SessionInput::Data(data)) => {
                    if pump.queue_input(&data) {
                        tracing::warn!(len = data.len(), "input backlog full; dropping input");
                        pump.emit(SessionOutput::InputDropped).await?;
                    }
                }
                Some(SessionInput::Resize(size)) => local.resize(size),
                Some(SessionInput::PromptReply(id, _)) => {
                    tracing::debug!(?id, "prompt reply for a local session ignored");
                }
                Some(SessionInput::Forward(_)) => tracing::debug!("forward for a local session ignored"),
                Some(SessionInput::QueryCwd) => {
                    // Windows has no portable `/proc`-style cwd lookup. Local cwd updates
                    // arrive through OSC 7 when the shell integration reports them.
                    pump.emit(SessionOutput::Foreground(None)).await?;
                }
                Some(SessionInput::WatchForeground(on)) => {
                    watching = on.then_some(None);
                }
                Some(SessionInput::Close) | None => return Ok(close(local).await),
            },
            written = local.write_rx.recv(), if write_in_flight => {
                write_in_flight = false;
                match written {
                    Some(Ok(n)) => pump.written(n),
                    Some(Err(err)) => {
                        tracing::debug!(error = %err, "PTY write failed; dropping pending input");
                        pump.write_failed();
                    }
                    None => {
                        pump.write_failed();
                    }
                }
            }
            read = local.read_rx.recv(), if !local.eof => match read {
                Some(Ok(bytes)) if bytes.is_empty() => local.eof = true,
                Some(Ok(bytes)) => pump.feed(&bytes).await?,
                Some(Err(err)) => {
                    tracing::warn!(error = %err, "PTY read failed");
                    local.eof = true;
                }
                None => local.eof = true,
            },
            () = sleep_until(sync_deadline.map_or_else(Instant::now, Instant::from_std)),
                if sync_deadline.is_some() => pump.feed(&[]).await?,
            _ = watch_tick.tick(), if watching.is_some() => {
                // No portable foreground-process query on Windows; the watch ends when the
                // shell writes the next prompt (the UI only uses this for a status hint).
                watching = Some(None);
            }
            status = &mut local.wait_rx => {
                return Ok(match status {
                    Ok(Ok(code)) => Ok(code),
                    Ok(Err(err)) => Err(err),
                    Err(err) => Err(io::Error::other(err.to_string())),
                });
            }
        }

        // Queue at most one write at a time; input accumulates in `pump.pending` meanwhile.
        if !write_in_flight && !pump.pending.is_empty() {
            let chunk = pump.pending.len().min(WRITE_CHUNK);
            let bytes = pump.pending.get(..chunk).unwrap_or_default().to_vec();
            if local.write_tx.send(bytes).await.is_err() {
                pump.write_failed();
            } else {
                write_in_flight = true;
            }
        }
    }
}

/// Close: kill the child, wait up to [`CLOSE_GRACE`], then reap.
async fn close(local: &mut Local) -> io::Result<i32> {
    let _ = local.killer.kill();
    match timeout(CLOSE_GRACE, &mut local.wait_rx).await {
        Ok(Ok(Ok(code))) => Ok(code),
        Ok(Ok(Err(err))) => Err(err),
        Ok(Err(err)) => Err(io::Error::other(err.to_string())),
        Err(_elapsed) => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "the shell did not exit after being killed",
        )),
    }
}

fn pty_size(size: Size) -> PtySize {
    PtySize {
        rows: size.rows.max(1),
        cols: size.cols.max(2),
        pixel_width: size.px_w,
        pixel_height: size.px_h,
    }
}

fn spawn(profile: &LocalProfile, size: Size) -> Result<Local, SessionError> {
    let program = profile.program.clone().unwrap_or_else(default_shell);
    let spawn_err = |source: io::Error| SessionError::Spawn {
        program: program.clone(),
        source: Arc::new(source),
    };
    let pair = native_pty_system()
        .openpty(pty_size(size))
        .map_err(|err| spawn_err(io::Error::other(err.to_string())))?;
    let mut cmd = CommandBuilder::new(&program);
    cmd.args(&profile.args);
    let cwd = profile
        .cwd
        .clone()
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    cmd.cwd(cwd);
    cmd.env_clear();
    for (key, value) in build_env(std::env::vars_os(), &profile.env) {
        cmd.env(key, value);
    }
    let PtyPair { slave, master } = pair;
    let child = slave
        .spawn_command(cmd)
        .map_err(|err| spawn_err(io::Error::other(err.to_string())))?;
    drop(slave);
    let reader = master
        .try_clone_reader()
        .map_err(|err| spawn_err(io::Error::other(err.to_string())))?;
    let writer = master
        .take_writer()
        .map_err(|err| spawn_err(io::Error::other(err.to_string())))?;
    let killer = child.clone_killer();

    let (read_tx, read_rx) = mpsc::channel(8);
    let (write_tx, mut write_queue) = mpsc::channel::<Vec<u8>>(8);
    let (write_done_tx, write_rx) = mpsc::channel(8);

    tokio::task::spawn_blocking(move || read_loop(reader, read_tx));
    tokio::task::spawn_blocking(move || write_loop(writer, &mut write_queue, write_done_tx));
    let wait_handle = tokio::task::spawn_blocking(move || {
        let mut child = child;
        child.wait().map(|status| status.exit_code().cast_signed())
    });

    Ok(Local {
        master,
        killer,
        read_rx,
        write_tx,
        write_rx,
        wait_rx: wait_handle,
        eof: false,
    })
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "the sender is moved into a blocking task"
)]
fn read_loop(mut reader: Box<dyn io::Read + Send>, tx: Sender<io::Result<Vec<u8>>>) {
    let mut buf = vec![0_u8; READ_CHUNK];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => {
                let _sent = tx.blocking_send(Ok(Vec::new()));
                return;
            }
            Ok(n) => {
                let chunk = buf.get(..n).unwrap_or_default().to_vec();
                if tx.blocking_send(Ok(chunk)).is_err() {
                    return;
                }
            }
            Err(err) => {
                let _sent = tx.blocking_send(Err(err));
                return;
            }
        }
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "the sender is moved into a blocking task"
)]
fn write_loop(
    mut writer: Box<dyn io::Write + Send>,
    queue: &mut Receiver<Vec<u8>>,
    done: Sender<io::Result<usize>>,
) {
    while let Some(bytes) = queue.blocking_recv() {
        let result = writer.write_all(&bytes).map(|()| bytes.len());
        let failed = result.is_err();
        if done.blocking_send(result).is_err() || failed {
            return;
        }
    }
}

/// The user's default shell on Windows: PowerShell 7, then Windows PowerShell, then `cmd.exe`.
fn default_shell() -> String {
    for name in ["pwsh.exe", "powershell.exe"] {
        if let Some(path) = find_in_path(name) {
            return path.to_string_lossy().into_owned();
        }
    }
    std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_owned())
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|path| path.is_file())
}

/// Kept for parity with the Unix tests and for callers that need a canonical local directory.
#[allow(dead_code)]
fn checked_cwd(path: &Path) -> Option<PathBuf> {
    canonical_dir(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pty_size_never_uses_zero_columns_or_rows() {
        let size = pty_size(Size::default());
        assert_eq!((size.cols, size.rows), (2, 1));
    }

    #[test]
    fn default_shell_is_never_empty() {
        assert_ne!(default_shell(), "");
    }
}
