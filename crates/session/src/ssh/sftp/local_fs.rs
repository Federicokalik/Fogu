//! Local writes for downloads. Remote names are attacker
//! input, so nothing here ever joins a remote *path* to a local one: the destination directory
//! is opened once, and every file and directory below it is reached from that fd one validated
//! name at a time with `O_NOFOLLOW`, so a pre-existing (or concurrently planted) local symlink
//! can never redirect a write outside it. Files are created `O_EXCL` (no silent overwrite) with
//! mode `0644`/`0755` (never setuid/setgid/sticky) and only become visible under their final
//! name by a rename that refuses to replace (`RENAME_NOREPLACE`) unless the user confirmed.

use std::fs::File;
use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::Path;

use rustix::fs::{self as rfs, AtFlags, FileType, Mode, OFlags, RenameFlags};
use rustix::io::Errno;

use crate::sftp::NameError;

/// Linux `NAME_MAX`: longest single path component in bytes.
pub(crate) const NAME_MAX: usize = 255;
/// Suffix of a download in progress.
pub(crate) const PART_SUFFIX: &str = ".part";

/// Accepts `name` as one local path component, or says why not: empty, `.`, `..`, `/`, NUL,
/// control characters (C0, DEL, C1), bidi overrides/isolates, or longer than `NAME_MAX` bytes.
pub fn validate_name(name: &str) -> Result<&str, NameError> {
    if name.is_empty() {
        return Err(NameError::Empty);
    }
    if name == "." || name == ".." {
        return Err(NameError::Dot);
    }
    if name.len() > NAME_MAX {
        return Err(NameError::Invalid);
    }
    for c in name.chars() {
        match c {
            '/' => return Err(NameError::Slash),
            '\0' => return Err(NameError::Nul),
            c if c.is_control() => return Err(NameError::Control),
            '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' => return Err(NameError::Bidi),
            _ => {}
        }
    }
    Ok(name)
}

/// `<name>.part`, if that still fits in `NAME_MAX`.
pub(crate) fn part_name(name: &str) -> Option<String> {
    (name.len() + PART_SUFFIX.len() <= NAME_MAX).then(|| format!("{name}{PART_SUFFIX}"))
}

/// Local mode for a downloaded file: `0755` if the remote mode has any `x` bit, else `0644`.
/// setuid/setgid/sticky are never copied.
pub(crate) fn file_mode(remote_mode: Option<u32>) -> Mode {
    if remote_mode.is_some_and(|m| m & 0o111 != 0) {
        Mode::from_raw_mode(0o755)
    } else {
        Mode::from_raw_mode(0o644)
    }
}

/// The download destination, opened once.
#[derive(Debug)]
pub(crate) struct LocalDir {
    root: OwnedFd,
}

/// What exists under a name (not following symlinks).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Existing {
    Nothing,
    File,
    Dir,
    /// Symlink, device, FIFO, socket.
    Other,
}

impl LocalDir {
    /// Opens the user-chosen destination directory (following symlinks: the user picked it).
    pub(crate) fn open(path: &Path) -> io::Result<Self> {
        let root = rfs::openat(
            rfs::CWD,
            path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        Ok(Self { root })
    }

    /// The destination directory itself.
    pub(crate) fn fd(&self) -> BorrowedFd<'_> {
        self.root.as_fd()
    }

    /// Opens `rel` (validated names) below the root, refusing to traverse any symlink.
    pub(crate) fn open_dir(&self, rel: &[String]) -> io::Result<OwnedFd> {
        let mut fd = rfs::openat(
            &self.root,
            ".",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        for name in rel {
            fd = open_subdir(&fd, name)?;
        }
        Ok(fd)
    }

    /// Creates the directory `rel` (its parent must exist; created by an earlier call). An
    /// existing directory is fine; an existing symlink or file of that name is an error.
    /// Returns whether it was created.
    pub(crate) fn make_dir(&self, rel: &[String]) -> io::Result<bool> {
        let Some((name, parent)) = rel.split_last() else {
            return Ok(false);
        };
        let parent = self.open_dir(parent)?;
        match rfs::mkdirat(&parent, name.as_str(), Mode::from_raw_mode(0o755)) {
            Ok(()) => Ok(true),
            Err(Errno::EXIST) => match existing(&parent, name)? {
                Existing::Dir => Ok(false),
                _ => Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "exists and is not a directory (symlinks are never followed)",
                )),
            },
            Err(err) => Err(err.into()),
        }
    }
}

fn open_subdir(dir: &OwnedFd, name: &str) -> io::Result<OwnedFd> {
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    Ok(rfs::openat(dir, name, flags, Mode::empty())?)
}

/// What `name` in `dir` is, without following a symlink.
pub(crate) fn existing(dir: &impl AsFd, name: &str) -> io::Result<Existing> {
    match rfs::statat(dir, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) => Ok(match FileType::from_raw_mode(stat.st_mode) {
            FileType::RegularFile => Existing::File,
            FileType::Directory => Existing::Dir,
            _ => Existing::Other,
        }),
        Err(Errno::NOENT) => Ok(Existing::Nothing),
        Err(err) => Err(err.into()),
    }
}

/// Creates `name` in `dir` for writing: `O_NOFOLLOW | O_CREAT | O_EXCL` (or, after a confirmed
/// overwrite, `O_NOFOLLOW | O_CREAT | O_TRUNC`). `O_NONBLOCK` keeps a planted FIFO from
/// blocking the open; anything but a regular file is refused afterwards.
pub(crate) fn create_file(
    dir: BorrowedFd<'_>,
    name: &str,
    mode: Mode,
    truncate_existing: bool,
) -> io::Result<File> {
    let mut flags = OFlags::WRONLY | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::NONBLOCK;
    flags |= OFlags::CLOEXEC;
    flags |= if truncate_existing {
        OFlags::TRUNC
    } else {
        OFlags::EXCL
    };
    let fd = rfs::openat(dir, name, flags, mode)?;
    regular_only(fd)
}

/// Opens an existing `name` (a kept `.part`) to append from its end: `O_NOFOLLOW`, regular files
/// only. Returns the file and its length.
pub(crate) fn open_for_resume(dir: BorrowedFd<'_>, name: &str) -> io::Result<(File, u64)> {
    let flags = OFlags::WRONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    let fd = rfs::openat(dir, name, flags, Mode::empty())?;
    let len = rfs::fstat(&fd)?.st_size;
    Ok((regular_only(fd)?, u64::try_from(len).unwrap_or(0)))
}

fn regular_only(fd: OwnedFd) -> io::Result<File> {
    let stat = rfs::fstat(&fd)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    Ok(File::from(fd))
}

/// Moves the finished `part` to `name` in `dir`. Without `replace` the rename fails if `name`
/// exists (`RENAME_NOREPLACE`, or `link` + `unlink` where the filesystem lacks it); with
/// `replace` an existing `name` is replaced (a symlink there is replaced itself, not followed).
pub(crate) fn finish(dir: BorrowedFd<'_>, part: &str, name: &str, replace: bool) -> io::Result<()> {
    if replace {
        rfs::renameat(dir, part, dir, name)?;
    } else {
        match rfs::renameat_with(dir, part, dir, name, RenameFlags::NOREPLACE) {
            Ok(()) => {}
            Err(Errno::INVAL | Errno::NOSYS) => {
                rfs::linkat(dir, part, dir, name, AtFlags::empty())?;
                rfs::unlinkat(dir, part, AtFlags::empty())?;
            }
            Err(err) => return Err(err.into()),
        }
    }
    // Makes the rename durable; best effort (some filesystems refuse fsync on a directory).
    let _best_effort = rfs::fsync(dir);
    Ok(())
}

/// Removes the file `name` in `dir` (a symlink would be removed itself).
pub(crate) fn remove(dir: BorrowedFd<'_>, name: &str) -> io::Result<()> {
    Ok(rfs::unlinkat(dir, name, AtFlags::empty())?)
}

#[cfg(test)]
#[path = "local_fs_tests.rs"]
mod tests;
