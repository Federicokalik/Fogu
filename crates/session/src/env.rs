//! Child environment for local shells.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};

/// Variables that must not leak from our process into user shells.
const REMOVED: &[&str] = &[
    "APPDIR",
    "APPIMAGE",
    "ARGV0",
    "DESKTOP_STARTUP_ID",
    "XDG_ACTIVATION_TOKEN",
    "RUST_LOG",
    "RUST_BACKTRACE",
];

/// Library overrides set by the `AppImage` runtime; removed only when running as an `AppImage`.
const APPIMAGE_LIB_VARS: &[&str] = &["LD_LIBRARY_PATH", "LD_PRELOAD"];

/// Builds the child environment: `parent` minus [`REMOVED`] (plus `LD_*` under an `AppImage`),
/// terminal identification variables, then `profile_env` with `${VAR}` expanded against that
/// result (unknown variables expand to nothing).
pub fn build_env<I>(
    parent: I,
    profile_env: &BTreeMap<String, String>,
) -> BTreeMap<OsString, OsString>
where
    I: IntoIterator<Item = (OsString, OsString)>,
{
    let mut env: BTreeMap<OsString, OsString> = parent.into_iter().collect();
    if env.contains_key(OsStr::new("APPIMAGE")) {
        for key in APPIMAGE_LIB_VARS {
            env.remove(OsStr::new(key));
        }
    }
    for key in REMOVED {
        env.remove(OsStr::new(key));
    }
    for (key, value) in [
        ("TERM", "xterm-256color"),
        ("COLORTERM", "truecolor"),
        ("TERM_PROGRAM", "fogu"),
        ("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION")),
    ] {
        env.insert(key.into(), value.into());
    }
    let expanded: Vec<(OsString, OsString)> = profile_env
        .iter()
        .map(|(key, value)| (key.into(), expand(value, &env).into()))
        .collect();
    env.extend(expanded);
    env
}

/// Replaces every `${NAME}` in `value` with `NAME` from `env`; an unterminated `${` is literal.
fn expand(value: &str, env: &BTreeMap<OsString, OsString>) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find("${") {
        let (before, tail) = rest.split_at(start);
        out.push_str(before);
        let Some(end) = tail.find('}') else {
            out.push_str(tail);
            return out;
        };
        let name = tail.get(2..end).unwrap_or_default();
        if let Some(found) = env.get(OsStr::new(name)) {
            out.push_str(&found.to_string_lossy());
        }
        rest = tail.get(end + 1..).unwrap_or_default();
    }
    out.push_str(rest);
    out
}
