//! The SFTP channel: opened on the tab's existing russh `Handle` (`channel_open_session` +
//! `subsystem sftp`), driven through russh-sftp's `RawSftpSession` so that every server reply is
//! bounded by us: listings stop at a cap (the crate's `read_dir` collects without limit), reads
//! are pipelined with at most `MAX_IN_FLIGHT` requests of at most one chunk each, and a data
//! reply longer than requested is a protocol error (`remote_io.rs`). Server texts are sanitized
//! before they leave this module.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use russh::ChannelMsg;
use russh::client;
use russh_sftp::client::RawSftpSession;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::client::rawsession::Limits;
use russh_sftp::protocol::{FileAttributes, OpenFlags, StatusCode};

use super::remote_io::{Reader, Tracked, Writer};
use crate::sanitize_remote;
use crate::sftp::{EntryKind, SftpOptions};
use crate::ssh::connect::Client;

/// Chunk size when the server has no `limits@openssh.com`.
pub(crate) const FALLBACK_CHUNK: u32 = 32 * 1024;
/// Largest SFTP packet we accept (russh-sftp's default; also bounds every reply in memory).
const MAX_PACKET: u32 = 256 * 1024;
/// `SSH_FXP_DATA` header: length(4) + type(1) + id(4) + data length(4).
const READ_OVERHEAD: u32 = 13;
/// `SSH_FXP_WRITE` header without the handle: length(4) + type(1) + id(4) + handle length(4) +
/// offset(8) + data length(4); handles are ≤ 256 bytes (SFTP v3).
const WRITE_OVERHEAD: u32 = 25 + 256;
/// Longest entry name accepted from a listing (Linux `PATH_MAX`); longer ones are dropped.
const NAME_BYTES_MAX: usize = 4096;
/// Name bytes per listing; beyond it the listing is truncated like at the entry cap.
const LISTING_BYTES_MAX: usize = 32 * 1024 * 1024;
/// Longest server error text kept.
const ERROR_MAX_CHARS: usize = 300;

/// Sanitized error text of a failed SFTP operation.
pub(crate) type RemoteResult<T> = Result<T, RemoteError>;

/// A failed SFTP operation: whether the path was missing, plus sanitized text.
#[derive(Clone, Debug)]
pub(crate) struct RemoteError {
    pub(crate) not_found: bool,
    pub(crate) message: String,
}

impl RemoteError {
    pub(crate) fn new(message: impl AsRef<str>) -> Self {
        Self {
            not_found: false,
            message: sanitize_remote(message.as_ref(), ERROR_MAX_CHARS, false),
        }
    }
}

impl From<SftpError> for RemoteError {
    fn from(err: SftpError) -> Self {
        let not_found =
            matches!(&err, SftpError::Status(s) if s.status_code == StatusCode::NoSuchFile);
        let message = match &err {
            SftpError::Status(s) if s.error_message.is_empty() => s.status_code.to_string(),
            SftpError::Status(s) => format!("{}: {}", s.status_code, s.error_message),
            SftpError::Timeout => "the server did not answer in time".to_owned(),
            other => other.to_string(),
        };
        Self {
            not_found,
            message: sanitize_remote(&message, ERROR_MAX_CHARS, false),
        }
    }
}

impl From<io::Error> for RemoteError {
    fn from(err: io::Error) -> Self {
        Self::new(err.to_string())
    }
}

/// Entry type from SFTP mode bits.
pub(crate) fn kind_of(attrs: &FileAttributes) -> EntryKind {
    match attrs.permissions.map(|m| m & 0o170_000) {
        Some(0o100_000) => EntryKind::File,
        Some(0o040_000) => EntryKind::Dir,
        Some(0o120_000) => EntryKind::Symlink,
        _ => EntryKind::Other,
    }
}

/// One open SFTP channel.
pub(crate) struct Remote {
    raw: Arc<RawSftpSession>,
    alive: Arc<AtomicBool>,
    /// Bytes per read request.
    pub(crate) read_chunk: u32,
    /// Bytes per write request.
    pub(crate) write_chunk: usize,
}

impl Remote {
    /// Opens the `sftp` subsystem on `handle` and negotiates version 3 + server limits.
    pub(crate) async fn open(
        handle: &client::Handle<Client>,
        opts: &SftpOptions,
    ) -> RemoteResult<Self> {
        let channel = handle
            .channel_open_session()
            .await
            .map_err(|e| RemoteError::new(e.to_string()))?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|e| RemoteError::new(e.to_string()))?;
        let mut channel = channel;
        loop {
            match channel.wait().await {
                Some(ChannelMsg::Success) => break,
                Some(ChannelMsg::Failure) => {
                    return Err(RemoteError::new("the server refused the SFTP subsystem"));
                }
                Some(ChannelMsg::Close | ChannelMsg::Eof) | None => {
                    return Err(RemoteError::new("the SFTP channel closed"));
                }
                Some(_) => {}
            }
        }
        Self::over(channel.into_stream(), opts).await
    }

    /// Runs the SFTP client protocol over `stream` (the subsystem channel; an in-memory pipe in
    /// tests) and negotiates version 3 + server limits.
    pub(crate) async fn over<S>(stream: S, opts: &SftpOptions) -> RemoteResult<Self>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        let alive = Arc::new(AtomicBool::new(true));
        let stream = Tracked::new(stream, Arc::clone(&alive));
        let config = russh_sftp::client::Config {
            max_packet_len: MAX_PACKET,
            request_timeout_secs: opts.request_timeout.as_secs().max(1),
            ..russh_sftp::client::Config::default()
        };
        let mut raw = RawSftpSession::new_with_config(stream, config);
        let version = raw.init().await?;
        if version.version != 3 {
            return Err(RemoteError::new(format!(
                "unsupported SFTP version {}",
                version.version
            )));
        }
        let has_limits = version
            .extensions
            .get(russh_sftp::extensions::LIMITS)
            .is_some_and(|v| v == "1");
        let limits = if has_limits {
            Some(Limits::from(raw.limits().await?))
        } else {
            None
        };
        let (read_chunk, write_chunk) = chunk_sizes(limits.as_ref());
        if let Some(limits) = limits {
            raw.set_limits(limits);
        }
        tracing::debug!(read_chunk, write_chunk, "SFTP channel open");
        Ok(Self {
            raw: Arc::new(raw),
            alive,
            read_chunk,
            write_chunk,
        })
    }

    /// `false` once the channel hit EOF or an error (the server's SFTP process or the
    /// connection went away).
    pub(crate) fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    /// New request timeout for later requests.
    pub(crate) fn set_timeout(&self, timeout: Duration) {
        self.raw.set_timeout(timeout.as_secs().max(1));
    }

    /// Closes the channel (best effort).
    pub(crate) fn close(&self) {
        let _gone = self.raw.close_session();
        self.alive.store(false, Ordering::Relaxed);
    }

    pub(crate) async fn lstat(&self, path: &str) -> RemoteResult<FileAttributes> {
        Ok(self.raw.lstat(path).await?.attrs)
    }

    pub(crate) async fn stat(&self, path: &str) -> RemoteResult<FileAttributes> {
        Ok(self.raw.stat(path).await?.attrs)
    }

    /// `lstat`, with `None` when the path does not exist.
    pub(crate) async fn try_lstat(&self, path: &str) -> RemoteResult<Option<FileAttributes>> {
        match self.lstat(path).await {
            Ok(attrs) => Ok(Some(attrs)),
            Err(err) if err.not_found => Ok(None),
            Err(err) => Err(err),
        }
    }

    pub(crate) async fn read_link(&self, path: &str) -> RemoteResult<String> {
        let name = self.raw.readlink(path).await?;
        first_name(name.files)
    }

    pub(crate) async fn realpath(&self, path: &str) -> RemoteResult<String> {
        let name = self.raw.realpath(path).await?;
        first_name(name.files)
    }

    pub(crate) async fn mkdir(&self, path: &str) -> RemoteResult<()> {
        self.raw.mkdir(path, FileAttributes::empty()).await?;
        Ok(())
    }

    pub(crate) async fn rename(&self, from: &str, to: &str) -> RemoteResult<()> {
        self.raw.rename(from, to).await?;
        Ok(())
    }

    pub(crate) async fn remove_file(&self, path: &str) -> RemoteResult<()> {
        self.raw.remove(path).await?;
        Ok(())
    }

    pub(crate) async fn remove_dir(&self, path: &str) -> RemoteResult<()> {
        self.raw.rmdir(path).await?;
        Ok(())
    }

    pub(crate) async fn chmod(&self, path: &str, mode: u32) -> RemoteResult<()> {
        let attrs = FileAttributes {
            permissions: Some(mode & 0o7777),
            ..FileAttributes::empty()
        };
        self.raw.setstat(path, attrs).await?;
        Ok(())
    }

    /// Reads directory `path`: at most `cap` entries (`.`/`..` and unusable names excluded, see
    /// [`usable_name`]); `true` = truncated (entry cap or [`LISTING_BYTES_MAX`] name bytes).
    /// An empty reply ends the listing like EOF (a server sending empty replies forever cannot
    /// keep us looping).
    pub(crate) async fn read_dir(
        &self,
        path: &str,
        cap: usize,
    ) -> RemoteResult<(Vec<(String, FileAttributes)>, bool)> {
        let handle = self.raw.opendir(path).await?.handle;
        let mut entries = Vec::new();
        let mut bytes = 0_usize;
        let mut truncated = false;
        let result = loop {
            match self.raw.readdir(handle.as_str()).await {
                Ok(name) if name.files.is_empty() => break Ok(()),
                Ok(name) => {
                    let names = name.files.into_iter().filter(|f| usable_name(&f.filename));
                    for file in names {
                        bytes += file.filename.len();
                        if entries.len() == cap || bytes > LISTING_BYTES_MAX {
                            truncated = true;
                            break;
                        }
                        entries.push((file.filename, file.attrs));
                    }
                    if truncated {
                        break Ok(());
                    }
                }
                Err(SftpError::Status(s)) if s.status_code == StatusCode::Eof => break Ok(()),
                Err(err) => break Err(RemoteError::from(err)),
            }
        };
        let _closed = self.raw.close(handle).await;
        result.map(|()| (entries, truncated))
    }

    /// Opens `path` for reading; returns the handle.
    pub(crate) async fn open_read(&self, path: &str) -> RemoteResult<String> {
        let handle = self
            .raw
            .open(path, OpenFlags::READ, FileAttributes::empty())
            .await?;
        Ok(handle.handle)
    }

    /// Opens `path` for writing: created with `mode` (`0o777` bits only); `truncate` replaces an
    /// existing file, otherwise an existing file is an error (`EXCLUDE`).
    pub(crate) async fn open_write(
        &self,
        path: &str,
        mode: u32,
        truncate: bool,
    ) -> RemoteResult<String> {
        let mut flags = OpenFlags::WRITE | OpenFlags::CREATE;
        flags |= if truncate {
            OpenFlags::TRUNCATE
        } else {
            OpenFlags::EXCLUDE
        };
        let attrs = FileAttributes {
            permissions: Some(mode & 0o777),
            ..FileAttributes::empty()
        };
        Ok(self.raw.open(path, flags, attrs).await?.handle)
    }

    /// Closes a file handle, reporting the server's verdict (a failed close can mean lost
    /// writes).
    pub(crate) async fn close_handle(&self, handle: String) -> RemoteResult<()> {
        self.raw.close(handle).await?;
        Ok(())
    }

    /// Pipelined reader of `handle` from `offset`.
    pub(crate) fn reader(&self, handle: &str, offset: u64) -> Reader {
        Reader::new(Arc::clone(&self.raw), handle, self.read_chunk, offset)
    }

    /// Pipelined writer of `handle` from `offset`.
    pub(crate) fn writer(&self, handle: &str, offset: u64) -> Writer {
        Writer::new(Arc::clone(&self.raw), handle, offset)
    }
}

/// (read, write) chunk sizes: the server's limits capped by our packet size, else 32 KiB.
fn chunk_sizes(limits: Option<&Limits>) -> (u32, usize) {
    let read_max = u64::from(MAX_PACKET - READ_OVERHEAD);
    let write_max = u64::from(MAX_PACKET - WRITE_OVERHEAD);
    let pick = |limit: Option<u64>, max: u64| {
        limit
            .filter(|&l| l > 0)
            .map_or(u64::from(FALLBACK_CHUNK), |l| l.min(max))
    };
    let packet = limits.and_then(|l| l.packet_len);
    let read = pick(limits.and_then(|l| l.read_len), read_max).min(packet.map_or(u64::MAX, |p| {
        p.saturating_sub(u64::from(READ_OVERHEAD)).max(1)
    }));
    let write =
        pick(limits.and_then(|l| l.write_len), write_max).min(packet.map_or(u64::MAX, |p| {
            p.saturating_sub(u64::from(WRITE_OVERHEAD)).max(1)
        }));
    (
        u32::try_from(read).unwrap_or(FALLBACK_CHUNK),
        usize::try_from(write).unwrap_or(FALLBACK_CHUNK as usize),
    )
}

/// A name that can be joined to its directory: not empty, `.`, `..`, without `/` or NUL, and at
/// most [`NAME_BYTES_MAX`] bytes.
/// A POSIX server never sends anything else; a hostile one could, to make a later join (delete,
/// download) address a different path. Such entries are dropped from listings.
fn usable_name(name: &str) -> bool {
    let ok = !matches!(name, "" | "." | "..")
        && !name.contains(['/', '\0'])
        && name.len() <= NAME_BYTES_MAX;
    if !ok && !matches!(name, "." | "..") {
        tracing::debug!(name = %crate::sftp::display_name(name), "dropped an invalid entry name from a listing");
    }
    ok
}

fn first_name(files: Vec<russh_sftp::protocol::File>) -> RemoteResult<String> {
    files
        .into_iter()
        .next()
        .map(|f| f.filename)
        .ok_or_else(|| RemoteError::new("the server returned no name"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_sizes_follow_limits_but_stay_bounded() {
        assert_eq!(chunk_sizes(None), (FALLBACK_CHUNK, FALLBACK_CHUNK as usize));
        let openssh = Limits {
            packet_len: Some(256 * 1024),
            read_len: Some(261_120),
            write_len: Some(261_120),
            open_handles: None,
        };
        let (read, write) = chunk_sizes(Some(&openssh));
        assert_eq!(read, 261_120);
        assert!(write <= (MAX_PACKET - WRITE_OVERHEAD) as usize);
        let hostile = Limits {
            packet_len: None,
            read_len: Some(u64::MAX),
            write_len: Some(1 << 40),
            open_handles: None,
        };
        let (read, write) = chunk_sizes(Some(&hostile));
        assert_eq!(read, MAX_PACKET - READ_OVERHEAD);
        assert_eq!(write, (MAX_PACKET - WRITE_OVERHEAD) as usize);
        let tiny = Limits {
            packet_len: Some(10),
            read_len: Some(0),
            write_len: None,
            open_handles: None,
        };
        assert_eq!(chunk_sizes(Some(&tiny)), (1, 1));
    }
}
