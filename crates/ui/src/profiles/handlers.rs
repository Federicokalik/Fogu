//! App wiring of the profile sidebar: toggling, search / quick connect, keyboard and mouse,
//! launching profiles (recording Recent) and the stored-profile edits.

use std::sync::Arc;

use fogu_core::config::{self, ProfileEdit, slug, unique_profile_id};
use fogu_core::{Group, GroupId, Profile, ProfileId, Side};
use iced::advanced::widget::operate;
use iced::advanced::widget::operation::focusable;
use iced::widget::operation;
use iced::{Point, Task, clipboard};

use super::{
    Entry, MIN_WIDTH, Origin, ProfilesView, QUICK_INPUT, SEARCH_INPUT, entries, items, lines,
    now_secs, quick_profile, rows, ssh_command,
};
use crate::app::App;
use crate::fuzzy_list;
use crate::message::Message;
use crate::modal::{Modal, ModalButton};
use crate::notifications::{Level, Notification};

/// Where a launched profile opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Launch {
    /// A new tab.
    Tab,
    /// A split right of the focused pane.
    Split,
    /// A new tab with the SFTP panel open (SSH).
    Sftp,
}

/// Sidebar messages.
#[derive(Clone, Debug)]
pub enum ProfilesMsg {
    Query(String),
    /// Enter in the search field (or with the sidebar focused): open the selected item.
    Submit,
    Quick(String),
    QuickSubmit,
    /// ↑ (`-1`) / ↓ (`1`).
    Move(i8),
    /// Click on the item at this position.
    Select(usize),
    Toggle(super::Section),
    Open(ProfileId, Launch),
    /// Double- or middle-click on a row: [`ProfilesMsg::Open`], and the press that did it
    /// (reported after this message) must not hand the keyboard back to the sidebar.
    RowOpen(ProfileId, Launch),
    /// Right-click on the row at this position (it becomes the selected one).
    Menu(ProfileId, usize),
    /// "Move to group ▸".
    MoveMenu,
    /// Move the menu's profile to a group (`None` = Ungrouped).
    SetGroup(Option<GroupId>),
    /// "New group…" submitted: create it and move the menu's profile into it.
    NewGroup(String),
    Duplicate(ProfileId),
    CopySsh(ProfileId),
    AskDelete(ProfileId),
    ConfirmDelete(bool),
    Hover(bool),
    Cursor(Point),
    ResizeStart,
    /// Pointer x while dragging the edge (window coordinates = the new width).
    ResizeTo(f32),
    ResizeEnd,
}

impl App {
    /// `toggle-profiles` / the tab bar button.
    pub(crate) fn toggle_profiles(&mut self) -> Task<Message> {
        let view = &mut self.profiles;
        view.open = !view.open;
        view.focused = view.open;
        view.resizing = false;
        if view.open {
            return operation::focus(SEARCH_INPUT);
        }
        Task::none()
    }

    /// Reads the Recent list from `path` and saves there from now on; a broken file is logged
    /// and replaced on the next save.
    pub(crate) fn load_recent(&mut self, path: std::path::PathBuf) {
        match fogu_core::recent::Recent::load(&path) {
            Ok(recent) => self.profiles.recent = recent,
            Err(err) => {
                tracing::warn!(path = %path.display(), error = %err, "recent profiles not read");
            }
        }
        self.profiles.recent_path = Some(path);
    }

    /// The sidebar has the keyboard: nothing else in front of it.
    pub(crate) fn sidebar_has_keys(&self) -> bool {
        self.profiles.open
            && self.profiles.focused
            && self.picker.is_none()
            && self.menu.is_none()
            && self.renaming.is_none()
            && self.pending_close.is_none()
            && self.pending_paste.is_none()
            && self.profiles.pending_delete.is_none()
    }

    /// Every profile the sidebar and picker show.
    pub(crate) fn all_entries(&self) -> Vec<Entry> {
        entries(
            &self.config,
            &self.profiles.builtins,
            Some(&self.profiles.recent),
        )
    }

    /// Entry indices the sidebar currently lists, in order.
    fn shown_items(&self, all: &[Entry]) -> Vec<usize> {
        let rows = rows(all, &self.config, &self.profiles.recent, now_secs());
        items(&lines(&self.profiles, &self.config, all, &rows))
    }

    pub(crate) fn on_profiles(&mut self, message: ProfilesMsg) -> Task<Message> {
        let view = &mut self.profiles;
        match message {
            ProfilesMsg::Query(query) => {
                view.query = query;
                view.selected = 0;
            }
            ProfilesMsg::Quick(text) => {
                view.quick = text;
                view.quick_error = None;
            }
            ProfilesMsg::QuickSubmit => return self.quick_connect(),
            ProfilesMsg::Move(delta) => {
                let len = self.shown_items(&self.all_entries()).len();
                let view = &mut self.profiles;
                view.selected = fuzzy_list::step(view.selected, delta, len);
            }
            ProfilesMsg::Select(pos) => {
                view.selected = pos;
                view.focused = true;
            }
            ProfilesMsg::Submit => {
                let all = self.all_entries();
                let shown = self.shown_items(&all);
                let pick = shown.get(self.profiles.selected).and_then(|&i| all.get(i));
                if let Some(entry) = pick {
                    let profile = entry.profile.clone();
                    return self.launch(&profile, Launch::Tab);
                }
            }
            ProfilesMsg::Toggle(section) => {
                if !view.collapsed.remove(&section) {
                    view.collapsed.insert(section);
                }
            }
            ProfilesMsg::RowOpen(id, how) => {
                view.skip_press = true;
                return self.on_profiles(ProfilesMsg::Open(id, how));
            }
            ProfilesMsg::Open(id, how) => {
                if let Some(entry) = self.all_entries().into_iter().find(|e| e.profile.id == id) {
                    return self.launch(&entry.profile, how);
                }
            }
            ProfilesMsg::Hover(over) => view.hovered = over,
            ProfilesMsg::Cursor(at) => view.cursor = at,
            ProfilesMsg::ResizeStart => view.resizing = true,
            ProfilesMsg::ResizeTo(x) => {
                let max = (self.window_size.width * 0.6).max(MIN_WIDTH);
                view.width = x.clamp(MIN_WIDTH, max);
            }
            ProfilesMsg::ResizeEnd => view.resizing = false,
            ProfilesMsg::ConfirmDelete(yes) => {
                let id = view.pending_delete.take();
                if let (true, Some(id)) = (yes, id) {
                    return self.save_edits(&[ProfileEdit::Delete(id)]);
                }
            }
            ProfilesMsg::Menu(..)
            | ProfilesMsg::MoveMenu
            | ProfilesMsg::SetGroup(_)
            | ProfilesMsg::NewGroup(_)
            | ProfilesMsg::Duplicate(_)
            | ProfilesMsg::CopySsh(_)
            | ProfilesMsg::AskDelete(_) => return self.on_profile_menu(message),
        }
        Task::none()
    }

    /// Opens `profile` as asked and moves it to the front of Recent; the keyboard goes to the
    /// new terminal.
    pub(crate) fn launch(&mut self, profile: &Profile, how: Launch) -> Task<Message> {
        self.menu = None;
        self.remember(&profile.id);
        self.profiles.focused = false;
        let unfocus = operate(focusable::unfocus());
        match (how, self.active_pane().map(|p| p.id)) {
            (Launch::Split, Some(target)) => {
                self.split_now(target, Side::Right, None, Some(profile.clone()));
            }
            (Launch::Sftp, _) => {
                self.open_tab(profile, None);
                return Task::batch([unfocus, self.toggle_sftp()]);
            }
            _ => {
                self.open_tab(profile, None);
            }
        }
        unfocus
    }

    /// Records `id` in Recent and saves the list (errors are logged: Recent is a convenience).
    fn remember(&mut self, id: &ProfileId) {
        let view = &mut self.profiles;
        view.recent.touch(id, now_secs());
        if let Some(path) = &view.recent_path
            && let Err(err) = view.recent.save(path)
        {
            tracing::warn!(path = %path.display(), error = %err, "recent profiles not saved");
        }
    }

    /// Quick connect: `[user@]host[:port]` → an SSH tab; a bad string is shown under the field.
    fn quick_connect(&mut self) -> Task<Message> {
        match quick_profile(&self.profiles.quick) {
            Ok(profile) => {
                self.profiles.quick.clear();
                self.launch(&profile, Launch::Tab)
            }
            Err(err) => {
                self.profiles.quick_error = Some(err.to_string());
                operation::focus(QUICK_INPUT)
            }
        }
    }

    /// Writes `edits` to config.toml in place, then applies them to the running config; an
    /// error leaves both untouched and says why.
    pub(crate) fn save_edits(&mut self, edits: &[ProfileEdit]) -> Task<Message> {
        self.menu = None;
        let dir = config::config_dir(self.config_dir.as_deref());
        if let Err(why) = config::edit_profiles(&dir, edits) {
            let note =
                Notification::new(Level::Error, "Profiles not saved").detail(why.to_string());
            return self.notify(note);
        }
        let config = Arc::make_mut(&mut self.config);
        for edit in edits {
            edit.apply(config);
        }
        Task::none()
    }

    /// "Duplicate": a stored profile is copied in the file; a built-in or quick-connect one is
    /// saved as a new stored profile.
    pub(crate) fn duplicate(&mut self, entry: &Entry) -> Task<Message> {
        let p = &entry.profile;
        let name = format!("{} copy", p.name);
        let edit = match entry.origin {
            Origin::Stored => ProfileEdit::Duplicate {
                from: p.id.clone(),
                id: unique_profile_id(&self.config, &format!("{}-copy", p.id.0)),
                name,
            },
            Origin::Builtin | Origin::Quick => ProfileEdit::Add(Box::new(Profile {
                id: unique_profile_id(&self.config, &slug(&name, "profile")),
                name,
                ..p.clone()
            })),
        };
        self.save_edits(&[edit])
    }

    /// Clipboard + a toast with what was copied.
    pub(crate) fn copy_ssh(&mut self, profile: &Profile) -> Task<Message> {
        self.menu = None;
        let Some(command) = ssh_command(profile, &self.config) else {
            return Task::none();
        };
        let note = Notification::new(Level::Info, "Copied").detail(command.clone());
        Task::batch([clipboard::write(command), self.notify(note)])
    }

    /// Creates group `name` and moves the menu's profile into it.
    pub(crate) fn new_group(&mut self, name: &str) -> Task<Message> {
        let name = name.trim();
        let Some(target) = self.profiles.menu_target.clone() else {
            return Task::none();
        };
        if name.is_empty() {
            return Task::none();
        }
        let group = Group {
            id: config::unique_group_id(&self.config, name),
            name: name.to_owned(),
        };
        let set = ProfileEdit::SetGroup {
            id: target,
            group: Some(group.id.clone()),
        };
        self.save_edits(&[ProfileEdit::AddGroup(group), set])
    }

    /// "Delete profile?" while a delete waits for confirmation.
    pub(crate) fn delete_modal(&self) -> Option<Modal> {
        let id = self.profiles.pending_delete.as_ref()?;
        let name = self
            .config
            .profiles
            .iter()
            .find(|p| &p.id == id)?
            .name
            .as_str();
        Some(Modal {
            title: "Delete profile?",
            body: format!(
                "\"{}\" is removed from config.toml (the previous file is kept as config.toml.bak).",
                fogu_term::sanitize_title(name)
            ),
            buttons: vec![
                ModalButton {
                    label: "Delete",
                    on_press: Message::Profiles(ProfilesMsg::ConfirmDelete(true)),
                },
                ModalButton {
                    label: "Cancel",
                    on_press: Message::Profiles(ProfilesMsg::ConfirmDelete(false)),
                },
            ],
        })
    }

    /// Esc with the sidebar focused: clear the search, else hand the keyboard back.
    pub(crate) fn sidebar_escape(&mut self) -> bool {
        let view: &mut ProfilesView = &mut self.profiles;
        if !view.open || !view.focused {
            return false;
        }
        if view.query.is_empty() {
            view.focused = false;
        } else {
            view.query.clear();
            view.selected = 0;
        }
        true
    }
}
