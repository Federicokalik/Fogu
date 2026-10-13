//! `fogu-session`: the one session API: `open()` spawns a transport task per tab that feeds a
//! [`TermHandle`] and talks to the UI over two bounded channels. Only the local-shell task (`run_local`) exists so far.

mod env;
mod local;
mod supervise;

use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use bytes::Bytes;
use fogu_core::{EffectiveProfile, Size};
use fogu_term::{ClipboardType, PromptMark, TermHandle};
use tokio::runtime::{Handle, Runtime};
use tokio::sync::mpsc::{self, Receiver, Sender};
use tracing::Instrument as _;

pub use env::build_env;
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
    /// The session needs an answer from the user (SSH).
    Prompt(PromptId, PromptKind),
    /// OSC 52 clipboard write; the UI applies the policy.
    ClipboardStore(ClipboardRequest),
    /// The transport ended.
    Exited(ExitInfo),
    /// The session failed.
    Error(SessionError),
}

/// Identifies one prompt within a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PromptId(pub u64);

/// Prompt kinds (host key, password, keyboard-interactive). No kinds yet.
#[derive(Clone, Debug)]
pub enum PromptKind {}

/// Replies to prompts. No replies yet.
#[derive(Debug)]
pub enum PromptReply {}

/// OSC 52 write request forwarded to the UI.
#[derive(Clone, Debug)]
pub struct ClipboardRequest {
    /// Clipboard or primary selection.
    pub target: ClipboardType,
    /// Text the remote wants to store.
    pub payload: String,
    /// Payload length in bytes.
    pub len: usize,
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
}

/// Starts a session task on `rt` for `profile` that feeds `term` and returns the UI's side of it.
/// SSH profiles have no transport yet: their session fails at once with
/// [`SessionError::Spawn`] (`ErrorKind::Unsupported`), so the pane shows the crashed state.
pub fn open(
    rt: &Handle,
    profile: &EffectiveProfile,
    size: Size,
    term: TermHandle,
) -> SessionHandle {
    let (in_tx, in_rx) = mpsc::channel(INPUT_CAPACITY);
    let (out_tx, out_rx) = mpsc::channel(OUTPUT_CAPACITY);
    let task = match profile {
        EffectiveProfile::Local(local) => {
            let span = tracing::info_span!("session", kind = "local");
            rt.spawn(
                local::run_local(local.clone(), size, term, in_rx, out_tx.clone()).instrument(span),
            )
        }
        EffectiveProfile::Ssh(ssh) => {
            let program = format!("ssh {}", ssh.host);
            let out = out_tx.clone();
            rt.spawn(async move {
                let source = Arc::new(io::Error::from(io::ErrorKind::Unsupported));
                let err = SessionError::Spawn { program, source };
                drop(in_rx);
                if out.send(SessionOutput::Error(err)).await.is_err() {
                    tracing::debug!("SSH stub error not reported: UI receiver dropped");
                }
            })
        }
    };
    rt.spawn(supervise(task, out_tx));
    SessionHandle {
        input: in_tx,
        events: out_rx,
    }
}
