//! Private-by-default filesystem helpers: state and
//! config directories are `0700`, files we create are `0600`. Plain `std::fs`.
//!
//! Whole-file rewrites go through [`write_private_atomic`]: a fresh temp file (`create_new`, so
//! an existing file or symlink of that name is never opened), `fsync`, then `rename` over the
//! target. A crash leaves either the old or the new content, never a mix.

use std::fs::Permissions;
use std::fs::{DirBuilder, File, OpenOptions};
use std::io::{self, Write as _};
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// Mode of directories we create.
pub const DIR_MODE: u32 = 0o700;
/// Mode of files we create.
pub const FILE_MODE: u32 = 0o600;

/// Creates `dir` and missing parents with mode `0700` (existing directories are left alone).
pub fn create_private_dir(dir: &Path) -> io::Result<()> {
    DirBuilder::new().recursive(true).mode(DIR_MODE).create(dir)
}

/// Like [`create_private_dir`], but an existing `dir` is also set to `0700`: for directories
/// only Fogu owns (state, logs), where files created later with the default umask (the daily
/// log) must still be private.
pub fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    create_private_dir(dir)?;
    std::fs::set_permissions(dir, Permissions::from_mode(DIR_MODE))
}

/// Opens `path` for appending, creating it with mode `0600`.
pub fn open_private_append(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .mode(FILE_MODE)
        .open(path)
}

/// Appends `bytes` to `path` (created `0600`).
pub fn append_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = open_private_append(path)?;
    file.write_all(bytes)
}

/// Replaces `path` with `bytes` atomically: writes a new `0600` temp file next to it
/// (`O_CREAT | O_EXCL`), `fsync`s it, renames it over `path`, then `fsync`s the directory (best
/// effort). The temp file is removed if any step fails. The parent directory must exist.
pub fn write_private_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = temp_sibling(path)?;
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(FILE_MODE)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _best_effort = std::fs::remove_file(&tmp);
        return result;
    }
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        // Makes the rename itself durable; some filesystems refuse fsync on a directory.
        let _best_effort = File::open(dir).and_then(|d| d.sync_all());
    }
    Ok(())
}

/// `.<name>.tmp-<pid>-<n>` in the directory of `path` (unique per process and call).
fn temp_sibling(path: &Path) -> io::Result<PathBuf> {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut tmp = std::ffi::OsString::from(".");
    tmp.push(name);
    tmp.push(format!(".tmp-{}-{n}", std::process::id()));
    Ok(path.with_file_name(tmp))
}

/// Sets mode `0600` on every regular file in `dir` whose name starts with `prefix` (log files
/// written by a library that creates them with the umask's mode). Best effort per file.
pub fn restrict_files(dir: &Path, prefix: &str) -> io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_string_lossy().starts_with(prefix) && entry.file_type()?.is_file() {
            std::fs::set_permissions(entry.path(), Permissions::from_mode(FILE_MODE))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![expect(clippy::unwrap_used, reason = "tests")]

    use super::*;

    #[test]
    fn dirs_are_0700_and_appended_files_0600() {
        let root = std::env::temp_dir().join(format!("fogu-fsutil-{}", std::process::id()));
        let dir = root.join("a/b");
        create_private_dir(&dir).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), DIR_MODE);
        let file = dir.join("crash.log");
        append_private(&file, b"one\n").unwrap();
        append_private(&file, b"two\n").unwrap();
        assert_eq!(mode(&file), FILE_MODE);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "one\ntwo\n");
        std::fs::write(dir.join("app.log.1"), "x").unwrap();
        std::fs::write(dir.join("other"), "x").unwrap();
        restrict_files(&dir, "app.log").unwrap();
        assert_eq!(mode(&dir.join("app.log.1")), FILE_MODE);
        assert_ne!(mode(&dir.join("other")), FILE_MODE);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn ensure_private_dir_tightens_an_existing_open_dir() {
        let dir = std::env::temp_dir().join(format!("fogu-fsutil-open-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, Permissions::from_mode(0o755)).unwrap();
        create_private_dir(&dir).unwrap();
        let mode = || std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode(),
            0o755,
            "create_private_dir leaves existing dirs alone"
        );
        ensure_private_dir(&dir).unwrap();
        assert_eq!(mode(), DIR_MODE);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn atomic_write_replaces_content_with_a_private_file_and_leaves_no_temp() {
        let root = std::env::temp_dir().join(format!("fogu-atomic-{}", std::process::id()));
        create_private_dir(&root).unwrap();
        let file = root.join("known_hosts");
        std::fs::write(&file, "old").unwrap();
        std::fs::set_permissions(&file, Permissions::from_mode(0o644)).unwrap();
        write_private_atomic(&file, b"new\n").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "new\n");
        let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, FILE_MODE);
        let names: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["known_hosts"]);
        // A missing parent fails cleanly (nothing to clean up).
        assert!(write_private_atomic(&root.join("no/such/file"), b"x").is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
