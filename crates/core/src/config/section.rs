//! Plain settings keys written in place: one key of `[terminal]`, `[sftp]`, `[recovery]`, … (or
//! a root key such as `default_profile`) changes and every other byte of `config.toml` stays —
//! comments, order, spacing and a trailing `# comment` on the replaced line. Same contract as
//! [`edit_profiles`](super::edit_profiles): the file is never rebuilt from the typed `Config`.

use std::path::Path;

use toml_edit::{DocumentMut, Item, TableLike};

use super::io::{ConfigError, FILE_NAME, write_atomic};

/// A settings value as written to TOML.
#[derive(Clone, Debug, PartialEq)]
pub enum TomlValue {
    /// `true` / `false`.
    Bool(bool),
    /// A whole number.
    Int(i64),
    /// A decimal number.
    Float(f64),
    /// A string.
    Str(String),
    /// An array of strings.
    List(Vec<String>),
}

impl TomlValue {
    /// `value` through its shortest decimal form: `0.6_f32 as f64` would be written as
    /// `0.6000000238418579`.
    #[must_use]
    pub fn float(value: f32) -> Self {
        Self::Float(value.to_string().parse().unwrap_or(f64::from(value)))
    }

    fn to_edit(&self) -> toml_edit::Value {
        match self {
            Self::Bool(b) => (*b).into(),
            Self::Int(n) => (*n).into(),
            Self::Float(f) => (*f).into(),
            Self::Str(s) => s.as_str().into(),
            Self::List(items) => {
                toml_edit::Value::Array(items.iter().map(String::as_str).collect())
            }
        }
    }
}

impl From<bool> for TomlValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<&str> for TomlValue {
    fn from(value: &str) -> Self {
        Self::Str(value.to_owned())
    }
}

impl From<String> for TomlValue {
    fn from(value: String) -> Self {
        Self::Str(value)
    }
}

/// One key to set (`Some`) or remove (`None`: the built-in default applies again).
#[derive(Clone, Debug, PartialEq)]
pub struct KeyEdit {
    /// The key inside its table.
    pub key: String,
    /// The new value; `None` removes the key.
    pub value: Option<TomlValue>,
}

impl KeyEdit {
    /// Sets `key` to `value`.
    #[must_use]
    pub fn set(key: &str, value: impl Into<TomlValue>) -> Self {
        Self {
            key: key.to_owned(),
            value: Some(value.into()),
        }
    }

    /// Removes `key`.
    #[must_use]
    pub fn remove(key: &str) -> Self {
        Self {
            key: key.to_owned(),
            value: None,
        }
    }
}

/// Applies `edits` to the table at `path` (`[]` = the document root, `["terminal"]`,
/// `["profile_defaults", "ssh"]`, …) of `dir/config.toml` in one atomic write. Creates the
/// file and missing tables; a file that does not parse is an error and is never written. Nothing
/// is written when no value actually changes.
pub fn set_keys(dir: &Path, path: &[&str], edits: &[KeyEdit]) -> Result<(), ConfigError> {
    edit_section(dir, path, |table| {
        for edit in edits {
            put(
                table,
                &edit.key,
                edit.value.as_ref().map(TomlValue::to_edit),
            );
        }
    })
}

/// Runs `edit` on the table at `path` of `dir/config.toml` (see [`set_keys`]) and writes the
/// result when it differs from the file.
pub(super) fn edit_section<R>(
    dir: &Path,
    path: &[&str],
    edit: impl FnOnce(&mut dyn TableLike) -> R,
) -> Result<R, ConfigError> {
    let (text, mut doc) = read_doc(dir)?;
    let table = table_at(&mut doc, path).map_err(|message| ConfigError::Parse {
        path: dir.join(FILE_NAME),
        message,
    })?;
    let result = edit(table);
    let new = doc.to_string();
    if new != text {
        write_atomic(dir, new.as_bytes())?;
    }
    Ok(result)
}

/// `dir/config.toml` as text and parsed (empty when missing; `dir` is created).
pub(super) fn read_doc(dir: &Path) -> Result<(String, DocumentMut), ConfigError> {
    std::fs::create_dir_all(dir).map_err(|err| ConfigError::Io {
        path: dir.to_path_buf(),
        source: err,
    })?;
    let path = dir.join(FILE_NAME);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(ConfigError::Io { path, source: err }),
    };
    let doc = text
        .parse()
        .map_err(|e: toml_edit::TomlError| ConfigError::Parse {
            path,
            message: e.to_string(),
        })?;
    Ok((text, doc))
}

/// The table at `path`, creating missing ones. New tables are implicit: a header appears only
/// once the table holds a key, so `[profile_defaults.ssh]` never brings an empty
/// `[profile_defaults]` along.
fn table_at<'d>(doc: &'d mut DocumentMut, path: &[&str]) -> Result<&'d mut dyn TableLike, String> {
    let mut table: &mut dyn TableLike = doc.as_table_mut();
    for (depth, key) in path.iter().enumerate() {
        if table.get(key).is_none() {
            let mut new = toml_edit::Table::new();
            new.set_implicit(true);
            table.insert(key, Item::Table(new));
        }
        let name = || path.get(..=depth).unwrap_or(path).join(".");
        table = table
            .get_mut(key)
            .and_then(Item::as_table_like_mut)
            .ok_or_else(|| format!("{} is not a table", name()))?;
    }
    Ok(table)
}

/// Sets or removes `key`. A replaced value keeps the old line's decor (its trailing comment);
/// an unchanged value is left exactly as written (`10_000` stays `10_000`).
pub(super) fn put(table: &mut dyn TableLike, key: &str, value: Option<toml_edit::Value>) {
    let Some(mut value) = value else {
        table.remove(key);
        return;
    };
    if let Some(old) = table.get(key).and_then(Item::as_value) {
        if same(old, &value) {
            return;
        }
        *value.decor_mut() = old.decor().clone();
    }
    table.insert(key, Item::Value(value));
}

/// Whether two values are equal whatever their formatting.
fn same(a: &toml_edit::Value, b: &toml_edit::Value) -> bool {
    use toml_edit::Value as V;
    match (a, b) {
        (V::String(a), V::String(b)) => a.value() == b.value(),
        (V::Integer(a), V::Integer(b)) => a.value() == b.value(),
        (V::Float(a), V::Float(b)) => a.value().to_bits() == b.value().to_bits(),
        (V::Boolean(a), V::Boolean(b)) => a.value() == b.value(),
        (V::Array(a), V::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b.iter()).all(|(a, b)| same(a, b))
        }
        _ => false,
    }
}

#[cfg(test)]
#[path = "section_tests.rs"]
mod tests;
