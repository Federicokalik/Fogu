//! In-place profile/group edits of `config.toml` for the profile manager (duplicate, move to
//! group, new group, delete): the file is parsed with `toml_edit` and only the touched entries
//! change, so the user's comments, order and spacing survive (same contract as
//! [`set_profile_background`](super::set_profile_background)). [`ProfileEdit::apply`] mirrors
//! each edit on the running [`Config`].

use std::path::Path;

use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, value};

use super::Config;
use super::io::{ConfigError, FILE_NAME, write_atomic};
use crate::profile::{Group, GroupId, Profile, ProfileId};

/// One change to the stored profiles or groups.
#[derive(Clone, Debug, PartialEq)]
pub enum ProfileEdit {
    /// Append a copy of stored profile `from` (its whole table) with a new `id` and `name`.
    Duplicate {
        /// The stored profile to copy.
        from: ProfileId,
        /// The copy's id (see [`unique_profile_id`]).
        id: ProfileId,
        /// The copy's name.
        name: String,
    },
    /// Append `profile` (a built-in or quick-connect profile being saved).
    Add(Box<Profile>),
    /// Set or clear a stored profile's group.
    SetGroup {
        /// The stored profile.
        id: ProfileId,
        /// The new group; `None` = ungrouped.
        group: Option<GroupId>,
    },
    /// Append a group.
    AddGroup(Group),
    /// Remove a stored profile.
    Delete(ProfileId),
}

impl ProfileEdit {
    /// The same change on the in-memory config (after the file was written).
    pub fn apply(&self, config: &mut Config) {
        match self {
            Self::Duplicate { from, id, name } => {
                if let Some(source) = config.profiles.iter().find(|p| &p.id == from) {
                    let mut copy = source.clone();
                    copy.id = id.clone();
                    copy.name.clone_from(name);
                    config.profiles.push(copy);
                }
            }
            Self::Add(profile) => config.profiles.push(Profile::clone(profile)),
            Self::SetGroup { id, group } => {
                if let Some(p) = config.profiles.iter_mut().find(|p| &p.id == id) {
                    p.group.clone_from(group);
                }
            }
            Self::AddGroup(group) => config.groups.push(group.clone()),
            Self::Delete(id) => config.profiles.retain(|p| &p.id != id),
        }
    }
}

/// Applies `edits` in order to `dir/config.toml` and writes it once (atomic, 0600, previous
/// file as `.bak`). The file must exist and parse, and every profile an edit names must be in
/// it; otherwise nothing is written. When the file does not exist yet and only additions are
/// asked for (`Add`, `AddGroup`), it is created.
pub fn edit_profiles(dir: &Path, edits: &[ProfileEdit]) -> Result<(), ConfigError> {
    let path = dir.join(FILE_NAME);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err)
            if err.kind() == std::io::ErrorKind::NotFound
                && edits
                    .iter()
                    .all(|e| matches!(e, ProfileEdit::Add(_) | ProfileEdit::AddGroup(_))) =>
        {
            String::new()
        }
        Err(err) => return Err(ConfigError::Io { path, source: err }),
    };
    let parse_error = |message: String| ConfigError::Parse {
        path: path.clone(),
        message,
    };
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|e: toml_edit::TomlError| parse_error(e.to_string()))?;
    for edit in edits {
        apply_to_doc(&mut doc, edit).map_err(|err| match err {
            DocError::NoSuchProfile(id) => ConfigError::NoSuchProfile {
                path: path.clone(),
                id,
            },
            DocError::Shape(what) => parse_error(what.to_owned()),
        })?;
    }
    write_atomic(dir, doc.to_string().as_bytes())
}

enum DocError {
    NoSuchProfile(String),
    Shape(&'static str),
}

fn apply_to_doc(doc: &mut DocumentMut, edit: &ProfileEdit) -> Result<(), DocError> {
    match edit {
        ProfileEdit::Duplicate { from, id, name } => {
            let index = find(doc, from)?;
            let mut copy = profiles(doc)?
                .get(index)
                .cloned()
                .ok_or_else(|| DocError::NoSuchProfile(from.0.clone()))?;
            copy.insert("id", value(id.0.as_str()));
            copy.insert("name", value(name.as_str()));
            detach(&mut copy);
            profiles(doc)?.push(copy);
        }
        ProfileEdit::Add(profile) => {
            let text =
                toml::to_string(profile).map_err(|_| DocError::Shape("unserializable profile"))?;
            let parsed: DocumentMut = text
                .parse()
                .map_err(|_| DocError::Shape("unserializable profile"))?;
            let mut table = parsed.as_table().clone();
            detach(&mut table);
            profiles(doc)?.push(table);
        }
        ProfileEdit::SetGroup { id, group } => {
            let index = find(doc, id)?;
            if let Some(table) = profiles(doc)?.get_mut(index) {
                match group {
                    Some(group) => {
                        table.insert("group", value(group.0.as_str()));
                    }
                    None => {
                        table.remove("group");
                    }
                }
            }
        }
        ProfileEdit::AddGroup(group) => {
            let mut table = Table::new();
            table.insert("id", value(group.id.0.as_str()));
            table.insert("name", value(group.name.as_str()));
            detach(&mut table);
            array_of_tables(doc, "groups")?.push(table);
        }
        ProfileEdit::Delete(id) => {
            let index = find(doc, id)?;
            profiles(doc)?.remove(index);
        }
    }
    Ok(())
}

/// `[[profiles]]`, created when missing.
fn profiles(doc: &mut DocumentMut) -> Result<&mut ArrayOfTables, DocError> {
    array_of_tables(doc, "profiles")
}

fn array_of_tables<'a>(
    doc: &'a mut DocumentMut,
    key: &str,
) -> Result<&'a mut ArrayOfTables, DocError> {
    doc.entry(key)
        .or_insert_with(|| Item::ArrayOfTables(ArrayOfTables::new()))
        .as_array_of_tables_mut()
        .ok_or(DocError::Shape("expected [[profiles]] / [[groups]] tables"))
}

/// Index of the `[[profiles]]` entry whose `id` is `id`.
fn find(doc: &mut DocumentMut, id: &ProfileId) -> Result<usize, DocError> {
    profiles(doc)?
        .iter()
        .position(|t| t.get("id").and_then(Item::as_str) == Some(id.0.as_str()))
        .ok_or_else(|| DocError::NoSuchProfile(id.0.clone()))
}

/// A table about to be appended: no leading comments of its own (one blank line instead) and
/// no document position, so it and its sub-tables are written after the last entry.
fn detach(table: &mut Table) {
    table.decor_mut().clear();
    table.decor_mut().set_prefix("\n");
    reset_positions(table);
}

fn reset_positions(table: &mut Table) {
    table.set_position(None);
    for (_, item) in table.iter_mut() {
        match item {
            Item::Table(sub) => reset_positions(sub),
            Item::ArrayOfTables(array) => array.iter_mut().for_each(reset_positions),
            _ => {}
        }
    }
}

/// `base`, or `base-2`, `base-3`, … — the first id no stored profile uses.
#[must_use]
pub fn unique_profile_id(config: &Config, base: &str) -> ProfileId {
    let taken = |id: &str| config.profiles.iter().any(|p| p.id.0 == id);
    ProfileId(unique(base, taken))
}

/// A group id made from `name` (see [`slug`]), unique among the groups.
#[must_use]
pub fn unique_group_id(config: &Config, name: &str) -> GroupId {
    let base = slug(name, "group");
    let taken = |id: &str| config.groups.iter().any(|g| g.id.0 == id);
    GroupId(unique(&base, taken))
}

/// `name` as an id: lowercase letters and digits, other runs as one `-`; `fallback` when
/// nothing is left.
#[must_use]
pub fn slug(name: &str, fallback: &str) -> String {
    let mut slug = String::new();
    for c in name.trim().chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            slug.push(c);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() { fallback } else { slug }.to_owned()
}

fn unique(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_owned();
    }
    (2..=u32::MAX)
        .map(|n| format!("{base}-{n}"))
        .find(|id| !taken(id))
        .unwrap_or_else(|| base.to_owned())
}

#[cfg(test)]
#[path = "edit_tests.rs"]
mod tests;
