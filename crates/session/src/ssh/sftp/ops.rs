//! Metadata operations: list, stat, realpath, mkdir, rename, remove (recursive), count,
//! chmod. Each runs as its own task and answers with exactly one event.

use std::sync::Arc;

use russh_sftp::protocol::FileAttributes;
use tokio::sync::mpsc::Sender;
use tokio::task::JoinSet;

use super::remote::{Remote, RemoteError, RemoteResult, kind_of};
use crate::sftp::{CmdId, Entry, EntryKind, SftpCmd, SftpEvent, SftpOptions};

/// Symlinks per listing whose target is resolved (`readlink` + `stat`, pipelined); the rest
/// are listed without a target.
pub(crate) const LINK_RESOLVE_MAX: usize = 256;

/// Path bytes one recursive walk may hold (its job list / directory list).
const WALK_BYTES_MAX: usize = 64 * 1024 * 1024;

/// The caps of one recursive walk (entries, depth via the caller, path bytes): a hostile server
/// can make a tree as wide, deep and long-named as it likes.
pub(crate) struct WalkBudget {
    entries: usize,
    bytes: usize,
    max_entries: usize,
}

impl WalkBudget {
    pub(crate) fn new(opts: &SftpOptions) -> Self {
        Self {
            entries: 0,
            bytes: 0,
            max_entries: opts.max_walk_entries,
        }
    }

    /// Accounts one entry whose full path is `path_len` bytes long.
    pub(crate) fn take(&mut self, path_len: usize) -> Result<(), RemoteError> {
        self.entries += 1;
        self.bytes += path_len;
        if self.entries > self.max_entries || self.bytes > WALK_BYTES_MAX {
            return Err(RemoteError::new("too many files"));
        }
        Ok(())
    }
}

/// `dir` + `/` + `name` (SFTP paths are always `/`-separated).
pub(crate) fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_owned()
    } else if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// Last component of a remote path (`"/a/b/"` → `"b"`, `"/"` → `""`).
pub(crate) fn last_component(path: &str) -> &str {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default()
}

/// An [`Entry`] from a name and its `lstat` attributes (link fields filled later).
pub(crate) fn entry(name: String, attrs: &FileAttributes) -> Entry {
    Entry {
        name,
        kind: kind_of(attrs),
        size: attrs.size,
        mtime: attrs.mtime,
        mode: attrs.permissions.map(|m| m & 0o7777),
        uid: attrs.uid,
        gid: attrs.gid,
        link_target: None,
        link_kind: None,
    }
}

/// Runs one metadata command and sends its single answer.
pub(crate) async fn run(
    remote: Arc<Remote>,
    opts: SftpOptions,
    id: CmdId,
    cmd: SftpCmd,
    events: Sender<SftpEvent>,
) {
    let result = match cmd {
        SftpCmd::List(path) => {
            list(&remote, &path, opts.max_listing_entries)
                .await
                .map(|(entries, truncated)| SftpEvent::Listing {
                    id,
                    path,
                    entries,
                    truncated,
                })
        }
        SftpCmd::Stat(path) => stat(&remote, &path)
            .await
            .map(|entry| SftpEvent::StatResult { id, path, entry }),
        SftpCmd::Realpath(path) => remote
            .realpath(&path)
            .await
            .map(|path| SftpEvent::Realpath { id, path }),
        SftpCmd::Mkdir(path) => remote.mkdir(&path).await.map(|()| SftpEvent::Done(id)),
        SftpCmd::Rename(from, to) => remote
            .rename(&from, &to)
            .await
            .map(|()| SftpEvent::Done(id)),
        SftpCmd::RemoveFile(path) => remote
            .remove_file(&path)
            .await
            .map(|()| SftpEvent::Done(id)),
        SftpCmd::RemoveDirRecursive(path) => remove_tree(&remote, &path, &opts)
            .await
            .map(|()| SftpEvent::Done(id)),
        SftpCmd::CountDir(path) => walk_tree(&remote, &path, &opts).await.map(|tree| {
            let entries = (tree.files.len() + tree.dirs.len()).saturating_sub(1) as u64;
            SftpEvent::Counted { id, path, entries }
        }),
        SftpCmd::Chmod(path, mode) => remote
            .chmod(&path, mode)
            .await
            .map(|()| SftpEvent::Done(id)),
        other => {
            tracing::error!(?other, "not a metadata command");
            Err(RemoteError::new("internal error: not a metadata command"))
        }
    };
    let event = result.unwrap_or_else(|err| SftpEvent::Error {
        id,
        message: err.message,
    });
    let _gone = events.send(event).await;
}

/// Lists `path`: dirs first, then by name; symlink targets resolved for the first
/// [`LINK_RESOLVE_MAX`] links.
pub(crate) async fn list(
    remote: &Arc<Remote>,
    path: &str,
    cap: usize,
) -> RemoteResult<(Vec<Entry>, bool)> {
    let (raw, truncated) = remote.read_dir(path, cap).await?;
    let mut entries: Vec<Entry> = raw
        .into_iter()
        .map(|(name, attrs)| entry(name, &attrs))
        .collect();
    let mut lookups = JoinSet::new();
    let links = entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.kind == EntryKind::Symlink);
    for (index, link) in links.take(LINK_RESOLVE_MAX) {
        let (remote, full) = (Arc::clone(remote), join(path, &link.name));
        lookups.spawn(async move { (index, resolve_link(&remote, &full).await) });
    }
    while let Some(done) = lookups.join_next().await {
        if let Ok((index, (target, kind))) = done
            && let Some(entry) = entries.get_mut(index)
        {
            entry.link_target = target;
            entry.link_kind = kind;
        }
    }
    entries.sort_by(|a, b| {
        let dir = |e: &Entry| e.kind != EntryKind::Dir && e.link_kind != Some(EntryKind::Dir);
        dir(a).cmp(&dir(b)).then_with(|| a.name.cmp(&b.name))
    });
    Ok((entries, truncated))
}

/// `readlink` + `stat` of a symlink (each `None` on failure: dangling links are normal).
async fn resolve_link(remote: &Remote, path: &str) -> (Option<String>, Option<EntryKind>) {
    let target = remote.read_link(path).await.ok();
    let kind = remote.stat(path).await.ok().map(|attrs| kind_of(&attrs));
    (target, kind)
}

/// `lstat` of one path as an [`Entry`] named after its last component.
async fn stat(remote: &Arc<Remote>, path: &str) -> RemoteResult<Entry> {
    let attrs = remote.lstat(path).await?;
    let mut entry = entry(last_component(path).to_owned(), &attrs);
    if entry.kind == EntryKind::Symlink {
        (entry.link_target, entry.link_kind) = resolve_link(remote, path).await;
    }
    Ok(entry)
}

/// Everything below a directory, found by [`walk_tree`].
struct Tree {
    /// Files, symlinks and special entries (removed with `remove`, never followed).
    files: Vec<String>,
    /// Directories, parents first (the root first).
    dirs: Vec<String>,
}

/// Walks the tree at `path` without following symlinks. The root must be a real directory (a
/// symlink is refused: delete it as a file). Bounded by the walk caps; nothing is changed.
async fn walk_tree(remote: &Remote, path: &str, opts: &SftpOptions) -> RemoteResult<Tree> {
    if kind_of(&remote.lstat(path).await?) != EntryKind::Dir {
        return Err(RemoteError::new(
            "not a directory (symlinks are not followed)",
        ));
    }
    let mut stack = vec![(path.to_owned(), 0_usize)];
    let mut tree = Tree {
        files: Vec::new(),
        dirs: Vec::new(),
    };
    let mut budget = WalkBudget::new(opts);
    while let Some((dir, depth)) = stack.pop() {
        if depth > opts.max_walk_depth {
            return Err(RemoteError::new("directory tree too deep (max_walk_depth)"));
        }
        let (entries, truncated) = remote.read_dir(&dir, opts.max_listing_entries).await?;
        if truncated {
            return Err(RemoteError::new(
                "a directory is too large (max_listing_entries)",
            ));
        }
        for (name, attrs) in entries {
            let child = join(&dir, &name);
            if budget.take(child.len()).is_err() {
                return Err(RemoteError::new(
                    "directory tree too large (max_walk_entries)",
                ));
            }
            if kind_of(&attrs) == EntryKind::Dir {
                stack.push((child, depth + 1));
            } else {
                tree.files.push(child);
            }
        }
        tree.dirs.push(dir);
    }
    Ok(tree)
}

/// Deletes the tree at `path`: the whole tree is walked first (a tree over the caps is not
/// touched at all), then files and links are removed, then directories deepest first.
async fn remove_tree(remote: &Remote, path: &str, opts: &SftpOptions) -> RemoteResult<()> {
    let tree = walk_tree(remote, path, opts)
        .await
        .map_err(|e| RemoteError {
            message: format!("{}; nothing was deleted", e.message),
            ..e
        })?;
    for file in &tree.files {
        remote.remove_file(file).await?;
    }
    for dir in tree.dirs.iter().rev() {
        remote.remove_dir(dir).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_paths_join_and_split() {
        assert_eq!(join("/home/u", "a"), "/home/u/a");
        assert_eq!(join("/", "a"), "/a");
        assert_eq!(join("", "a"), "a");
        assert_eq!(last_component("/a/b"), "b");
        assert_eq!(last_component("/a/b/"), "b");
        assert_eq!(last_component("b"), "b");
        assert_eq!(last_component("/"), "");
        assert_eq!(last_component("/a/.."), "..");
    }
}
