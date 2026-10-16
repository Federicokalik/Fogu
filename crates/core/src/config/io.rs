//! Config file I/O: XDG path, atomic save, load with graceful fallback, and semantic validation.

use std::collections::{HashMap, HashSet};
use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};

use super::Config;
use crate::action::Action;
use crate::background::{Background, Fit};
use crate::keychord::KeyChord;
use crate::profile::ProfileId;
use crate::scheme::ColorScheme;

/// The config file's name inside [`config_dir`].
pub(super) const FILE_NAME: &str = "config.toml";
/// Permissions for the config file and its backup (may contain nothing secret today, but
/// `profiles` will later carry hostnames/usernames some users consider sensitive).
const FILE_MODE: u32 = 0o600;

/// Something went wrong loading or saving the config file.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// A filesystem operation failed.
    #[error("{path}: {source}")]
    Io {
        /// The path being read/written/renamed.
        path: PathBuf,
        /// The underlying OS error.
        #[source]
        source: std::io::Error,
    },
    /// The file's content was not valid TOML, or did not match the `Config` schema. `message`
    /// is `toml`'s own `Display`, which includes the line/column when available.
    #[error("{path}: {message}")]
    Parse {
        /// The file that failed to parse.
        path: PathBuf,
        /// `toml::de::Error`'s message (line/column included for syntax errors).
        message: String,
    },
    /// The profile to edit is not in the config file.
    #[error("{path}: no profile with id {id:?}")]
    NoSuchProfile {
        /// The config file that was searched.
        path: PathBuf,
        /// The profile id that was not found.
        id: String,
    },
}

impl ConfigError {
    fn io(path: &Path, source: std::io::Error) -> Self {
        Self::Io {
            path: path.to_path_buf(),
            source,
        }
    }
}

/// `$XDG_CONFIG_HOME/fogu` (default `~/.config/fogu`), or `override_dir` verbatim when
/// given (the `--config-dir` CLI flag).
#[must_use]
pub fn config_dir(override_dir: Option<&Path>) -> PathBuf {
    if let Some(dir) = override_dir {
        return dir.to_path_buf();
    }
    xdg::BaseDirectories::with_prefix("fogu")
        .get_config_home()
        .unwrap_or_else(|| PathBuf::from(".config/fogu"))
}

/// Loads `config.toml` from `dir`. A missing file is not an error (returns `Config::default()`);
/// any other failure (unreadable file, bad TOML, schema mismatch) also falls back to defaults but
/// is returned alongside so the caller can log it and show a toast.
#[must_use]
pub fn load(dir: &Path) -> (Config, Option<ConfigError>) {
    let path = dir.join(FILE_NAME);
    match std::fs::read_to_string(&path) {
        Ok(text) => match parse(&text, &path) {
            Ok(config) => (config, None),
            Err(err) => (Config::default(), Some(err)),
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => (Config::default(), None),
        Err(err) => (Config::default(), Some(ConfigError::io(&path, err))),
    }
}

fn parse(text: &str, path: &Path) -> Result<Config, ConfigError> {
    let to_parse_error = |message: &dyn std::fmt::Display| ConfigError::Parse {
        path: path.to_path_buf(),
        message: message.to_string(),
    };
    let raw: toml::Table = toml::from_str(text).map_err(|err| to_parse_error(&err))?;
    let raw = migrate(raw);
    toml::Value::Table(raw)
        .try_into()
        .map_err(|err: toml::de::Error| to_parse_error(&err))
}

/// Upgrades an older raw TOML table to the current schema, operating before typed
/// deserialisation. A no-op
/// chain today: `version = 1` is the only version that has ever existed. Add a `version == N =>
/// { ...; table.insert("version", (N + 1).into()); }` arm here for the next breaking change.
fn migrate(table: toml::Table) -> toml::Table {
    table
}

/// Sets (`Some`) or removes (`None`) the `background` key of the `[[profiles]]` entry with `id`
/// in `dir/config.toml`, leaving every other byte of the file (comments, order, spacing)
/// untouched. The file must already exist, parse as TOML and contain the profile: nothing is
/// ever created or rewritten from the typed `Config`. Written atomically (temp file, `fsync`,
/// rename), mode `0600`, keeping the previous file as `config.toml.bak`.
pub fn set_profile_background(
    dir: &Path,
    id: &str,
    background: Option<&Background>,
) -> Result<(), ConfigError> {
    let path = dir.join(FILE_NAME);
    let text = std::fs::read_to_string(&path).map_err(|err| ConfigError::io(&path, err))?;
    let parse_error = |message: String| ConfigError::Parse {
        path: path.clone(),
        message,
    };
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|e: toml_edit::TomlError| parse_error(e.to_string()))?;
    let profile = doc
        .get_mut("profiles")
        .and_then(toml_edit::Item::as_array_of_tables_mut)
        .and_then(|profiles| {
            profiles
                .iter_mut()
                .find(|p| p.get("id").and_then(toml_edit::Item::as_str) == Some(id))
        })
        .ok_or_else(|| ConfigError::NoSuchProfile {
            path: path.clone(),
            id: id.to_owned(),
        })?;
    match background {
        None => {
            profile.remove("background");
        }
        Some(background) => {
            let mut value = background_value(background);
            // Keep a trailing `# comment` on the line being replaced.
            if let Some(old) = profile
                .get("background")
                .and_then(toml_edit::Item::as_value)
            {
                *value.decor_mut() = old.decor().clone();
            }
            profile.insert("background", toml_edit::Item::Value(value));
        }
    }
    write_atomic(dir, doc.to_string().as_bytes())
}

/// `background` as the inline table the docs show: `{ color = "#rrggbb" }` or
/// `{ image = { path = "…", opacity = 0.6, fit = "contain" } }` (`blur` only when on).
fn background_value(background: &Background) -> toml_edit::Value {
    let mut outer = toml_edit::InlineTable::new();
    match background {
        Background::Color(color) => {
            outer.insert("color", color.to_string().into());
        }
        Background::Image {
            path,
            opacity,
            fit,
            blur,
        } => {
            let mut image = toml_edit::InlineTable::new();
            image.insert("path", path.to_string_lossy().as_ref().into());
            // Through the shortest decimal form: 0.6_f32 as f64 would print 0.6000000238418579.
            let opacity = opacity.to_string().parse::<f64>().unwrap_or(1.0);
            image.insert("opacity", opacity.into());
            let fit = match fit {
                Fit::Cover => "cover",
                Fit::Contain => "contain",
                Fit::Tile => "tile",
            };
            image.insert("fit", fit.into());
            if *blur {
                image.insert("blur", true.into());
            }
            outer.insert("image", toml_edit::Value::InlineTable(image));
        }
    }
    toml_edit::Value::InlineTable(outer)
}

pub(super) fn write_atomic(dir: &Path, contents: &[u8]) -> Result<(), ConfigError> {
    let dest = dir.join(FILE_NAME);
    let tmp = dir.join(format!("{FILE_NAME}.tmp"));
    write_new_file(&tmp, contents)?;
    let bak = dir.join(format!("{FILE_NAME}.bak"));
    match std::fs::rename(&dest, &bak) {
        // The user's original may have been created 0644; the backup is private too.
        Ok(()) => {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&bak, std::fs::Permissions::from_mode(FILE_MODE))
                .map_err(|err| ConfigError::io(&bak, err))?;
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(ConfigError::io(&dest, err)),
    }
    std::fs::rename(&tmp, &dest).map_err(|err| ConfigError::io(&tmp, err))
}

fn write_new_file(path: &Path, contents: &[u8]) -> Result<(), ConfigError> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(FILE_MODE)
        .open(path)
        .map_err(|err| ConfigError::io(path, err))?;
    file.write_all(contents)
        .map_err(|err| ConfigError::io(path, err))?;
    file.sync_all().map_err(|err| ConfigError::io(path, err))
}

/// One semantic problem found by [`validate`]. Syntax errors never reach this point (they fail
/// at [`load`] time instead).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ConfigIssue {
    /// Two profiles share the same `id`.
    #[error("duplicate profile id {0:?}")]
    DuplicateProfileId(ProfileId),
    /// A profile's `group` does not match any entry in `groups`.
    #[error("profile {profile:?} references unknown group {group:?}")]
    UnknownGroup {
        /// The profile with the dangling reference.
        profile: ProfileId,
        /// The group id it references.
        group: String,
    },
    /// `appearance.color_scheme` does not match a built-in scheme.
    #[error("unknown colour scheme {0:?}")]
    UnknownColorScheme(String),
    /// The same chord is bound to more than one action.
    #[error("{chord} is bound to more than one action: {actions:?}")]
    AmbiguousChord {
        /// The shared chord.
        chord: KeyChord,
        /// The actions it is bound to.
        actions: Vec<Action>,
    },
}

/// Checks `config` for problems that successful deserialisation cannot catch: duplicate ids,
/// dangling group references, an unknown colour scheme name, and chords bound to more than one
/// action. Returns every issue found (never panics, never fails to load the config).
#[must_use]
pub fn validate(config: &Config) -> Vec<ConfigIssue> {
    let mut issues = Vec::new();

    let mut seen_ids = HashSet::new();
    let group_ids: HashSet<&str> = config.groups.iter().map(|g| g.id.0.as_str()).collect();
    for profile in &config.profiles {
        if !seen_ids.insert(&profile.id) {
            issues.push(ConfigIssue::DuplicateProfileId(profile.id.clone()));
        }
        if let Some(group) = &profile.group
            && !group_ids.contains(group.0.as_str())
        {
            issues.push(ConfigIssue::UnknownGroup {
                profile: profile.id.clone(),
                group: group.0.clone(),
            });
        }
    }

    if ColorScheme::by_name(&config.appearance.color_scheme).is_none() {
        issues.push(ConfigIssue::UnknownColorScheme(
            config.appearance.color_scheme.clone(),
        ));
    }

    let mut by_chord: HashMap<KeyChord, Vec<Action>> = HashMap::new();
    for (&action, chords) in config.hotkeys.bindings() {
        for &chord in chords {
            by_chord.entry(chord).or_default().push(action);
        }
    }
    for (chord, actions) in by_chord {
        if actions.len() > 1 {
            issues.push(ConfigIssue::AmbiguousChord { chord, actions });
        }
    }

    issues
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::indexing_slicing,
        reason = "tests"
    )]

    use super::*;
    use crate::profile::{Group, GroupId, LocalOpts, Profile, ProfileKind};

    fn profile(id: &str) -> Profile {
        Profile {
            id: ProfileId(id.to_owned()),
            kind: ProfileKind::Local(LocalOpts::default()),
            ..Profile::default_shell()
        }
    }

    #[test]
    fn config_dir_honours_the_override() {
        let dir = config_dir(Some(Path::new("/tmp/example")));
        assert_eq!(dir, PathBuf::from("/tmp/example"));
    }

    #[test]
    fn config_dir_without_override_is_under_home() {
        let dir = config_dir(None);
        assert!(dir.ends_with("fogu"), "{}", dir.display());
    }

    #[test]
    fn missing_file_loads_defaults_without_an_error() {
        let tmp = tempdir();
        let (config, err) = load(tmp.path());
        assert!(err.is_none());
        assert_eq!(config, Config::default());
    }

    #[test]
    fn syntax_error_falls_back_to_defaults_and_reports_line_col() {
        let tmp = tempdir();
        std::fs::write(tmp.path().join(FILE_NAME), "not valid toml [[[").unwrap();
        let (config, err) = load(tmp.path());
        assert_eq!(config, Config::default());
        let err = err.expect("a parse error");
        let message = err.to_string();
        assert!(message.contains("line"), "{message}");
    }

    const FIXTURE: &str = "\
# my hand-written config
version = 1

[appearance]
font_size = 14   # big enough
color_scheme = \"Dracula\"


# --- work ---
[[profiles]]
name = \"Work\"   # shown in the menu
id = \"work\"
type = \"local\"

# --- play ---
[[profiles]]
type = \"local\"
id = \"play\"
name = \"Play\"
background = { color = \"#112233\" }   # keep me
";

    fn fixture(tmp: &TempDir) -> PathBuf {
        let path = tmp.path().join(FILE_NAME);
        std::fs::write(&path, FIXTURE).unwrap();
        path
    }

    fn differing_lines(before: &str, after: &str) -> (Vec<String>, Vec<String>) {
        let b: Vec<&str> = before.lines().collect();
        let a: Vec<&str> = after.lines().collect();
        let prefix = b.iter().zip(&a).take_while(|(x, y)| x == y).count();
        let suffix = b[prefix..]
            .iter()
            .rev()
            .zip(a[prefix..].iter().rev())
            .take_while(|(x, y)| x == y)
            .count();
        let owned = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect();
        (
            owned(&b[prefix..b.len() - suffix]),
            owned(&a[prefix..a.len() - suffix]),
        )
    }

    #[test]
    fn set_background_adds_only_that_key_and_keeps_a_0600_backup() {
        let tmp = tempdir();
        let path = fixture(&tmp);
        let bg = Background::Image {
            path: PathBuf::from("~/wall.jpg"),
            opacity: 0.6,
            fit: Fit::Contain,
            blur: false,
        };
        set_profile_background(tmp.path(), "work", Some(&bg)).unwrap();

        let after = std::fs::read_to_string(&path).unwrap();
        let (removed, added) = differing_lines(FIXTURE, &after);
        assert!(removed.is_empty(), "{removed:?}");
        assert_eq!(
            added,
            [
                "background = { image = { path = \"~/wall.jpg\", opacity = 0.6, fit = \"contain\" } }"
            ]
        );
        // Everything else is byte-identical: removing the added line gives the original back.
        let restored = after.replace(&format!("{}\n", added[0]), "");
        assert_eq!(restored, FIXTURE);

        let (config, err) = load(tmp.path());
        assert!(err.is_none());
        assert_eq!(config.profiles[0].background, Some(bg));
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), FILE_MODE);
        let bak = tmp.path().join(format!("{FILE_NAME}.bak"));
        assert_eq!(std::fs::read_to_string(&bak).unwrap(), FIXTURE);
        assert_eq!(mode(&bak), FILE_MODE);
    }

    #[test]
    fn set_background_replaces_in_place_and_keeps_the_trailing_comment() {
        let tmp = tempdir();
        let path = fixture(&tmp);
        let bg = Background::Color(crate::RgbColor::from_hex(0x00_aa_bb));
        set_profile_background(tmp.path(), "play", Some(&bg)).unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        let (removed, added) = differing_lines(FIXTURE, &after);
        assert_eq!(
            removed,
            ["background = { color = \"#112233\" }   # keep me"]
        );
        assert_eq!(added, ["background = { color = \"#00aabb\" }   # keep me"]);
    }

    #[test]
    fn remove_background_deletes_only_that_line() {
        let tmp = tempdir();
        let path = fixture(&tmp);
        set_profile_background(tmp.path(), "play", None).unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        let (removed, added) = differing_lines(FIXTURE, &after);
        assert_eq!(
            removed,
            ["background = { color = \"#112233\" }   # keep me"]
        );
        assert!(added.is_empty(), "{added:?}");
    }

    #[test]
    fn unknown_profile_or_unparsable_or_missing_file_is_an_error_and_untouched() {
        let tmp = tempdir();
        let path = fixture(&tmp);
        let bg = Background::Color(crate::RgbColor::from_hex(1));
        let err = set_profile_background(tmp.path(), "gone", Some(&bg)).unwrap_err();
        assert!(matches!(err, ConfigError::NoSuchProfile { .. }), "{err}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), FIXTURE);
        assert!(!tmp.path().join(format!("{FILE_NAME}.bak")).exists());

        std::fs::write(&path, "not valid toml [[[").unwrap();
        let err = set_profile_background(tmp.path(), "work", Some(&bg)).unwrap_err();
        assert!(matches!(err, ConfigError::Parse { .. }), "{err}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "not valid toml [[["
        );
        assert!(!tmp.path().join(format!("{FILE_NAME}.bak")).exists());

        std::fs::remove_file(&path).unwrap();
        let err = set_profile_background(tmp.path(), "work", Some(&bg)).unwrap_err();
        assert!(matches!(err, ConfigError::Io { .. }), "{err}");
        assert!(!path.exists());
    }

    #[test]
    fn validate_catches_duplicate_profile_ids() {
        let config = Config {
            profiles: vec![profile("dup"), profile("dup")],
            ..Config::default()
        };
        let issues = validate(&config);
        assert!(
            issues
                .iter()
                .any(|i| matches!(i, ConfigIssue::DuplicateProfileId(id) if id.0 == "dup")),
            "{issues:?}"
        );
    }

    #[test]
    fn validate_catches_unknown_group_and_scheme() {
        let mut p = profile("solo");
        p.group = Some(GroupId("nonexistent".to_owned()));
        let config = Config {
            profiles: vec![p],
            groups: vec![Group {
                id: GroupId("other".to_owned()),
                name: "Other".to_owned(),
            }],
            appearance: super::super::Appearance {
                color_scheme: "Not A Real Scheme".to_owned(),
                ..super::super::Appearance::default()
            },
            ..Config::default()
        };
        let issues = validate(&config);
        assert!(matches!(
            &issues[0],
            ConfigIssue::UnknownGroup { group, .. } if group == "nonexistent"
        ));
        assert!(
            issues.iter().any(
                |i| matches!(i, ConfigIssue::UnknownColorScheme(s) if s == "Not A Real Scheme")
            )
        );
    }

    #[test]
    fn validate_catches_ambiguous_chords() {
        use crate::action::{Action, Hotkeys};
        use std::collections::BTreeMap;
        let mut bindings = BTreeMap::new();
        let chord = KeyChord::parse("Ctrl-Shift-X").unwrap();
        bindings.insert(Action::Copy, vec![chord]);
        bindings.insert(Action::Paste, vec![chord]);
        let config = Config {
            hotkeys: Hotkeys::new(bindings),
            ..Config::default()
        };
        let issues = validate(&config);
        assert!(
            issues
                .iter()
                .any(|i| matches!(i, ConfigIssue::AmbiguousChord { .. })),
            "{issues:?}"
        );
    }

    use std::os::unix::fs::PermissionsExt as _;

    /// A tiny unique-per-test temp dir under the system temp dir, removed on drop.
    struct TempDir(PathBuf);
    impl TempDir {
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn tempdir() -> TempDir {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("fogu-core-test-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }
}
