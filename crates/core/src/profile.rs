//! Profiles: `Profile` (common fields) + `ProfileKind` (per-kind options), profile defaults, and
//! the one `resolve` merge function. Only local and SSH profiles exist; serial/telnet kinds are
//! not modelled.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::scheme::RgbColor;

/// How to start a local shell in a PTY. The concrete, fully-resolved form of a local profile
/// produced by [`resolve`] from a stored [`LocalOpts`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LocalProfile {
    /// Program to run; `None` = the user's default shell (`$SHELL` → passwd → `/bin/sh`).
    pub program: Option<String>,
    /// Arguments after argv0. Stored separately from `program` (never `sh -c "<string>"`).
    pub args: Vec<String>,
    /// Working directory; `None` = `$HOME`.
    pub cwd: Option<PathBuf>,
    /// Extra environment; values support `${VAR}` expansion from the parent environment.
    pub env: BTreeMap<String, String>,
    /// Start as a login shell (argv0 prefixed with `-`, e.g. `-zsh`).
    pub login: bool,
}

/// What to show the user via OSC 52 (clipboard write) and whether to allow it. The UI does not
/// apply this yet: `ui/src/app.rs` always drops `SessionOutput::ClipboardStore` (equivalent to
/// `Deny`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Osc52Policy {
    /// Prompt the user each time.
    #[default]
    Ask,
    /// Always write to the clipboard.
    Allow,
    /// Always ignore the request.
    Deny,
}

/// Stored (partial) options for a local profile: every field is `Option<T>` so a profile can
/// leave a field unset and inherit it from [`ProfileDefaults::local`], which in turn inherits
/// from [`LocalProfile::default`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalOpts {
    /// See [`LocalProfile::program`].
    pub program: Option<String>,
    /// See [`LocalProfile::args`].
    pub args: Option<Vec<String>>,
    /// See [`LocalProfile::cwd`].
    pub cwd: Option<PathBuf>,
    /// See [`LocalProfile::env`].
    pub env: Option<BTreeMap<String, String>>,
    /// See [`LocalProfile::login`].
    pub login: Option<bool>,
    /// Per-profile override of `terminal.osc52_write`.
    pub osc52_write: Option<Osc52Policy>,
}

/// Stored (partial) SSH options. Only `host`/`port`/`user` exist so far; connection, auth and
/// host-key fields are not modelled yet.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SshOpts {
    /// Hostname or address; required to actually connect (no builtin default).
    pub host: Option<String>,
    /// TCP port; defaults to 22.
    pub port: Option<u16>,
    /// Remote username; `None` lets the SSH client decide (local user name).
    pub user: Option<String>,
    /// Per-profile override of `terminal.osc52_write`.
    pub osc52_write: Option<Osc52Policy>,
}

/// Per-kind profile options (an enum, not trait objects — the set of kinds is
/// closed and known at compile time).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ProfileKind {
    /// A local shell/command.
    Local(LocalOpts),
    /// An SSH connection.
    Ssh(SshOpts),
}

/// What happens to the tab when the session ends. `Auto`/`Reconnect` do not exist yet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionEndBehavior {
    /// Leave the tab open, showing the exit status.
    #[default]
    Keep,
    /// Close the tab immediately.
    Close,
}

/// A profile identifier. Newtype so profile/group ids are never confused with plain strings.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProfileId(pub String);

/// A profile group identifier.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GroupId(pub String);

/// A profile group (just the data model for now).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    /// Stable identifier.
    pub id: GroupId,
    /// Display name.
    pub name: String,
}

/// One stored profile: fields common to every kind, plus the kind-specific options.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    /// Stable identifier (referenced by `Config.default_profile`, tab recovery, `jumpHost`...).
    pub id: ProfileId,
    /// Display name.
    pub name: String,
    /// Group this profile belongs to, if any.
    #[serde(default)]
    pub group: Option<GroupId>,
    /// Icon name.
    #[serde(default)]
    pub icon: Option<String>,
    /// Tab colour; `"#RRGGBB"` in TOML.
    #[serde(default)]
    pub color: Option<RgbColor>,
    /// Suppress OSC-2/0/title-driven tab titles for this profile.
    #[serde(default)]
    pub disable_dynamic_title: bool,
    /// See [`SessionEndBehavior`].
    #[serde(default)]
    pub behavior_on_session_end: SessionEndBehavior,
    /// Kind-specific options, flattened into the same TOML table (`type = "local"` / `"ssh"`).
    #[serde(flatten)]
    pub kind: ProfileKind,
}

impl Profile {
    /// The built-in "Default shell" profile: generated at runtime, never stored.
    #[must_use]
    pub fn default_shell() -> Self {
        Self {
            id: ProfileId("builtin:local:default".to_owned()),
            name: "Default shell".to_owned(),
            group: None,
            icon: None,
            color: None,
            disable_dynamic_title: false,
            behavior_on_session_end: SessionEndBehavior::default(),
            kind: ProfileKind::Local(LocalOpts::default()),
        }
    }
}

/// Partial defaults applied under every profile of a kind, themselves falling back to
/// `LocalProfile::default`/builtin SSH defaults.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProfileDefaults {
    /// Defaults applied to every `Local` profile.
    pub local: LocalOpts,
    /// Defaults applied to every `Ssh` profile.
    pub ssh: SshOpts,
}

/// A resolved SSH profile. Minimal for now: just enough for a `QuickConnect`-style address;
/// auth, host keys and jump hosts are not modelled yet.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EffectiveSsh {
    /// Hostname or address.
    pub host: String,
    /// TCP port (default 22).
    pub port: u16,
    /// Remote username, if set.
    pub user: Option<String>,
}

/// The default SSH port.
const DEFAULT_SSH_PORT: u16 = 22;

/// A profile with every option resolved to a concrete value, ready to open a session.
#[derive(Clone, Debug, PartialEq)]
pub enum EffectiveProfile {
    /// A local shell/command, ready for `fogu_session::open`.
    Local(LocalProfile),
    /// An SSH connection (minimal fields).
    Ssh(EffectiveSsh),
}

/// The one profile-defaults merge function:
/// `profile.or(defaults[kind]).or(builtin_default)`.
#[must_use]
pub fn resolve(profile: &Profile, defaults: &ProfileDefaults) -> EffectiveProfile {
    match &profile.kind {
        ProfileKind::Local(opts) => EffectiveProfile::Local(resolve_local(opts, &defaults.local)),
        ProfileKind::Ssh(opts) => EffectiveProfile::Ssh(resolve_ssh(opts, &defaults.ssh)),
    }
}

fn resolve_local(opts: &LocalOpts, defaults: &LocalOpts) -> LocalProfile {
    LocalProfile {
        program: opts.program.clone().or_else(|| defaults.program.clone()),
        args: opts
            .args
            .clone()
            .or_else(|| defaults.args.clone())
            .unwrap_or_default(),
        cwd: opts.cwd.clone().or_else(|| defaults.cwd.clone()),
        env: opts
            .env
            .clone()
            .or_else(|| defaults.env.clone())
            .unwrap_or_default(),
        login: opts.login.or(defaults.login).unwrap_or(false),
    }
}

fn resolve_ssh(opts: &SshOpts, defaults: &SshOpts) -> EffectiveSsh {
    EffectiveSsh {
        host: opts
            .host
            .clone()
            .or_else(|| defaults.host.clone())
            .unwrap_or_default(),
        port: opts.port.or(defaults.port).unwrap_or(DEFAULT_SSH_PORT),
        user: opts.user.clone().or_else(|| defaults.user.clone()),
    }
}

/// The effective `terminal.osc52_write` for a profile: its own override, else the global default.
#[must_use]
pub fn effective_osc52_write(profile: &Profile, global_default: Osc52Policy) -> Osc52Policy {
    let override_ = match &profile.kind {
        ProfileKind::Local(opts) => opts.osc52_write,
        ProfileKind::Ssh(opts) => opts.osc52_write,
    };
    override_.unwrap_or(global_default)
}

#[cfg(test)]
mod tests {
    #![expect(clippy::unwrap_used, clippy::panic, reason = "tests")]

    use super::*;

    #[test]
    fn resolve_local_merges_profile_over_defaults_over_builtin() {
        let defaults = ProfileDefaults {
            local: LocalOpts {
                program: Some("/bin/bash".to_owned()),
                login: Some(true),
                ..LocalOpts::default()
            },
            ssh: SshOpts::default(),
        };
        // Profile overrides `program`, inherits `login` from defaults, `args`/`env`/`cwd` fall
        // back to `LocalProfile::default`.
        let profile = Profile {
            kind: ProfileKind::Local(LocalOpts {
                program: Some("/usr/bin/fish".to_owned()),
                ..LocalOpts::default()
            }),
            ..Profile::default_shell()
        };
        let EffectiveProfile::Local(effective) = resolve(&profile, &defaults) else {
            panic!("expected a local profile");
        };
        assert_eq!(effective.program, Some("/usr/bin/fish".to_owned()));
        assert!(effective.login);
        assert_eq!(effective.args, Vec::<String>::new());
        assert_eq!(effective.cwd, None);
    }

    #[test]
    fn resolve_local_with_nothing_set_is_the_builtin_default() {
        let profile = Profile::default_shell();
        let EffectiveProfile::Local(effective) = resolve(&profile, &ProfileDefaults::default())
        else {
            panic!("expected a local profile");
        };
        assert_eq!(effective, LocalProfile::default());
    }

    #[test]
    fn resolve_ssh_defaults_the_port() {
        let profile = Profile {
            kind: ProfileKind::Ssh(SshOpts {
                host: Some("example.com".to_owned()),
                ..SshOpts::default()
            }),
            ..Profile::default_shell()
        };
        let EffectiveProfile::Ssh(effective) = resolve(&profile, &ProfileDefaults::default())
        else {
            panic!("expected an ssh profile");
        };
        assert_eq!(effective.host, "example.com");
        assert_eq!(effective.port, 22);
        assert_eq!(effective.user, None);
    }

    #[test]
    fn profile_osc52_override_wins_over_global_default() {
        let mut profile = Profile::default_shell();
        profile.kind = ProfileKind::Local(LocalOpts {
            osc52_write: Some(Osc52Policy::Allow),
            ..LocalOpts::default()
        });
        assert_eq!(
            effective_osc52_write(&profile, Osc52Policy::Deny),
            Osc52Policy::Allow
        );
        let unset = Profile::default_shell();
        assert_eq!(
            effective_osc52_write(&unset, Osc52Policy::Deny),
            Osc52Policy::Deny
        );
    }

    #[test]
    fn profile_round_trips_through_toml() {
        let profile = Profile {
            color: Some(RgbColor::from_hex(0x12_3456)),
            kind: ProfileKind::Ssh(SshOpts {
                host: Some("example.com".to_owned()),
                port: Some(2222),
                user: Some("alice".to_owned()),
                osc52_write: None,
            }),
            ..Profile::default_shell()
        };
        let text = toml::to_string(&profile).unwrap();
        assert!(text.contains("type = \"ssh\""));
        let back: Profile = toml::from_str(&text).unwrap();
        assert_eq!(back, profile);
    }
}
