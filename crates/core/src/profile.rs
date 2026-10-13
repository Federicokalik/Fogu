//! Local shell profile: the program, arguments, working directory, extra environment and login flag.

use std::collections::BTreeMap;
use std::path::PathBuf;

/// How to start a local shell in a PTY.
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
