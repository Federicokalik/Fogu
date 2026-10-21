#![expect(clippy::unwrap_used, reason = "tests")]

use std::path::PathBuf;

use super::*;

const FIXTURE: &str = "\
# my config
version = 1

[terminal]
scrollback_lines = 10_000   # plenty
copy_on_select = true

[[profiles]]
id = \"work\"
name = \"Work\"
type = \"local\"
";

struct Tmp(PathBuf);
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tmp(text: Option<&str>) -> Tmp {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("fogu-section-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    if let Some(text) = text {
        std::fs::write(dir.join(FILE_NAME), text).unwrap();
    }
    Tmp(dir)
}

fn read(t: &Tmp) -> String {
    std::fs::read_to_string(t.0.join(FILE_NAME)).unwrap()
}

#[test]
fn a_changed_key_keeps_its_comment_and_the_rest_of_the_file() {
    let t = tmp(Some(FIXTURE));
    set_keys(
        &t.0,
        &["terminal"],
        &[KeyEdit::set("scrollback_lines", TomlValue::Int(500))],
    )
    .unwrap();
    assert_eq!(
        read(&t),
        FIXTURE.replace("10_000   # plenty", "500   # plenty")
    );
}

#[test]
fn an_unchanged_value_writes_nothing() {
    let t = tmp(Some(FIXTURE));
    set_keys(
        &t.0,
        &["terminal"],
        &[KeyEdit::set("scrollback_lines", TomlValue::Int(10_000))],
    )
    .unwrap();
    assert_eq!(read(&t), FIXTURE);
    assert!(!t.0.join(format!("{FILE_NAME}.bak")).exists());
}

#[test]
fn nested_and_root_keys_are_created_and_removed() {
    let t = tmp(None);
    set_keys(
        &t.0,
        &["profile_defaults", "ssh"],
        &[KeyEdit::set("user", "alice")],
    )
    .unwrap();
    set_keys(&t.0, &[], &[KeyEdit::set("default_profile", "work")]).unwrap();
    let text = read(&t);
    assert!(
        text.contains("[profile_defaults.ssh]\nuser = \"alice\""),
        "{text}"
    );
    assert!(!text.contains("[profile_defaults]\n"), "{text}");
    let config: crate::Config = toml::from_str(&text).unwrap();
    assert_eq!(config.profile_defaults.ssh.user.as_deref(), Some("alice"));
    assert_eq!(config.default_profile.map(|p| p.0).as_deref(), Some("work"));

    set_keys(&t.0, &[], &[KeyEdit::remove("default_profile")]).unwrap();
    assert!(!read(&t).contains("default_profile"));
    set_keys(
        &t.0,
        &["profile_defaults", "ssh"],
        &[KeyEdit::remove("user")],
    )
    .unwrap();
    assert!(!read(&t).contains("[profile_defaults.ssh]"), "{}", read(&t));
}

#[test]
fn hotkeys_keep_unchanged_entries_and_drop_reset_ones() {
    use std::collections::BTreeMap;

    use crate::{Action, KeyChord};
    let t = tmp(Some(
        "[hotkeys]\ncopy = [\"ctrl-shift-x\"]   # mine\npaste = []\nnot-an-action = 1\n",
    ));
    let chord = |s| KeyChord::parse(s).unwrap();
    let overrides = BTreeMap::from([
        (Action::Copy, vec![chord("Ctrl-Shift-X")]),
        (Action::Find, vec![chord("Ctrl-Alt-F")]),
    ]);
    set_hotkeys(&t.0, &overrides).unwrap();
    assert_eq!(
        read(&t),
        "[hotkeys]\ncopy = [\"ctrl-shift-x\"]   # mine\nnot-an-action = 1\nfind = [\"Ctrl-Alt-F\"]\n"
    );
}

#[test]
fn a_file_that_does_not_parse_is_left_alone() {
    let t = tmp(Some("not valid [[["));
    let err = set_keys(&t.0, &["terminal"], &[KeyEdit::set("x", true)]).unwrap_err();
    assert!(matches!(err, ConfigError::Parse { .. }), "{err}");
    assert_eq!(read(&t), "not valid [[[");
}
