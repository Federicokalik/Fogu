//! Engine events folded into the panel: listings, errors, transfer progress and outcomes,
//! conflicts, a closed channel.

use std::time::Instant;

use fogu_session::sftp::{CmdId, Entry, TransferId, TransferReport};
use fogu_session::{SftpEvent, SftpRequest, display_name};
use tokio::sync::mpsc::Sender;

use super::transfers::{self, Conflict, Direction, Origin, TransferState};
use super::{Pending, Reaction, SftpPanelState, error_note};
use crate::notifications::{Level, Notification, ToastAction};

impl SftpPanelState {
    /// Folds one engine event into the panel.
    pub fn on_event(
        &mut self,
        event: SftpEvent,
        tx: &Sender<SftpRequest>,
        now: Instant,
    ) -> Reaction {
        let mut out = Reaction::default();
        match event {
            SftpEvent::Realpath { id, path } => {
                if let Some(Pending::Resolve { home, .. }) = self.pending.remove(&id) {
                    if home {
                        self.home = Some(path.clone());
                    }
                    let push = self.path.is_some();
                    if let Err(err) = self.list(tx, path, push) {
                        out.notes.push(error_note("SFTP", err));
                    }
                }
            }
            SftpEvent::Listing {
                id,
                path,
                entries,
                truncated,
            } => self.on_listing(id, path, entries, truncated, &mut out),
            SftpEvent::Error { id, message } => self.on_error(id, message, &mut out),
            SftpEvent::TransferQueued { id, files, total } => {
                self.transfers.queued(id, files, total);
            }
            SftpEvent::TransferProgress {
                id,
                bytes,
                total,
                rate,
            } => self.transfers.progress(id, bytes, total, rate),
            SftpEvent::TransferDone { id, report } => {
                self.on_transfer_end(id, TransferState::Done, &report, (now, tx), &mut out);
            }
            SftpEvent::TransferFailed {
                id,
                message,
                report,
            } => {
                let state = TransferState::Failed(message);
                self.on_transfer_end(id, state, &report, (now, tx), &mut out);
            }
            SftpEvent::TransferCancelled { id, report } => {
                self.on_transfer_end(id, TransferState::Cancelled, &report, (now, tx), &mut out);
            }
            SftpEvent::Conflict { id, path, kind } => {
                if self.transfers.get(id).is_some() {
                    self.conflicts.queue.push_back(Conflict {
                        transfer: id,
                        path,
                        kind,
                    });
                }
            }
            SftpEvent::Closed => {
                self.pending.clear();
                self.loading = None;
                if self.transfers.active_count() > 0 {
                    self.transfers.fail_active("connection closed", now);
                    out.sweep = true;
                }
            }
            SftpEvent::Done(_) | SftpEvent::StatResult { .. } => {}
        }
        out
    }

    fn on_listing(
        &mut self,
        id: CmdId,
        path: String,
        entries: Vec<Entry>,
        truncated: bool,
        out: &mut Reaction,
    ) {
        let Some(Pending::List { from, .. }) = self.pending.remove(&id) else {
            return;
        };
        if self.loading != Some(id) {
            return;
        }
        self.loading = None;
        self.error = None;
        if let Some(from) = from {
            self.back.push(from);
        }
        if self.path.as_deref() != Some(path.as_str()) {
            self.listing.reset_view();
            self.scroll_y = 0.0;
        }
        if truncated {
            out.notes.push(
                Notification::new(Level::Warn, "Listing truncated").detail(format!(
                    "{} has more entries than max_listing_entries ({}).",
                    display_name(&path),
                    entries.len()
                )),
            );
        }
        self.path = Some(path);
        self.listing.replace(entries, truncated);
    }

    fn on_error(&mut self, id: CmdId, message: String, out: &mut Reaction) {
        let what = match self.pending.remove(&id) {
            Some(Pending::Resolve { home: true, .. }) => {
                "Cannot open the home directory".to_owned()
            }
            Some(Pending::List { path, .. } | Pending::Resolve { path, .. }) => {
                format!("Cannot open {}", display_name(&path))
            }
            None => "SFTP".to_owned(),
        };
        if self.loading == Some(id) {
            self.loading = None;
            if self.path.is_none() {
                self.error = Some(format!("{what}: {message}"));
            }
        }
        out.notes.push(error_note(&what, message));
    }

    fn on_transfer_end(
        &mut self,
        id: TransferId,
        state: TransferState,
        report: &TransferReport,
        (now, tx): (Instant, &Sender<SftpRequest>),
        out: &mut Reaction,
    ) {
        let Some(t) = self.transfers.finish(id, state, report, now) else {
            return;
        };
        out.sweep = true;
        let summary = transfers::report_summary(report);
        let failed = !report.failed.is_empty();
        let note = match (&t.state, t.direction, &t.origin) {
            (TransferState::Done, _, _) if failed && report.files_done == 0 => {
                Notification::new(Level::Error, "Transfer failed").detail(summary)
            }
            (TransferState::Done, Direction::Download, Origin::Download { local_dir, .. }) => {
                let level = if failed { Level::Warn } else { Level::Info };
                let note = Notification::new(level, "Download finished")
                    .detail(format!("{summary} \u{2192} {}", local_dir.display()));
                if report.files_done > 0 {
                    note.action(ToastAction::ShowInFileManager)
                } else {
                    note
                }
            }
            (TransferState::Done, _, _) => {
                let level = if failed { Level::Warn } else { Level::Info };
                Notification::new(level, "Upload finished").detail(summary)
            }
            (TransferState::Failed(why), _, _) => {
                Notification::new(Level::Error, "Transfer failed").detail(why)
            }
            (TransferState::Cancelled, _, _) => {
                Notification::new(Level::Info, "Transfer cancelled")
                    .detail(format!("Completed before the cancel: {summary}"))
            }
            _ => return,
        };
        let upload_here = matches!(&t.origin, Origin::Upload { remote_dir, .. }
            if self.path.as_deref() == Some(remote_dir.as_str()));
        out.notes.push(note);
        if upload_here
            && self.loading.is_none()
            && let Some(path) = self.path.clone()
            && let Err(err) = self.list(tx, path, false)
        {
            out.notes.push(error_note("SFTP", err));
        }
    }
}
