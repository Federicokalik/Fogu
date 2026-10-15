//! Config file I/O: XDG path, atomic save, load with graceful fallback, and semantic validation.

use std::collections::{HashMap, HashSet};
use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};

use super::Config;
use crate::action::Action;
use crate::keychord::KeyChord;
use crate::profile::ProfileId;
use crate::scheme::ColorScheme;

/// The config file's name inside [`config_dir`].
const FILE_NAME: &str = "config.toml";
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
    /// The config could not be serialised back to TOML (should not happen for a valid `Config`,
    /// but a library bug here must not panic).
    #[error("failed to serialise config: {0}")]
    Serialize(String),
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

/// Saves `config` to `dir/config.toml` atomically (temp file, `fsync`, rename), mode `0600`,
/// keeping the previous file as `config.toml.bak`.
pub fn save(config: &Config, dir: &Path) -> Result<(), ConfigError> {
    crate::fsutil::create_private_dir(dir).map_err(|err| ConfigError::io(dir, err))?;
    let text =
        toml::to_string_pretty(config).map_err(|err| ConfigError::Serialize(err.to_string()))?;
    let dest = dir.join(FILE_NAME);
    let tmp = dir.join(format!("{FILE_NAME}.tmp"));
    write_new_file(&tmp, text.as_bytes())?;
    let bak = dir.join(format!("{FILE_NAME}.bak"));
    match std::fs::rename(&dest, &bak) {
        Ok(()) => {}
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

    #[test]
    fn save_then_load_round_trips_with_0600_and_a_backup() {
        let tmp = tempdir();
        let mut config = Config::default();
        config.appearance.font_size = 20.0;
        save(&config, tmp.path()).unwrap();

        let path = tmp.path().join(FILE_NAME);
        let perms = std::fs::metadata(&path).unwrap().permissions();
        assert_eq!(perms.mode() & 0o777, FILE_MODE);

        let (loaded, err) = load(tmp.path());
        assert!(err.is_none());
        assert_eq!(loaded, config);

        // A second save keeps the previous content as `.bak`.
        config.appearance.font_size = 22.0;
        save(&config, tmp.path()).unwrap();
        let bak = std::fs::read_to_string(tmp.path().join(format!("{FILE_NAME}.bak"))).unwrap();
        assert!(bak.contains("font_size = 20"), "{bak}");
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
