#![expect(clippy::unwrap_used, reason = "tests")]

use super::*;

#[test]
fn candidates_order_env_systemd_gcr_then_dir() {
    let got = candidates(
        Some("/env.sock".into()),
        Some(Path::new("/run/user/1000")),
        vec![PathBuf::from("/h/.ssh/agent/s.1")],
    );
    let want: Vec<PathBuf> = [
        "/env.sock",
        "/run/user/1000/ssh-agent.socket",
        "/run/user/1000/gcr/ssh",
        "/h/.ssh/agent/s.1",
    ]
    .iter()
    .map(PathBuf::from)
    .collect();
    assert_eq!(got, want);
    assert_eq!(
        candidates(Some(OsString::new()), None, Vec::new()),
        Vec::<PathBuf>::new()
    );
}

#[test]
fn owned_sockets_needs_a_private_dir_of_ours() {
    let dir = std::env::temp_dir().join(format!("fogu-agent-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let sock = dir.join("s.1");
    let _listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
    std::fs::write(dir.join("not-a-socket"), b"x").unwrap();
    let uid = std::fs::metadata(&dir).unwrap().uid();
    assert_eq!(owned_sockets(&dir, uid), vec![sock]);
    assert!(
        owned_sockets(&dir, uid + 1).is_empty(),
        "someone else's dir"
    );
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
    assert!(owned_sockets(&dir, uid).is_empty(), "world-writable dir");
    let _ = std::fs::remove_dir_all(&dir);
}
