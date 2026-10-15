//! Transfers: streamed upload/download, conflicts, symlink rules, cancel, resume, local symlink
//! refusal.

use std::fs;
use std::os::unix::fs::{PermissionsExt as _, symlink};
use std::path::{Path, PathBuf};

use fogu_session::sftp::{ConflictKind, OverwritePolicy, SftpOptions, SkipReason};
use fogu_session::{SftpCmd, SftpEvent};

use super::{Sftp, answer_id, done_report, mkdir, random_bytes};

#[test]
fn sftp_upload_then_download_5_mib_is_identical() {
    let Some(mut s) = Sftp::start() else { return };
    let data = random_bytes(5 * 1024 * 1024 + 123);
    let src = mkdir(s.dir().join("src"));
    fs::write(src.join("blob.bin"), &data).unwrap();
    fs::set_permissions(src.join("blob.bin"), fs::Permissions::from_mode(0o4755)).unwrap();
    let remote = mkdir(s.dir().join("remote"));
    let up = s.transfer(SftpCmd::Upload {
        local: vec![src.join("blob.bin")],
        remote_dir: remote.display().to_string(),
        overwrite: OverwritePolicy::Ask,
    });
    let report = done_report(up);
    assert_eq!((report.files_done, report.bytes), (1, data.len() as u64));
    assert_eq!(fs::read(remote.join("blob.bin")).unwrap(), data);
    let mode = fs::metadata(remote.join("blob.bin"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o7000, 0, "setuid never copied");

    let local = mkdir(s.dir().join("local"));
    let report = done_report(s.download(
        &[&remote.join("blob.bin")],
        &local,
        OverwritePolicy::Ask,
        false,
    ));
    assert_eq!((report.files_done, report.bytes), (1, data.len() as u64));
    assert!(
        fs::read(local.join("blob.bin")).unwrap() == data,
        "downloaded bytes differ"
    );
    let mode = fs::metadata(local.join("blob.bin"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o7000, 0);
    assert_eq!(mode & 0o100, 0o100, "x bit kept");
    assert!(!local.join("blob.bin.part").exists());
    assert!(
        s.seen
            .iter()
            .any(|e| matches!(e, SftpEvent::TransferQueued { files: 1, .. }))
    );

    // No silent overwrite: Ask → Conflict + skipped; Skip → skipped; Overwrite → replaced.
    fs::write(local.join("blob.bin"), "local edit").unwrap();
    let report = done_report(s.download(
        &[&remote.join("blob.bin")],
        &local,
        OverwritePolicy::Ask,
        false,
    ));
    assert_eq!(report.files_done, 0);
    assert_eq!(
        report.skipped[0].reason,
        SkipReason::Exists(ConflictKind::Exists)
    );
    assert!(s.seen.iter().any(|e| matches!(e, SftpEvent::Conflict { kind: ConflictKind::Exists, path, .. } if path == "blob.bin")));
    assert_eq!(
        fs::read_to_string(local.join("blob.bin")).unwrap(),
        "local edit"
    );
    let report = done_report(s.download(
        &[&remote.join("blob.bin")],
        &local,
        OverwritePolicy::Overwrite,
        false,
    ));
    assert_eq!(report.files_done, 1);
    assert!(
        fs::read(local.join("blob.bin")).unwrap() == data,
        "overwritten bytes differ"
    );
    // Upload conflicts the same way.
    let up = s.transfer(SftpCmd::Upload {
        local: vec![src.join("blob.bin")],
        remote_dir: remote.display().to_string(),
        overwrite: OverwritePolicy::Skip,
    });
    assert_eq!(
        done_report(up).skipped[0].reason,
        SkipReason::Exists(ConflictKind::Exists)
    );
}

#[test]
fn sftp_recursive_download_skips_symlink_loops_and_reports_them() {
    let Some(mut s) = Sftp::start() else { return };
    let tree = mkdir(s.dir().join("tree"));
    fs::write(tree.join("a.txt"), "aaa").unwrap();
    mkdir(tree.join("sub"));
    fs::write(tree.join("sub/b.txt"), "bbb").unwrap();
    symlink("..", tree.join("sub/loop")).unwrap();
    symlink("a.txt", tree.join("alias.txt")).unwrap();
    symlink("/nowhere", tree.join("dangling")).unwrap();
    let local = mkdir(s.dir().join("got"));
    let report = done_report(s.download(&[&tree], &local, OverwritePolicy::Ask, false));
    assert_eq!(report.files_done, 3, "{report:?}");
    assert_eq!(report.dirs_created, 2);
    assert_eq!(fs::read_to_string(local.join("tree/a.txt")).unwrap(), "aaa");
    assert_eq!(
        fs::read_to_string(local.join("tree/sub/b.txt")).unwrap(),
        "bbb"
    );
    // A link to a regular file is downloaded as the target's content, never as a link.
    let alias = local.join("tree/alias.txt");
    assert!(fs::symlink_metadata(&alias).unwrap().file_type().is_file());
    assert_eq!(fs::read_to_string(alias).unwrap(), "aaa");
    let mut skipped: Vec<_> = report
        .skipped
        .iter()
        .map(|k| (k.path.as_str(), k.reason))
        .collect();
    skipped.sort_by_key(|k| k.0);
    assert_eq!(
        skipped,
        [
            ("tree/dangling", SkipReason::SymlinkNotFile),
            ("tree/sub/loop", SkipReason::SymlinkToDir)
        ]
    );
    assert!(!local.join("tree/sub/loop").exists());
}

/// A large sparse remote file (fast to create, slow enough to cancel mid-way).
fn big_remote(s: &Sftp, len: u64) -> PathBuf {
    let dir = mkdir(s.dir().join("big"));
    let path = dir.join("big.bin");
    let file = fs::File::create(&path).unwrap();
    file.set_len(len).unwrap();
    // Some non-zero bytes so a resume that misplaces data is caught.
    let mut file = file;
    for at in [0, len / 3, len - 7] {
        std::io::Seek::seek(&mut file, std::io::SeekFrom::Start(at)).unwrap();
        std::io::Write::write_all(&mut file, b"MARKER!").unwrap();
    }
    path
}

/// Starts a download of `remote` and cancels it after the first progress event.
fn cancel_mid_way(s: &mut Sftp, remote: &Path, local: &Path, resume: bool) -> SftpEvent {
    let id = s.send(SftpCmd::Download {
        remote: vec![remote.display().to_string()],
        local_dir: local.to_owned(),
        overwrite: OverwritePolicy::Ask,
        resume,
    });
    let first =
        s.wait(|e| matches!(e, SftpEvent::TransferProgress { .. }) || answer_id(e) == Some(id));
    assert!(
        matches!(first, SftpEvent::TransferProgress { .. }),
        "finished before it could be cancelled: {first:?}"
    );
    s.send(SftpCmd::Cancel(id));
    s.wait(|e| answer_id(e) == Some(id) && !matches!(e, SftpEvent::Done(_)))
}

#[test]
fn sftp_cancel_mid_transfer_removes_the_part_file() {
    let Some(mut s) = Sftp::start() else { return };
    let remote = big_remote(&s, 512 << 20);
    let local = mkdir(s.dir().join("cancelled"));
    let end = cancel_mid_way(&mut s, &remote, &local, false);
    assert!(
        matches!(end, SftpEvent::TransferCancelled { .. }),
        "{end:?}"
    );
    let left: Vec<_> = fs::read_dir(&local)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(left.is_empty(), "left behind: {left:?}");
}

#[test]
fn sftp_resume_completes_with_identical_content() {
    let Some(mut s) = Sftp::start() else { return };
    let len = 512 << 20;
    let remote = big_remote(&s, len);
    let local = mkdir(s.dir().join("resumed"));
    s.ok(SftpCmd::SetOptions(SftpOptions {
        keep_partial: true,
        ..SftpOptions::default()
    }));
    let end = cancel_mid_way(&mut s, &remote, &local, false);
    assert!(
        matches!(end, SftpEvent::TransferCancelled { .. }),
        "{end:?}"
    );
    let part = local.join("big.bin.part");
    let kept = fs::metadata(&part).expect("partial kept").len();
    assert!(kept > 0 && kept < len, "kept {kept}");
    // Without `resume` the partial is a conflict, not silently truncated.
    let report = done_report(s.download(&[&remote], &local, OverwritePolicy::Ask, false));
    assert_eq!(
        report.skipped[0].reason,
        SkipReason::Exists(ConflictKind::PartialExists)
    );
    let report = done_report(s.download(&[&remote], &local, OverwritePolicy::Ask, true));
    assert_eq!((report.files_done, report.bytes), (1, len - kept));
    assert!(!part.exists());
    assert!(
        files_equal(&remote, &local.join("big.bin")),
        "resumed file differs"
    );
}

fn files_equal(left: &Path, right: &Path) -> bool {
    use std::io::Read as _;
    let mut left = fs::File::open(left).unwrap();
    let mut right = fs::File::open(right).unwrap();
    let (mut lbuf, mut rbuf) = (vec![0; 1 << 20], vec![0; 1 << 20]);
    loop {
        let got = left.read(&mut lbuf).unwrap();
        // Same-sized reads: both are local regular files.
        let other = right.read_exact(&mut rbuf[..got]).map(|()| got);
        if other.is_err() || lbuf[..got] != rbuf[..got] {
            return false;
        }
        if got == 0 {
            return right.read(&mut rbuf).unwrap() == 0;
        }
    }
}

#[test]
fn sftp_never_writes_through_a_local_symlink() {
    let Some(mut s) = Sftp::start() else { return };
    let remote = mkdir(s.dir().join("r"));
    fs::write(remote.join("a.txt"), "from server").unwrap();
    let outside = s.dir().join("outside.txt");
    fs::write(&outside, "precious").unwrap();
    let local = mkdir(s.dir().join("l"));
    let file = remote.join("a.txt");

    // A planted `.part` symlink: never followed, even on a confirmed overwrite.
    symlink(&outside, local.join("a.txt.part")).unwrap();
    let report = done_report(s.download(&[&file], &local, OverwritePolicy::Ask, true));
    assert_eq!(
        report.skipped[0].reason,
        SkipReason::Exists(ConflictKind::PartialExists)
    );
    let report = done_report(s.download(&[&file], &local, OverwritePolicy::Overwrite, false));
    assert_eq!(report.files_done, 0);
    assert_eq!(report.failed.len(), 1, "{report:?}");
    assert_eq!(fs::read_to_string(&outside).unwrap(), "precious");
    fs::remove_file(local.join("a.txt.part")).unwrap();

    // A target symlink: a conflict; a confirmed overwrite replaces the link, not its target.
    symlink(&outside, local.join("a.txt")).unwrap();
    let report = done_report(s.download(&[&file], &local, OverwritePolicy::Ask, false));
    assert_eq!(
        report.skipped[0].reason,
        SkipReason::Exists(ConflictKind::Exists)
    );
    let report = done_report(s.download(&[&file], &local, OverwritePolicy::Overwrite, false));
    assert_eq!(report.files_done, 1);
    assert!(
        fs::symlink_metadata(local.join("a.txt"))
            .unwrap()
            .file_type()
            .is_file()
    );
    assert_eq!(
        fs::read_to_string(local.join("a.txt")).unwrap(),
        "from server"
    );
    assert_eq!(fs::read_to_string(&outside).unwrap(), "precious");

    // A symlinked subdirectory is never entered when downloading a tree into it.
    let tree = mkdir(remote.join("t"));
    fs::write(tree.join("x"), "x").unwrap();
    let elsewhere = mkdir(s.dir().join("elsewhere"));
    symlink(&elsewhere, local.join("t")).unwrap();
    let report = done_report(s.download(&[&tree], &local, OverwritePolicy::Overwrite, false));
    assert_eq!(report.files_done, 0);
    assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
}

#[test]
fn sftp_shutdown_closes_and_the_next_command_reopens() {
    let Some(mut s) = Sftp::start() else { return };
    s.send(SftpCmd::Shutdown);
    s.wait(|e| matches!(e, SftpEvent::Closed));
    assert!(matches!(
        s.request(SftpCmd::Realpath(".".into())),
        SftpEvent::Realpath { .. }
    ));
    // Closing the tab ends the engine with `Closed`.
    s.client.close();
    s.wait(|e| matches!(e, SftpEvent::Closed));
}

/// Throughput probe (not a regression test): `cargo nextest run -p fogu-session --run-ignored
/// only sftp_throughput --no-capture`.
#[test]
#[ignore = "measurement only"]
fn sftp_throughput() {
    let Some(mut s) = Sftp::start() else { return };
    let len = 512_u64 << 20;
    let remote = big_remote(&s, len);
    let local = mkdir(s.dir().join("tp"));
    let start = std::time::Instant::now();
    let report = done_report(s.download(&[&remote], &local, OverwritePolicy::Ask, false));
    let secs = start.elapsed().as_secs_f64();
    assert_eq!(report.bytes, len);
    eprintln!("download: {:.0} MiB/s", f64::from(512_u32) / secs);
    let up = mkdir(s.dir().join("tp-up"));
    let start = std::time::Instant::now();
    let report = done_report(s.transfer(SftpCmd::Upload {
        local: vec![local.join("big.bin")],
        remote_dir: up.display().to_string(),
        overwrite: OverwritePolicy::Ask,
    }));
    let secs = start.elapsed().as_secs_f64();
    assert_eq!(report.bytes, len);
    eprintln!("upload: {:.0} MiB/s", f64::from(512_u32) / secs);
}
