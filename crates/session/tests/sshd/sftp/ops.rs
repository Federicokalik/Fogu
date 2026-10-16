//! More engine operations against the real `sftp-server`: counting a tree before a delete, the
//! "keep both" policy in both directions, and resuming an upload.

use std::fs;
use std::os::unix::fs::symlink;

use fogu_session::SftpCmd;
use fogu_session::SftpEvent;
use fogu_session::sftp::{ConflictKind, OverwritePolicy, SkipReason};

use super::{Sftp, done_report, mkdir, random_bytes};

#[test]
fn sftp_count_dir_counts_without_following_links() {
    let Some(mut s) = Sftp::start() else { return };
    let tree = mkdir(s.dir().join("count"));
    fs::write(tree.join("f1"), "x").unwrap();
    fs::write(tree.join("f2"), "x").unwrap();
    mkdir(tree.join("sub"));
    fs::write(tree.join("sub/f3"), "x").unwrap();
    let outside = mkdir(s.dir().join("outside"));
    for i in 0..5 {
        fs::write(outside.join(i.to_string()), "x").unwrap();
    }
    symlink(&outside, tree.join("link")).unwrap();
    let path = tree.display().to_string();
    match s.request(SftpCmd::CountDir(path.clone())) {
        // f1, f2, sub, sub/f3, link — never what the link points to.
        SftpEvent::Counted { entries, .. } => assert_eq!(entries, 5),
        other => panic!("{other:?}"),
    }
    // A symlink root is refused, like the recursive delete.
    let link = tree.join("link").display().to_string();
    assert!(matches!(
        s.request(SftpCmd::CountDir(link)),
        SftpEvent::Error { .. }
    ));
    s.ok(SftpCmd::RemoveDirRecursive(path));
    assert!(!tree.exists());
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 5);
}

#[test]
fn sftp_keep_both_picks_a_free_name_in_both_directions() {
    let Some(mut s) = Sftp::start() else { return };
    let remote = mkdir(s.dir().join("kb-remote"));
    let local = mkdir(s.dir().join("kb-local"));
    fs::write(remote.join("a.txt"), "new").unwrap();
    fs::write(local.join("a.txt"), "old").unwrap();
    // `a (1).txt` is taken by a stale partial: the next free name is used.
    fs::write(local.join("a (1).txt.part"), "stale").unwrap();
    let file = remote.join("a.txt");
    let report = done_report(s.download(&[&file], &local, OverwritePolicy::KeepBoth, false));
    assert_eq!(report.files_done, 1, "{report:?}");
    assert_eq!(fs::read_to_string(local.join("a.txt")).unwrap(), "old");
    assert_eq!(fs::read_to_string(local.join("a (2).txt")).unwrap(), "new");
    assert_eq!(
        fs::read_to_string(local.join("a (1).txt.part")).unwrap(),
        "stale"
    );

    let up = s.transfer(SftpCmd::Upload {
        local: vec![local.join("a.txt")],
        remote_dir: remote.display().to_string(),
        overwrite: OverwritePolicy::KeepBoth,
        resume: false,
    });
    assert_eq!(done_report(up).files_done, 1);
    assert_eq!(fs::read_to_string(remote.join("a.txt")).unwrap(), "new");
    assert_eq!(fs::read_to_string(remote.join("a (1).txt")).unwrap(), "old");
}

#[test]
fn sftp_upload_resume_appends_to_a_shorter_remote_file() {
    let Some(mut s) = Sftp::start() else { return };
    let data = random_bytes(3 << 20);
    let src = mkdir(s.dir().join("up-src"));
    fs::write(src.join("big.bin"), &data).unwrap();
    let remote = mkdir(s.dir().join("up-dst"));
    let kept = 1_000_000;
    // Written after the local file: plausible as an interrupted upload.
    fs::write(remote.join("big.bin"), &data[..kept]).unwrap();
    let upload = |resume| SftpCmd::Upload {
        local: vec![src.join("big.bin")],
        remote_dir: remote.display().to_string(),
        overwrite: OverwritePolicy::Ask,
        resume,
    };
    let report = done_report(s.transfer(upload(false)));
    assert_eq!(
        report.skipped[0].reason,
        SkipReason::Exists(ConflictKind::Exists)
    );
    assert!(s.seen.iter().any(|e| matches!(
        e,
        SftpEvent::Conflict {
            resumable: true,
            ..
        }
    )));
    let report = done_report(s.transfer(upload(true)));
    assert_eq!(
        (report.files_done, report.bytes),
        (1, (data.len() - kept) as u64)
    );
    assert!(
        fs::read(remote.join("big.bin")).unwrap() == data,
        "resumed upload differs"
    );

    // A remote file longer than the local one is never "resumed".
    fs::write(remote.join("big.bin"), [&data[..], b"extra"].concat()).unwrap();
    let report = done_report(s.transfer(upload(true)));
    assert_eq!(report.files_done, 0);
    assert!(!s.seen.iter().rev().take(3).any(|e| matches!(
        e,
        SftpEvent::Conflict {
            resumable: true,
            ..
        }
    )));
}
