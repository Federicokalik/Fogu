//! `fogu-session`: the one session API: `open()` spawns a transport task per tab that
//! feeds a [`TermHandle`] and talks to the UI over two bounded channels. Transports: a local
//! shell (`run_local`) and, behind the default `ssh` feature, an SSH shell channel (`run_ssh`).

mod env;
mod local;
mod pipeline;
mod prompt;
pub mod sftp;
#[cfg(feature = "ssh")]
mod ssh;
mod supervise;

use std::fmt;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use bytes::Bytes;
use fogu_core::{EffectiveProfile, EffectiveSsh, Size};
use fogu_term::{ClipboardType, PromptMark, TermHandle};
use tokio::runtime::{Handle, Runtime};
use tokio::sync::mpsc::{self, Receiver, Sender};
use tracing::Instrument as _;

pub use env::build_env;
pub use prompt::{
    KbdPrompt, PROMPT_INSTRUCTIONS_MAX_CHARS, PROMPT_MAX_COUNT, PROMPT_TEXT_MAX_CHARS, PromptId,
    PromptKind, PromptReply, sanitize_remote,
};
pub use sftp::{SftpCmd, SftpEvent, SftpHandle, SftpRequest, display_name};
#[cfg(feature = "ssh")]
pub use ssh::{PROMPT_TIMEOUT, SshPaths, validate_name};
pub use supervise::supervise;

/// Capacity of the UI → session input channel.
const INPUT_CAPACITY: usize = 64;
/// Capacity of the session → UI event channel.
const OUTPUT_CAPACITY: usize = 256;

/// Owns the tokio runtime that runs every session task (2 worker threads).
#[derive(Debug)]
pub struct SessionRuntime {
    rt: Runtime,
}

impl SessionRuntime {
    /// Builds the multi-thread runtime.
    pub fn new() -> io::Result<Self> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("fogu-session")
            .enable_all()
            .build()?;
        Ok(Self { rt })
    }

    /// Handle to pass to [`open`].
    pub fn handle(&self) -> &Handle {
        self.rt.handle()
    }
}

/// The UI's side of one session.
#[derive(Debug)]
pub struct SessionHandle {
    /// Keys, paste chunks, resizes, prompt replies, close.
    pub input: Sender<SessionInput>,
    /// Everything the session reports back; merged into one iced subscription.
    pub events: Receiver<SessionOutput>,
    /// SSH sessions only: the SFTP engine on the same connection (idle until the first
    /// command; commands sent before the shell is up wait).
    pub sftp: Option<SftpHandle>,
}

/// UI → session messages.
#[derive(Debug)]
pub enum SessionInput {
    /// Already-encoded keys or paste chunks.
    Data(Bytes),
    /// New size; the UI resizes the `TermHandle` itself first.
    Resize(Size),
    /// Answer to a [`SessionOutput::Prompt`].
    PromptReply(PromptId, PromptReply),
    /// Ask for the foreground process's current directory (on demand only, for opening a new tab in the same directory);
    /// the session replies with [`SessionOutput::Cwd`] if it could determine one.
    QueryCwd,
    /// Close the transport; the session answers with `Exited`.
    Close,
}

/// Session → UI messages.
#[derive(Clone, Debug)]
pub enum SessionOutput {
    /// The terminal changed; sent only when `dirty` goes false → true.
    Wakeup,
    /// Sanitized window title (empty = reset to default).
    Title(String),
    /// Terminal bell.
    Bell,
    /// Current working directory (OSC 7/1337, or an on-demand [`SessionInput::QueryCwd`] reply).
    Cwd {
        /// The reported directory.
        path: PathBuf,
        /// `true` when the reporting host wasn't the local machine (OSC 7 with a foreign host);
        /// such a path cannot be validated or opened locally.
        remote: bool,
    },
    /// Progress (OSC 9;4).
    Progress(Option<u8>),
    /// Desktop notification (OSC 9 / 777); no title for OSC 9.
    Notify {
        /// Present only for OSC 777.
        title: Option<String>,
        /// Sanitized, capped body text.
        body: String,
    },
    /// Shell-integration prompt mark (OSC 133); not used by the UI yet.
    PromptMark(PromptMark),
    /// User input was dropped: the program stopped reading and the backlog is full. Sent once
    /// per backlog.
    InputDropped,
    /// The session needs an answer from the user (SSH): reply with
    /// [`SessionInput::PromptReply`] carrying the same id within [`PROMPT_TIMEOUT`].
    Prompt(PromptId, PromptKind),
    /// Something the user should know about this session (warning badge / toast).
    Warning(SessionWarning),
    /// OSC 52 clipboard write; the UI applies the policy.
    ClipboardStore(ClipboardRequest),
    /// The transport ended.
    Exited(ExitInfo),
    /// The session failed.
    Error(SessionError),
}

/// Non-fatal conditions a session reports (closed set).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionWarning {
    /// `verify_host_keys = false`: the server's identity was not checked (persistent badge).
    HostKeyNotVerified,
    /// Profile options that are set but not implemented yet (accepted, ignored).
    Unsupported(Vec<&'static str>),
}

/// Why a server's host key was not accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum HostKeyFailure {
    /// The user declined an unknown key (or the prompt was abandoned).
    #[error("host key not accepted")]
    Rejected,
    /// The key differs from the recorded one and was not explicitly replaced.
    #[error("host key changed (possible man-in-the-middle attack); connection refused")]
    Changed,
    /// The key is marked `@revoked` in a `known_hosts` file.
    #[error("host key is revoked")]
    Revoked,
    /// The server presented a certificate; certificates are not supported yet.
    #[error("host certificates are not supported")]
    Certificate,
}

/// OSC 52 write request forwarded to the UI. `Debug` shows only the target and the length: the
/// payload is attacker-chosen text that must never reach a log.
#[derive(Clone)]
pub struct ClipboardRequest {
    /// Clipboard or primary selection.
    pub target: ClipboardType,
    /// Text the remote wants to store.
    pub payload: String,
    /// Payload length in bytes.
    pub len: usize,
}

impl fmt::Debug for ClipboardRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClipboardRequest")
            .field("target", &self.target)
            .field("len", &self.len)
            .finish_non_exhaustive()
    }
}

/// How the transport ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExitInfo {
    /// Exit code, if the process exited normally.
    pub code: Option<i32>,
    /// Terminating signal, if it was killed by one.
    pub signal: Option<i32>,
}

/// Session failures reported to the UI. Never carries secrets.
#[derive(Clone, Debug, thiserror::Error)]
pub enum SessionError {
    /// The local program could not be started.
    #[error("failed to start {program}")]
    Spawn {
        /// Program path or name.
        program: String,
        /// Underlying OS error.
        #[source]
        source: Arc<io::Error>,
    },
    /// I/O failure on a running session.
    #[error("session I/O error")]
    Io(#[source] Arc<io::Error>),
    /// The session task panicked; the pane's `TermHandle` must not be reused.
    #[error("session task crashed")]
    Crashed,
    /// The SSH profile has no host.
    #[error("no host configured")]
    NoHost,
    /// TCP connection failed.
    #[error("could not connect to {target}")]
    Connect {
        /// `host:port` as configured (sanitized).
        target: String,
        /// Underlying OS error.
        #[source]
        source: Arc<io::Error>,
    },
    /// Connect, handshake or an authentication request exceeded `ready_timeout`.
    #[error("connection timed out")]
    Timeout,
    /// No answer to a prompt within [`PROMPT_TIMEOUT`].
    #[error("no answer to the prompt in time")]
    PromptTimeout,
    /// The server's host key was not accepted.
    #[error(transparent)]
    HostKey(HostKeyFailure),
    /// Every allowed authentication method failed.
    #[error("authentication failed")]
    AuthFailed,
    /// Any other SSH failure; the text is sanitized and capped (it may quote the server).
    #[error("SSH error: {0}")]
    Ssh(String),
}

/// Starts a session task on `rt` for `profile` that feeds `term` and returns the UI's side of it.
/// SSH profiles use the default [`SshPaths`] (our `known_hosts` under the default config dir);
/// see [`open_ssh`] to choose them.
pub fn open(
    rt: &Handle,
    profile: &EffectiveProfile,
    size: Size,
    term: TermHandle,
) -> SessionHandle {
    match profile {
        EffectiveProfile::Local(local) => spawn(rt, |in_rx, out_tx| {
            let span = tracing::info_span!("session", kind = "local");
            rt.spawn(local::run_local(local.clone(), size, term, in_rx, out_tx).instrument(span))
        }),
        #[cfg(feature = "ssh")]
        EffectiveProfile::Ssh(ssh) => open_ssh(rt, ssh, SshPaths::default_for_user(), size, term),
        #[cfg(not(feature = "ssh"))]
        EffectiveProfile::Ssh(ssh) => unsupported_ssh(rt, ssh),
    }
}

/// [`open`] for an SSH profile with explicit `known_hosts` locations (`--config-dir`, tests).
#[cfg(feature = "ssh")]
pub fn open_ssh(
    rt: &Handle,
    ssh: &EffectiveSsh,
    paths: SshPaths,
    size: Size,
    term: TermHandle,
) -> SessionHandle {
    let (sftp, sftp_ends) = ssh::sftp::channels();
    let mut handle = spawn(rt, |in_rx, out_tx| {
        let span = tracing::info_span!("session", kind = "ssh");
        let task = ssh::run_ssh(ssh.clone(), paths, size, term, in_rx, out_tx, sftp_ends);
        rt.spawn(task.instrument(span))
    });
    handle.sftp = Some(sftp);
    handle
}

/// Without the `ssh` feature an SSH profile fails at once with [`SessionError::Spawn`]
/// (`ErrorKind::Unsupported`), so the pane shows the crashed state.
#[cfg(not(feature = "ssh"))]
fn unsupported_ssh(rt: &Handle, ssh: &EffectiveSsh) -> SessionHandle {
    let program = format!("ssh {}", ssh.host);
    spawn(rt, |in_rx, out| {
        rt.spawn(async move {
            let source = Arc::new(io::Error::from(io::ErrorKind::Unsupported));
            drop(in_rx);
            let _gone = out
                .send(SessionOutput::Error(SessionError::Spawn {
                    program,
                    source,
                }))
                .await;
        })
    })
}

/// Creates the channels, starts the task built by `start` and its supervisor.
fn spawn(
    rt: &Handle,
    start: impl FnOnce(Receiver<SessionInput>, Sender<SessionOutput>) -> tokio::task::JoinHandle<()>,
) -> SessionHandle {
    let (in_tx, in_rx) = mpsc::channel(INPUT_CAPACITY);
    let (out_tx, out_rx) = mpsc::channel(OUTPUT_CAPACITY);
    let task = start(in_rx, out_tx.clone());
    rt.spawn(supervise(task, out_tx));
    SessionHandle {
        input: in_tx,
        events: out_rx,
        sftp: None,
    }
}
