//! The SFTP engine: one task per SSH connection owning the SFTP channel, driven by
//! [`SftpRequest`]s and answering with [`SftpEvent`]s. The channel is opened lazily on the tab's
//! existing russh `Handle` (no second login), reopened after it dies, and closed on `Shutdown`
//! once no transfer runs. Every command runs as its own task so a slow listing never delays a
//! `Cancel`.
//!
//! Files: `remote.rs` (channel, bounded listing), `remote_io.rs` (pipelined reads/writes),
//! `ops.rs` (metadata commands), `transfer.rs` (shared transfer plumbing), `download.rs`,
//! `upload.rs`, `local_fs.rs` (symlink-safe local writes), `disk_guard.rs` (download size cap +
//! free-space reserve).

mod disk_guard;
mod download;
#[cfg(test)]
mod hostile_server;
#[cfg(test)]
mod hostile_tests;
mod local_fs;
mod ops;
mod remote;
mod remote_io;
mod transfer;
mod upload;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use russh::client;
use tokio::sync::Semaphore;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::task::{Id, JoinSet};
use tokio_util::sync::CancellationToken;

pub(crate) use disk_guard::Statvfs;

use self::disk_guard::SpaceProbe;
use self::remote::{Remote, RemoteError};
use self::transfer::Ctx;
use super::connect::Client;
use crate::sftp::{CmdId, SftpCmd, SftpEvent, SftpHandle, SftpOptions, SftpRequest};

/// Capacity of the UI → engine command channel.
const SFTP_CMD_CAPACITY: usize = 64;
/// Capacity of the engine → UI event channel.
const SFTP_EVENT_CAPACITY: usize = 256;

/// How long the engine waits for its transfers to clean up when the connection ends.
pub(crate) const STOP_GRACE: Duration = Duration::from_secs(3);

/// The engine's side of the channel pair, held by `run_ssh` until the shell is up.
pub(crate) struct SftpEnds {
    cmds: Receiver<SftpRequest>,
    events: Sender<SftpEvent>,
}

/// Creates the UI handle and the engine ends.
pub(crate) fn channels() -> (SftpHandle, SftpEnds) {
    let (cmd_tx, cmd_rx) = mpsc::channel(SFTP_CMD_CAPACITY);
    let (ev_tx, ev_rx) = mpsc::channel(SFTP_EVENT_CAPACITY);
    let handle = SftpHandle {
        cmds: cmd_tx,
        events: ev_rx,
    };
    (
        handle,
        SftpEnds {
            cmds: cmd_rx,
            events: ev_tx,
        },
    )
}

/// What a finished task was.
enum TaskKind {
    Op,
    Transfer(CmdId),
}

struct Engine {
    ssh: Arc<client::Handle<Client>>,
    events: Sender<SftpEvent>,
    opts: SftpOptions,
    slots: Arc<Semaphore>,
    remote: Option<Arc<Remote>>,
    tasks: JoinSet<()>,
    kinds: HashMap<Id, TaskKind>,
    transfers: HashMap<CmdId, CancellationToken>,
    /// Root of every transfer token: cancelled when the connection ends.
    all: CancellationToken,
    shutdown_pending: bool,
    /// Local free-space probe for downloads.
    space: SpaceProbe,
}

/// The engine task. Ends when the UI drops its handle or `stop` fires (the shell ended): running
/// transfers are cancelled (partial files cleaned up) and `Closed` is sent. `space` probes local
/// free space for the downloads' disk-fill guard ([`Statvfs`] in production).
pub(crate) async fn run(
    ssh: Arc<client::Handle<Client>>,
    ends: SftpEnds,
    stop: CancellationToken,
    space: SpaceProbe,
) {
    let SftpEnds { mut cmds, events } = ends;
    let opts = SftpOptions::default();
    let mut engine = Engine {
        ssh,
        events,
        slots: Arc::new(Semaphore::new(opts.max_parallel_files.max(1))),
        opts,
        remote: None,
        tasks: JoinSet::new(),
        kinds: HashMap::new(),
        transfers: HashMap::new(),
        all: CancellationToken::new(),
        shutdown_pending: false,
        space,
    };
    loop {
        tokio::select! {
            biased;
            () = stop.cancelled() => break,
            Some(done) = engine.tasks.join_next_with_id(), if !engine.tasks.is_empty() => {
                engine.task_ended(done.map_or_else(|e| e.id(), |(id, ())| id)).await;
            }
            request = cmds.recv() => match request {
                Some(request) => engine.handle(request).await,
                None => break,
            },
        }
    }
    engine.stop().await;
}

impl Engine {
    async fn emit(&self, event: SftpEvent) {
        let _gone = self.events.send(event).await;
    }

    async fn handle(&mut self, SftpRequest { id, cmd }: SftpRequest) {
        match cmd {
            SftpCmd::Cancel(transfer) => {
                if let Some(token) = self.transfers.get(&transfer) {
                    token.cancel();
                }
                self.emit(SftpEvent::Done(id)).await;
            }
            SftpCmd::SetOptions(opts) => {
                self.set_options(opts);
                self.emit(SftpEvent::Done(id)).await;
            }
            SftpCmd::Shutdown => {
                self.shutdown_pending = true;
                self.maybe_close().await;
            }
            cmd => {
                self.shutdown_pending = false;
                match self.remote().await {
                    Ok(remote) => self.spawn(id, cmd, remote),
                    Err(err) => {
                        self.emit(SftpEvent::Error {
                            id,
                            message: err.message,
                        })
                        .await;
                    }
                }
            }
        }
    }

    fn set_options(&mut self, mut opts: SftpOptions) {
        opts.max_parallel_files = opts.max_parallel_files.clamp(1, 64);
        if opts.max_parallel_files != self.opts.max_parallel_files {
            // Running transfers keep their slots; new ones share the new limit.
            self.slots = Arc::new(Semaphore::new(opts.max_parallel_files));
        }
        if let Some(remote) = &self.remote {
            remote.set_timeout(opts.request_timeout);
        }
        self.opts = opts;
    }

    /// The open channel, opening (or reopening) it when needed.
    async fn remote(&mut self) -> Result<Arc<Remote>, RemoteError> {
        if let Some(remote) = &self.remote {
            if remote.is_alive() {
                return Ok(Arc::clone(remote));
            }
            self.remote = None;
            self.emit(SftpEvent::Closed).await;
        }
        let open = Remote::open(&self.ssh, &self.opts);
        let remote = tokio::time::timeout(self.opts.request_timeout, open)
            .await
            .map_err(|_| RemoteError::new("opening the SFTP channel timed out"))??;
        let remote = Arc::new(remote);
        self.remote = Some(Arc::clone(&remote));
        Ok(remote)
    }

    fn spawn(&mut self, id: CmdId, cmd: SftpCmd, remote: Arc<Remote>) {
        let events = self.events.clone();
        let (kind, handle) = match cmd {
            SftpCmd::Download {
                remote: paths,
                local_dir,
                overwrite,
                resume,
            } => {
                let ctx = self.transfer_ctx(id, remote);
                let task = download::download(ctx, paths, local_dir, overwrite, resume);
                (TaskKind::Transfer(id), self.tasks.spawn(task))
            }
            SftpCmd::Upload {
                local,
                remote_dir,
                overwrite,
                resume,
            } => {
                let ctx = self.transfer_ctx(id, remote);
                let task = upload::upload(ctx, local, remote_dir, (overwrite, resume));
                (TaskKind::Transfer(id), self.tasks.spawn(task))
            }
            cmd => {
                let task = ops::run(remote, self.opts.clone(), id, cmd, events);
                (TaskKind::Op, self.tasks.spawn(task))
            }
        };
        self.kinds.insert(handle.id(), kind);
    }

    fn transfer_ctx(&mut self, id: CmdId, remote: Arc<Remote>) -> Ctx {
        let cancel = self.all.child_token();
        self.transfers.insert(id, cancel.clone());
        Ctx {
            id,
            remote,
            events: self.events.clone(),
            cancel,
            slots: Arc::clone(&self.slots),
            opts: self.opts.clone(),
            space: Arc::clone(&self.space),
        }
    }

    async fn task_ended(&mut self, task: Id) {
        match self.kinds.remove(&task) {
            Some(TaskKind::Transfer(id)) => {
                self.transfers.remove(&id);
            }
            Some(TaskKind::Op) | None => {}
        }
        self.maybe_close().await;
    }

    /// After `Shutdown`: closes the channel once no transfer runs.
    async fn maybe_close(&mut self) {
        if !self.shutdown_pending || !self.transfers.is_empty() {
            return;
        }
        self.shutdown_pending = false;
        if let Some(remote) = self.remote.take() {
            remote.close();
        }
        self.emit(SftpEvent::Closed).await;
    }

    /// The connection is ending: cancel everything, give transfers a moment to clean up.
    async fn stop(mut self) {
        self.all.cancel();
        let drain = async { while self.tasks.join_next().await.is_some() {} };
        if tokio::time::timeout(STOP_GRACE, drain).await.is_err() {
            self.tasks.abort_all();
        }
        if let Some(remote) = self.remote.take() {
            remote.close();
        }
        // Never wait for a UI that stopped reading: the connection is closing.
        let _full = self.events.try_send(SftpEvent::Closed);
    }
}
