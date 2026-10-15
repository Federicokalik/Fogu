//! Uploads: local walk (blocking, on the blocking pool) → remote directories first → files
//! streamed through a pipelined writer. Same symlink rules as downloads: a local symlink is sent
//! as its target's content only when the target is a regular file; symlinked directories are
//! never descended. Remote files are created with `EXCLUDE` unless the user confirmed an
//! overwrite, with the local permission bits minus setuid/setgid/sticky.

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::io::AsyncReadExt as _;
use tokio_util::sync::CancellationToken;

use super::local_fs::validate_name;
use super::ops::join;
use super::remote::kind_of;
use super::transfer::{Ctx, Fatal, FileEnd, Progress, finish, on_existing, run_jobs};
use crate::sftp::{
    ConflictKind, EntryKind, NameError, OverwritePolicy, SftpEvent, SftpOptions, SkipReason,
    TransferReport,
};

/// One file to send.
struct Job {
    local: PathBuf,
    remote: String,
    size: u64,
    mode: u32,
    overwrite: OverwritePolicy,
    progress: Arc<Progress>,
}

/// What the local walk found: remote dirs to create (parents first), files, skips.
#[derive(Default)]
struct Plan {
    dirs: Vec<(String, String)>,
    files: Vec<(String, PathBuf, String, fs::Metadata)>,
    skipped: Vec<(String, SkipReason)>,
}

/// Runs a whole `Upload` command and sends its events.
pub(crate) async fn upload(
    ctx: Ctx,
    local: Vec<PathBuf>,
    remote_dir: String,
    overwrite: OverwritePolicy,
) {
    let mut report = TransferReport::default();
    let result = run(&ctx, local, &remote_dir, overwrite, &mut report).await;
    finish(&ctx, result, report).await;
}

async fn run(
    ctx: &Ctx,
    roots: Vec<PathBuf>,
    remote_dir: &str,
    overwrite: OverwritePolicy,
    report: &mut TransferReport,
) -> Result<(), Fatal> {
    if kind_of(&ctx.remote.stat(remote_dir).await?) != EntryKind::Dir {
        return Err(Fatal("the destination is not a directory".into()));
    }
    let (opts, cancel, base) = (ctx.opts.clone(), ctx.cancel.clone(), remote_dir.to_owned());
    let plan = tokio::task::spawn_blocking(move || walk(&roots, &base, &opts, &cancel))
        .await
        .map_err(|_| Fatal("local walk failed".into()))??;
    for (rel, reason) in plan.skipped {
        report.skip(rel, reason);
    }
    for (rel, path) in &plan.dirs {
        match ctx.remote.mkdir(path).await {
            Ok(()) => report.dirs_created += 1,
            Err(err) => match ctx.remote.lstat(path).await {
                Ok(attrs) if kind_of(&attrs) == EntryKind::Dir => {}
                _ => report.fail(rel.clone(), err.message),
            },
        }
    }
    let total = plan.files.iter().map(|(_, _, _, m)| m.len()).sum();
    let queued = SftpEvent::TransferQueued {
        id: ctx.id,
        files: plan.files.len() as u64,
        total,
    };
    let _gone = ctx.events.send(queued).await;
    let progress = Progress::new(ctx, total);
    let jobs = plan
        .files
        .into_iter()
        .map(|(rel, local, remote, meta)| {
            let job = Job {
                local,
                remote,
                size: meta.len(),
                mode: meta.permissions().mode() & 0o777,
                overwrite,
                progress: Arc::clone(&progress),
            };
            (rel, job)
        })
        .collect();
    run_jobs(ctx, jobs, report, |ctx, job| async move {
        send(&ctx, job).await
    })
    .await;
    Ok(())
}

/// The local name of `path` as a valid remote name.
fn local_name(path: &Path) -> Result<String, NameError> {
    let name = path
        .file_name()
        .ok_or(NameError::Empty)?
        .to_str()
        .ok_or(NameError::Invalid)?;
    validate_name(name).map(str::to_owned)
}

/// Iterative local walk with the depth/entry caps (runs on the blocking pool).
fn walk(
    roots: &[PathBuf],
    remote_dir: &str,
    opts: &SftpOptions,
    cancel: &CancellationToken,
) -> Result<Plan, Fatal> {
    let mut plan = Plan::default();
    let mut queue = std::collections::VecDeque::new();
    for root in roots {
        match local_name(root) {
            Ok(name) => classify(
                &mut plan,
                &mut queue,
                root.clone(),
                name.clone(),
                join(remote_dir, &name),
                0,
            ),
            Err(err) => plan
                .skipped
                .push((root.display().to_string(), SkipReason::InvalidName(err))),
        }
    }
    let mut seen = 0_usize;
    while let Some((dir, rel, remote, depth)) = queue.pop_front() {
        if cancel.is_cancelled() {
            break;
        }
        if depth >= opts.max_walk_depth {
            return Err(Fatal(
                "directory tree too deep; nothing was transferred".into(),
            ));
        }
        let entries = fs::read_dir(&dir).map_err(|e| Fatal(format!("{rel}: {e}")))?;
        for entry in entries {
            let entry = entry.map_err(|e| Fatal(format!("{rel}: {e}")))?;
            seen += 1;
            if seen > opts.max_walk_entries {
                return Err(Fatal("too many files; nothing was transferred".into()));
            }
            let path = entry.path();
            match local_name(&path) {
                Ok(name) => {
                    let (child_rel, child_remote) = (format!("{rel}/{name}"), join(&remote, &name));
                    classify(
                        &mut plan,
                        &mut queue,
                        path,
                        child_rel,
                        child_remote,
                        depth + 1,
                    );
                }
                Err(err) => plan.skipped.push((
                    format!(
                        "{rel}/{}",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    ),
                    SkipReason::InvalidName(err),
                )),
            }
        }
    }
    Ok(plan)
}

type Queue = std::collections::VecDeque<(PathBuf, String, String, usize)>;

/// Sorts one local entry (`lstat`) into the plan.
fn classify(
    plan: &mut Plan,
    queue: &mut Queue,
    path: PathBuf,
    rel: String,
    remote: String,
    depth: usize,
) {
    let Ok(meta) = fs::symlink_metadata(&path) else {
        plan.skipped.push((rel, SkipReason::SymlinkNotFile));
        return;
    };
    let kind = meta.file_type();
    if kind.is_dir() {
        plan.dirs.push((rel.clone(), remote.clone()));
        queue.push_back((path, rel, remote, depth));
    } else if kind.is_file() {
        plan.files.push((rel, path, remote, meta));
    } else if kind.is_symlink() {
        match fs::metadata(&path) {
            Ok(target) if target.is_file() => plan.files.push((rel, path, remote, target)),
            Ok(target) if target.is_dir() => plan.skipped.push((rel, SkipReason::SymlinkToDir)),
            _ => plan.skipped.push((rel, SkipReason::SymlinkNotFile)),
        }
    } else {
        plan.skipped.push((rel, SkipReason::Special));
    }
}

/// Uploads one file; a remote file this run created is removed on failure/cancel unless
/// `keep_partial`.
async fn send(ctx: &Ctx, job: Job) -> FileEnd {
    let truncate = match ctx.remote.try_lstat(&job.remote).await {
        Ok(None) => false,
        Ok(Some(_)) => match on_existing(job.overwrite, ConflictKind::Exists) {
            Ok(()) => true,
            Err(end) => return end,
        },
        Err(err) => return FileEnd::Failed(err.message),
    };
    let mut input = match tokio::fs::File::open(&job.local).await {
        Ok(file) => file,
        Err(err) => return FileEnd::Failed(err.to_string()),
    };
    let handle = match ctx.remote.open_write(&job.remote, job.mode, truncate).await {
        Ok(handle) => handle,
        // Created meanwhile (EXCLUDE): same as finding it in the check above.
        Err(_) if !truncate && matches!(ctx.remote.try_lstat(&job.remote).await, Ok(Some(_))) => {
            return match on_existing(job.overwrite, ConflictKind::Exists) {
                Err(end) => end,
                Ok(()) => FileEnd::Failed("the target appeared during the upload".into()),
            };
        }
        Err(err) => return FileEnd::Failed(err.message),
    };
    let mut writer = ctx.remote.writer(&handle, 0);
    let mut bytes = 0_u64;
    let result = async {
        loop {
            if ctx.cancel.is_cancelled() {
                return Ok(false);
            }
            let mut buf = vec![0; ctx.remote.write_chunk];
            let n = input.read(&mut buf).await.map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            buf.truncate(n);
            writer.write(buf).await.map_err(|e| e.message)?;
            bytes += n as u64;
            job.progress.add(n as u64);
        }
        writer.finish().await.map_err(|e| e.message)?;
        Ok::<_, String>(true)
    }
    .await;
    drop(writer);
    let closed = ctx.remote.close_handle(handle).await;
    let end = match (result, closed) {
        (Ok(true), Ok(())) => return FileEnd::Done { bytes },
        (Ok(true), Err(err)) => FileEnd::Failed(format!("close: {}", err.message)),
        (Ok(false), _) => FileEnd::Cancelled,
        (Err(message), _) => FileEnd::Failed(message),
    };
    tracing::debug!(size = job.size, bytes, "upload did not complete");
    if !ctx.opts.keep_partial {
        let _best_effort = ctx.remote.remove_file(&job.remote).await;
    }
    end
}
