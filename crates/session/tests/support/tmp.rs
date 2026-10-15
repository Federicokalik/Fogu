//! A private temp directory removed on drop (also while unwinding from a failed test).

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// `$TMPDIR/fogu-<label>-<pid>-<n>`, mode 0700.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(label: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("fogu-{label}-{}-{n}", std::process::id()));
        let _stale = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create temp dir");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).expect("chmod temp dir");
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _best_effort = fs::remove_dir_all(&self.0);
    }
}
