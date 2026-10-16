//! The read-only "SSH config" source of the sidebar and picker: `~/.ssh/config` hosts as
//! profiles (`fogu_core::ssh_config`), re-read at startup and whenever the sidebar or the
//! picker opens if a file it read changed. Off with `ssh.read_ssh_config = false`.

use std::path::PathBuf;

use fogu_core::ssh_config::{Env, ID_PREFIX, Issue, SshConfig};
use fogu_core::{
    EffectiveProfile, EffectiveSsh, Profile, ProfileId, ProfileKind, SshOpts, resolve,
};
use fogu_term::sanitize_title;
use iced::Task;

use crate::app::App;
use crate::message::Message;
use crate::notifications::{Level, Notification};

/// Issues listed in the toast (the rest is counted).
const SHOWN_ISSUES: usize = 3;

/// Where the hosts come from and what was read last.
#[derive(Debug, Default)]
pub struct SshHosts {
    /// `None` = no source (no `$HOME`, test builds).
    pub source: Option<(Env, PathBuf)>,
    loaded: Option<SshConfig>,
    /// One profile per plain `Host` alias.
    pub profiles: Vec<Profile>,
}

impl SshHosts {
    /// `~/.ssh/config` of this process's user.
    pub fn from_process() -> Self {
        Self::at(Env::from_process().map(|env| {
            let path = env.user_config();
            (env, path)
        }))
    }

    /// A source at an explicit place (tests).
    pub fn at(source: Option<(Env, PathBuf)>) -> Self {
        Self {
            source,
            ..Self::default()
        }
    }

    /// Re-reads the files if they changed (or were never read); returns the new issues as
    /// `~/…:line: message` texts, so the caller can show them once per change.
    pub fn refresh(&mut self) -> Vec<String> {
        let Some((env, path)) = &self.source else {
            return Vec::new();
        };
        if self.loaded.as_ref().is_some_and(|l| !l.changed()) {
            return Vec::new();
        }
        let loaded = SshConfig::load(path, env);
        self.profiles = loaded.profiles(env);
        let issues = loaded.issues.iter().map(|i| tilde(i, env)).collect();
        self.loaded = Some(loaded);
        issues
    }

    /// The options of the jump host `spec` names (`ssh-config:<spec>` ids; see
    /// `SshConfig::jump_opts`). `Err` when the file isn't loaded or `spec` is unusable.
    pub fn jump_opts(&self, spec: &str) -> Result<SshOpts, String> {
        match (&self.source, &self.loaded) {
            (Some((env, _)), Some(loaded)) => loaded.jump_opts(spec, env),
            _ => Err("~/.ssh/config is not loaded".to_owned()),
        }
    }
}

/// `issue` with the home directory shown as `~`.
fn tilde(issue: &Issue, env: &Env) -> String {
    match issue.file.strip_prefix(&env.home) {
        Ok(rest) => Issue {
            file: PathBuf::from("~").join(rest),
            ..issue.clone()
        }
        .to_string(),
        Err(_) => issue.to_string(),
    }
}

/// "~/.ssh/config" warning toast for `issues` (`None` when there are none).
pub fn issues_note(issues: &[String]) -> Option<Notification> {
    if issues.is_empty() {
        return None;
    }
    let mut lines: Vec<String> = issues
        .iter()
        .take(SHOWN_ISSUES)
        .map(|i| sanitize_title(i))
        .collect();
    if issues.len() > SHOWN_ISSUES {
        lines.push(format!("…and {} more", issues.len() - SHOWN_ISSUES));
    }
    Some(Notification::new(Level::Warn, "~/.ssh/config").detail(lines.join("  ·  ")))
}

impl App {
    /// Re-reads `~/.ssh/config` if enabled and changed; problems become a warning toast.
    pub(crate) fn refresh_ssh_hosts(&mut self) -> Task<Message> {
        if !self.config.ssh.read_ssh_config {
            self.profiles.ssh_hosts.profiles.clear();
            return Task::none();
        }
        match issues_note(&self.profiles.ssh_hosts.refresh()) {
            Some(note) => self.notify(note),
            None => Task::none(),
        }
    }
}

impl App {
    /// Fills `ssh.jump` (the `jump_host` chain) from the stored profiles and `~/.ssh/config`
    /// hosts; an unknown, non-SSH or looping jump host leaves a reason the session reports
    /// instead of connecting.
    pub(crate) fn resolve_jumps(&self, id: &ProfileId, ssh: &mut EffectiveSsh) {
        let lookup = |hop: &ProfileId| -> Result<EffectiveSsh, String> {
            let stored = self.config.profiles.iter().find(|p| &p.id == hop);
            let profile = match (stored, hop.0.strip_prefix(ID_PREFIX)) {
                (Some(profile), _) => profile.clone(),
                (None, Some(_)) if !self.config.ssh.read_ssh_config => {
                    return Err("~/.ssh/config is off (ssh.read_ssh_config = false)".to_owned());
                }
                (None, Some(spec)) => Profile {
                    id: hop.clone(),
                    name: spec.to_owned(),
                    kind: ProfileKind::Ssh(self.profiles.ssh_hosts.jump_opts(spec)?),
                    ..Profile::default_shell()
                },
                (None, None) => return Err(format!("no profile with id {}", hop.0)),
            };
            match resolve(&profile, &self.config.profile_defaults) {
                EffectiveProfile::Ssh(hop) => Ok(hop),
                EffectiveProfile::Local(_) => {
                    Err(format!("{} is not an SSH profile", profile.name))
                }
            }
        };
        fogu_core::resolve_jumps(id, ssh, lookup);
    }
}

#[cfg(test)]
#[path = "ssh_config_tests.rs"]
mod tests;
