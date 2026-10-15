//! Pipelined SFTP file I/O and the liveness-tracking stream wrapper. `Reader`/`Writer` keep up
//! to [`MAX_IN_FLIGHT`] requests outstanding (each its own small task holding the shared
//! session), so a file's memory is bounded by that many chunks; dropping them aborts the rest.

use std::collections::VecDeque;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};

use russh_sftp::client::RawSftpSession;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::StatusCode;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::task::JoinHandle;

use super::remote::{RemoteError, RemoteResult};

/// Requests in flight per file (pipelined reads/writes hide the round-trip latency).
pub(crate) const MAX_IN_FLIGHT: usize = 16;

type ReadReply = Result<russh_sftp::protocol::Data, SftpError>;

/// Keeps up to [`MAX_IN_FLIGHT`] read requests outstanding. Memory: at most that many chunks.
pub(crate) struct Reader {
    raw: Arc<RawSftpSession>,
    handle: String,
    chunk: u32,
    next_offset: u64,
    in_flight: VecDeque<(u64, JoinHandle<ReadReply>)>,
    eof: bool,
}

impl Reader {
    pub(super) fn new(raw: Arc<RawSftpSession>, handle: &str, chunk: u32, offset: u64) -> Self {
        Self {
            raw,
            handle: handle.to_owned(),
            chunk: chunk.max(1),
            next_offset: offset,
            in_flight: VecDeque::new(),
            eof: false,
        }
    }

    /// The next chunk in file order, or `None` at end of file.
    pub(crate) async fn next(&mut self) -> RemoteResult<Option<Vec<u8>>> {
        loop {
            if self.eof {
                return Ok(None);
            }
            while self.in_flight.len() < MAX_IN_FLIGHT {
                let (raw, handle, offset, len) = (
                    Arc::clone(&self.raw),
                    self.handle.clone(),
                    self.next_offset,
                    self.chunk,
                );
                let task = tokio::spawn(async move { raw.read(handle, offset, len).await });
                self.in_flight.push_back((offset, task));
                self.next_offset = offset.saturating_add(u64::from(self.chunk));
            }
            let Some((offset, task)) = self.in_flight.pop_front() else {
                return Ok(None);
            };
            let reply = task
                .await
                .map_err(|_| RemoteError::new("SFTP read task failed"))?;
            match reply {
                Ok(data) if data.data.len() > self.chunk as usize => {
                    return Err(RemoteError::new("the server sent more data than requested"));
                }
                Ok(data) if data.data.is_empty() => self.stop(),
                Ok(data) => {
                    if data.data.len() < self.chunk as usize {
                        // Short read: the server serves smaller chunks (or this is the end).
                        // Drop the requests behind it, continue from the gap, ask for that size.
                        self.abort_all();
                        self.next_offset = offset + data.data.len() as u64;
                        self.chunk = u32::try_from(data.data.len()).unwrap_or(self.chunk);
                    }
                    return Ok(Some(data.data));
                }
                Err(SftpError::Status(s)) if s.status_code == StatusCode::Eof => self.stop(),
                Err(err) => return Err(err.into()),
            }
        }
    }

    fn stop(&mut self) {
        self.eof = true;
        self.abort_all();
    }

    fn abort_all(&mut self) {
        for (_, task) in self.in_flight.drain(..) {
            task.abort();
        }
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        self.abort_all();
    }
}

type WriteReply = Result<russh_sftp::protocol::Status, SftpError>;

/// Keeps up to [`MAX_IN_FLIGHT`] write requests outstanding.
pub(crate) struct Writer {
    raw: Arc<RawSftpSession>,
    handle: String,
    offset: u64,
    in_flight: VecDeque<JoinHandle<WriteReply>>,
}

impl Writer {
    pub(super) fn new(raw: Arc<RawSftpSession>, handle: &str, offset: u64) -> Self {
        Self {
            raw,
            handle: handle.to_owned(),
            offset,
            in_flight: VecDeque::new(),
        }
    }

    /// Queues `data` (one request, ≤ the write chunk) at the current offset.
    pub(crate) async fn write(&mut self, data: Vec<u8>) -> RemoteResult<()> {
        if self.in_flight.len() >= MAX_IN_FLIGHT {
            self.wait_oldest().await?;
        }
        let (raw, handle, offset) = (Arc::clone(&self.raw), self.handle.clone(), self.offset);
        self.offset += data.len() as u64;
        self.in_flight.push_back(tokio::spawn(async move {
            raw.write(handle, offset, data).await
        }));
        Ok(())
    }

    /// Waits for every outstanding write.
    pub(crate) async fn finish(&mut self) -> RemoteResult<()> {
        while !self.in_flight.is_empty() {
            self.wait_oldest().await?;
        }
        Ok(())
    }

    async fn wait_oldest(&mut self) -> RemoteResult<()> {
        if let Some(task) = self.in_flight.pop_front() {
            task.await
                .map_err(|_| RemoteError::new("SFTP write task failed"))??;
        }
        Ok(())
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        for task in self.in_flight.drain(..) {
            task.abort();
        }
    }
}

/// The channel stream, flagging EOF/errors so the engine knows to reopen.
pub(super) struct Tracked<S> {
    inner: S,
    alive: Arc<AtomicBool>,
}

impl<S> Tracked<S> {
    pub(super) fn new(inner: S, alive: Arc<AtomicBool>) -> Self {
        Self { inner, alive }
    }

    fn note<T>(&self, poll: &Poll<io::Result<T>>, eof: bool) {
        if eof || matches!(poll, Poll::Ready(Err(_))) {
            self.alive.store(false, Ordering::Relaxed);
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Tracked<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let poll = Pin::new(&mut self.inner).poll_read(cx, buf);
        let eof = matches!(poll, Poll::Ready(Ok(()))) && buf.filled().len() == before;
        self.note(&poll, eof);
        poll
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Tracked<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let poll = Pin::new(&mut self.inner).poll_write(cx, buf);
        self.note(&poll, false);
        poll
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let poll = Pin::new(&mut self.inner).poll_flush(cx);
        self.note(&poll, false);
        poll
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let poll = Pin::new(&mut self.inner).poll_shutdown(cx);
        self.alive.store(false, Ordering::Relaxed);
        poll
    }
}
