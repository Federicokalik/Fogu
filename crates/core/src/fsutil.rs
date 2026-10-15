//! Private-by-default filesystem helpers: state and
//! config directories are `0700`, files we create are `0600`. Plain `std::fs`.

use std::fs::Permissions;
use std::fs::{DirBuilder, File, OpenOptions};
use std::io::{self, Write as _};
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::Path;

/// Mode of directories we create.
pub const DIR_MODE: u32 = 0o700;
/// Mode of files we create.
pub const FILE_MODE: u32 = 0o600;

/// Creates `dir` and missing parents with mode `0700` (existing directories are left alone).
pub fn create_private_dir(dir: &Path) -> io::Result<()> {
    DirBuilder::new().recursive(true).mode(DIR_MODE).create(dir)
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
}
