#![expect(clippy::unwrap_used, clippy::indexing_slicing, reason = "tests")]

use std::path::PathBuf;

use super::*;
use crate::config::load;
use crate::config::ssh_opts::SshOpts;
use crate::profile::ProfileKind;

const FIXTURE: &str = "\
# my config
version = 1

# --- work ---
[[profiles]]
id = \"work\"   # keep
name = \"Work\"
type = \"ssh\"
host = \"work.lan\"
port = 2222

[[profiles]]
id = \"play\"
name = \"Play\"
type = \"local\"
env = { A = \"1\" }

# trailing comment
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
    let dir = std::env::temp_dir().join(format!("fogu-edit-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    if let Some(text) = text {
        std::fs::write(dir.join(FILE_NAME), text).unwrap();
    }
    Tmp(dir)
}

fn read(t: &Tmp) -> String {
    std::fs::read_to_string(t.0.join(FILE_NAME)).unwrap()
}

fn id(s: &str) -> ProfileId {
    ProfileId(s.to_owned())
}

/// Applies `edits` to the file and to the loaded config; both must agree afterwards.
fn edit_and_check(t: &Tmp, edits: &[ProfileEdit]) -> Config {
    let (mut config, err) = load(&t.0);
    assert!(err.is_none());
    edit_profiles(&t.0, edits).unwrap();
    for e in edits {
        e.apply(&mut config);
    }
    let (reloaded, err) = load(&t.0);
    assert!(err.is_none(), "{err:?}\n{}", read(t));
    assert_eq!(reloaded, config, "{}", read(t));
    reloaded
}

#[test]
fn duplicate_appends_a_copy_and_keeps_everything_else() {
    let t = tmp(Some(FIXTURE));
    let edit = ProfileEdit::Duplicate {
        from: id("work"),
        id: id("work-copy"),
        name: "Work copy".to_owned(),
    };
    let config = edit_and_check(&t, &[edit]);
    assert_eq!(config.profiles.len(), 3);
    assert_eq!(config.profiles[2].name, "Work copy");
    assert_eq!(config.profiles[2].kind, config.profiles[0].kind);
    let after = read(&t);
    // The original text is untouched; the copy comes after the last profile.
    assert!(
        after.starts_with(&FIXTURE[..FIXTURE.find("# trailing").unwrap()]),
        "{after}"
    );
    assert_eq!(after.matches("# --- work ---").count(), 1, "{after}");
    assert!(
        after.contains("[[profiles]]\nid = \"work-copy\"\nname = \"Work copy\"\ntype = \"ssh\""),
        "{after}"
    );
}

#[test]
fn new_group_and_move_touch_only_those_lines() {
    let t = tmp(Some(FIXTURE));
    let group = Group {
        id: GroupId("servers".to_owned()),
        name: "Servers".to_owned(),
    };
    let edits = [
        ProfileEdit::AddGroup(group.clone()),
        ProfileEdit::SetGroup {
            id: id("play"),
            group: Some(group.id.clone()),
        },
    ];
    let config = edit_and_check(&t, &edits);
    assert_eq!(config.groups, std::slice::from_ref(&group));
    assert_eq!(config.profiles[1].group, Some(group.id));
    let after = read(&t);
    assert!(
        after.contains("[[groups]]\nid = \"servers\"\nname = \"Servers\""),
        "{after}"
    );
    assert_eq!(
        after
            .replace("group = \"servers\"\n", "")
            .lines()
            .filter(|l| !FIXTURE.contains(l))
            .count(),
        3, // [[groups]], id, name
        "{after}"
    );
    // Ungrouping removes the line again.
    let ungroup = ProfileEdit::SetGroup {
        id: id("play"),
        group: None,
    };
    let config = edit_and_check(&t, &[ungroup]);
    assert_eq!(config.profiles[1].group, None);
}

#[test]
fn delete_removes_only_that_profile() {
    let t = tmp(Some(FIXTURE));
    let config = edit_and_check(&t, &[ProfileEdit::Delete(id("play"))]);
    assert_eq!(config.profiles.len(), 1);
    let after = read(&t);
    assert!(!after.contains("Play"), "{after}");
    assert!(
        after.starts_with("# my config\nversion = 1\n\n# --- work ---\n"),
        "{after}"
    );
    assert!(after.contains("# trailing comment"), "{after}");
}

#[test]
fn unknown_profile_or_broken_file_changes_nothing() {
    let t = tmp(Some(FIXTURE));
    let err = edit_profiles(&t.0, &[ProfileEdit::Delete(id("gone"))]).unwrap_err();
    assert!(matches!(err, ConfigError::NoSuchProfile { .. }), "{err}");
    // A later bad edit also cancels an earlier good one.
    let edits = [
        ProfileEdit::Delete(id("work")),
        ProfileEdit::SetGroup {
            id: id("gone"),
            group: None,
        },
    ];
    assert!(edit_profiles(&t.0, &edits).is_err());
    assert_eq!(read(&t), FIXTURE);
    std::fs::write(t.0.join(FILE_NAME), "not toml [[[").unwrap();
    assert!(edit_profiles(&t.0, &[ProfileEdit::Delete(id("work"))]).is_err());
    assert_eq!(read(&t), "not toml [[[");
    assert!(!t.0.join("config.toml.bak").exists());
}

#[test]
fn adding_to_a_missing_file_creates_it() {
    let t = tmp(None);
    let profile = Profile {
        id: id("quick"),
        name: "me@h".to_owned(),
        kind: ProfileKind::Ssh(SshOpts {
            host: Some("h".to_owned()),
            user: Some("me".to_owned()),
            ..SshOpts::default()
        }),
        ..Profile::default_shell()
    };
    let config = edit_and_check(&t, &[ProfileEdit::Add(Box::new(profile.clone()))]);
    assert_eq!(config.profiles, [profile]);
    // Anything else needs an existing file.
    let t = tmp(None);
    assert!(edit_profiles(&t.0, &[ProfileEdit::Delete(id("x"))]).is_err());
}

#[test]
fn unique_ids() {
    let (config, _) = load(&tmp(Some(FIXTURE)).0);
    assert_eq!(unique_profile_id(&config, "work").0, "work-2");
    assert_eq!(unique_profile_id(&config, "new").0, "new");
    let mut config = config;
    config.groups.push(Group {
        id: GroupId("my-work".to_owned()),
        name: String::new(),
    });
    assert_eq!(unique_group_id(&config, "  My  Work! ").0, "my-work-2");
    assert_eq!(unique_group_id(&config, "Ünï").0, "ünï");
    assert_eq!(unique_group_id(&config, "!!!").0, "group");
}
