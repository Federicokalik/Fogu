//! "Follow terminal cwd" and "cd terminal here" : the remote shell's OSC 7 reports (OSC
//! side-channel scanner, `SessionOutput::Cwd { remote: true }`) move the panel while following
//! is on; manual navigation turns it off until "resume following". "cd terminal here" types a
//! quoted `cd` into the panel's shell.
//!
//! The reported path is remote-shell output (attacker data): it is only ever sent back to the
//! server as an SFTP path and shown through `display_name`; a path typed into the shell is
//! single-quoted and refused if it holds control characters.

use std::path::Path;

use fogu_core::shellquote::cd_command;
use fogu_session::SftpRequest;
use iced::Task;
use tokio::sync::mpsc::Sender;

use super::SftpPanelState;
use crate::app::App;
use crate::message::Message;
use crate::notifications::{Level, Notification};
use crate::tab::PaneId;

impl SftpPanelState {
    /// The panel's shell reported `path` as its cwd: remembered, and listed when following
    /// and visible. Relative or non-UTF-8 paths are ignored.
    pub fn on_shell_cwd(&mut self, path: &str, tx: &Sender<SftpRequest>) -> Result<(), String> {
        if !path.starts_with('/') {
            return Ok(());
        }
        self.shell_cwd = Some(path.to_owned());
        if !self.follow || !self.visible || self.path.as_deref() == Some(path) {
            return Ok(());
        }
        self.list_with(tx, path.to_owned(), self.path.is_some(), true)
    }

    /// The "follow terminal" toggle; turning it on goes to the terminal's cwd when known.
    pub fn set_follow(&mut self, on: bool, tx: &Sender<SftpRequest>) -> Result<(), String> {
        self.follow = on;
        match self.shell_cwd.clone() {
            Some(cwd) if on && self.path.as_deref() != Some(cwd.as_str()) => {
                self.list_with(tx, cwd, self.path.is_some(), true)
            }
            _ => Ok(()),
        }
    }
}

impl App {
    /// A remote cwd report from pane `pane`: kept on the pane (a panel opened later starts
    /// there) and passed to the panel bound to that pane.
    pub(crate) fn on_remote_cwd(&mut self, pane: PaneId, path: &Path) -> Task<Message> {
        let Some(path) = path.to_str() else {
            return Task::none();
        };
        let Some(tab) = self.tabs.iter_mut().find(|t| t.panes.contains_key(&pane)) else {
            return Task::none();
        };
        if let Some(p) = tab.panes.get_mut(&pane) {
            p.remote_cwd = Some(path.to_owned());
        }
        let tab = tab.id;
        let Some((panel, tx)) = self.panel_of(tab).filter(|(p, _)| p.pane == pane) else {
            return Task::none();
        };
        match panel.on_shell_cwd(path, &tx) {
            Ok(()) => Task::none(),
            Err(err) => self.notify(Notification::new(Level::Error, "SFTP").detail(err)),
        }
    }

    /// "cd terminal here": the panel's directory into its shell.
    pub(super) fn cd_here(&mut self, tab: crate::tab::TabId) -> Task<Message> {
        let scroll = self.config.terminal.scroll_on_input;
        let Some((panel, _)) = self.panel_of(tab) else {
            return Task::none();
        };
        let (Some(path), pane) = (panel.path.clone(), panel.pane) else {
            return Task::none();
        };
        let bytes = match cd_command(&path) {
            Ok(bytes) => bytes,
            Err(why) => {
                let note = Notification::new(Level::Warn, "cd terminal here").detail(why);
                return self.notify(note);
            }
        };
        // The shell will report the new cwd: keep following from there.
        panel.follow = true;
        if let Some(pane) = self.pane_mut(pane).filter(|p| p.state.is_running()) {
            pane.send_input(bytes, scroll);
            return Task::none();
        }
        let note = Notification::new(Level::Warn, "cd terminal here")
            .detail("The terminal is not connected.");
        self.notify(note)
    }
}
