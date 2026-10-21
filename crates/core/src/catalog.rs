//! Shared loading for the user catalogs under the config directory (`themes/` colour schemes,
//! `styles/` UI styles): every `*.toml` file, sorted by file name, each capped in size so one bad
//! file cannot stall startup. Problems become [`CatalogWarning`]s, never errors.

use std::fmt;
use std::path::{Path, PathBuf};

/// Largest catalog file accepted.
const MAX_FILE_BYTES: u64 = 256 * 1024;

/// A catalog file (or directory) that was skipped; loading continues with the other files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogWarning {
    /// The file (or catalog directory) that caused the warning.
    pub path: PathBuf,
    /// Why it was skipped.
    pub message: String,
}

impl fmt::Display for CatalogWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.message)
    }
}

/// The `*.toml` files of `dir`, sorted by file name, each read as text. A missing directory is
/// normal (no files, no warning); unreadable entries and oversized files become warnings.
pub(crate) fn read_toml_dir(dir: &Path) -> (Vec<(PathBuf, String)>, Vec<CatalogWarning>) {
    let mut warnings = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return (Vec::new(), warnings),
        Err(err) => {
            warnings.push(CatalogWarning {
                path: dir.to_path_buf(),
                message: format!("cannot read directory: {err}"),
            });
            return (Vec::new(), warnings);
        }
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(std::ffi::OsStr::to_str)
                .is_some_and(|ext| ext.eq_ignore_ascii_case("toml"))
        })
        .collect();
    files.sort();
    let mut texts = Vec::with_capacity(files.len());
    for path in files {
        match read_capped(&path) {
            Ok(text) => texts.push((path, text)),
            Err(message) => warnings.push(CatalogWarning { path, message }),
        }
    }
    (texts, warnings)
}

fn read_capped(path: &Path) -> Result<String, String> {
    let size = std::fs::metadata(path)
        .map_err(|err| format!("cannot inspect file: {err}"))?
        .len();
    if size > MAX_FILE_BYTES {
        return Err(format!(
            "file is {size} bytes, over the {MAX_FILE_BYTES} byte limit"
        ));
    }
    std::fs::read_to_string(path).map_err(|err| format!("cannot read file: {err}"))
}

/// Writes a custom catalog file: over `existing` when editing one, else to
/// `dir/<file name from name>.toml`, refusing to replace a file that is already there. Returns
/// the path written.
pub(crate) fn write_custom(
    dir: &Path,
    name: &str,
    text: &str,
    existing: Option<&Path>,
) -> Result<PathBuf, String> {
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(format!("the file is over the {MAX_FILE_BYTES} byte limit"));
    }
    let path = if let Some(path) = existing {
        path.to_path_buf()
    } else {
        let file = format!("{}.toml", crate::config::slug(name, "custom"));
        let path = dir.join(file);
        if path.exists() {
            return Err(format!(
                "{} already exists; pick another name",
                path.display()
            ));
        }
        path
    };
    std::fs::create_dir_all(dir)
        .map_err(|err| format!("cannot create {}: {err}", dir.display()))?;
    std::fs::write(&path, text).map_err(|err| format!("cannot write {}: {err}", path.display()))?;
    Ok(path)
}
