//! The Profiles page: the profile new tabs open, the sidebar's options, every profile with its
//! actions, the groups, and the defaults every local or SSH profile inherits.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};

use fogu_core::config::{KeyEdit, ProfileClick, ProfileEdit, TomlValue, config_dir, set_keys};
use fogu_core::{Config, GroupId, Profile, ProfileId, ProfileKind};
use iced::widget::{Id, button, column, container, row, space, text};
use iced::{Alignment, Border, Element, Length, Task};

use super::render::{badge_chip, choice_owned, field, small};
use super::spec::{Applies, Kind, Spec, Unit, Value, from_usize, round, to_f32};
use super::{Confirm, Section, SettingsMsg};
use crate::app::App;
use crate::controls::{self, ButtonKind};
use crate::forms::{Field, FieldValue};
use crate::message::Message;
use crate::notifications::{Level, Notification};
use crate::palette::Look;
use crate::profile_editor::{Effect, FieldId, ProfileEditor, Source};
use crate::profiles::{Entry, Origin};

/// One plain Profiles page setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProfKey {
    ShowBuiltin,
    RecentProfiles,
    ProfilesClick,
    ReadSshConfig,
    SidebarWidth,
}

const CLICK: &[&str] = &["One click", "Double click"];

impl ProfKey {
    /// Every key, in page order.
    pub const ALL: [Self; 5] = [
        Self::ShowBuiltin,
        Self::RecentProfiles,
        Self::ProfilesClick,
        Self::ReadSshConfig,
        Self::SidebarWidth,
    ];

    /// Its description.
    pub fn spec(self) -> Spec {
        let row = |(path, key), label, help, keywords, kind| Spec {
            section: Section::Profiles,
            group: "Sidebar",
            path,
            key,
            label,
            help,
            keywords,
            kind,
            applies: Applies::Live,
        };
        let look: &'static [&'static str] = &["appearance"];
        match self {
            Self::ShowBuiltin => row(
                (look, "show_builtin_profiles"),
                "Show built-in shells",
                "\"Default shell\" and the installed shells in the sidebar and the picker.",
                "builtin powershell cmd bash shells",
                Kind::Toggle,
            ),
            Self::RecentProfiles => row(
                (look, "recent_profiles"),
                "Recent profiles",
                "How many recently opened profiles the sidebar lists first (0 hides them).",
                "recent history",
                Kind::Slider {
                    min: 0.0,
                    max: 10.0,
                    step: 1.0,
                    unit: Unit::Plain(0),
                    auto: None,
                },
            ),
            Self::ProfilesClick => row(
                (look, "profiles_click"),
                "Open a profile with",
                "In the sidebar; the picker always opens on one click.",
                "single double click open",
                Kind::Choice(CLICK),
            ),
            Self::ReadSshConfig => row(
                (&["ssh"], "read_ssh_config"),
                "Hosts from ~/.ssh/config",
                "Every plain Host of ~/.ssh/config as a read-only profile.",
                "ssh config hosts openssh import",
                Kind::Toggle,
            ),
            Self::SidebarWidth => row(
                (look, "profiles_sidebar_width"),
                "Sidebar width",
                "Its starting width; drag its edge to resize it.",
                "sidebar width size",
                Kind::Slider {
                    min: 180.0,
                    max: 600.0,
                    step: 10.0,
                    unit: Unit::Px,
                    auto: None,
                },
            ),
        }
    }

    /// Its current value.
    pub fn get(self, config: &Config) -> Value {
        let a = &config.appearance;
        match self {
            Self::ShowBuiltin => Value::Bool(a.show_builtin_profiles),
            Self::RecentProfiles => Value::Num(from_usize(a.recent_profiles)),
            Self::ProfilesClick => Value::Choice(match a.profiles_click {
                ProfileClick::Single => 0,
                ProfileClick::Double => 1,
            }),
            Self::ReadSshConfig => Value::Bool(config.ssh.read_ssh_config),
            Self::SidebarWidth => Value::Num(f64::from(a.profiles_sidebar_width)),
        }
    }

    /// Sets it in the running config.
    pub fn set(self, config: &mut Config, value: &Value) {
        let a = &mut config.appearance;
        match self {
            Self::ShowBuiltin => a.show_builtin_profiles = value.bool(),
            Self::RecentProfiles => {
                a.recent_profiles = usize::try_from(round(value.num())).unwrap_or(0);
            }
            Self::ProfilesClick => {
                a.profiles_click = if value.choice() == 1 {
                    ProfileClick::Double
                } else {
                    ProfileClick::Single
                };
            }
            Self::ReadSshConfig => config.ssh.read_ssh_config = value.bool(),
            Self::SidebarWidth => a.profiles_sidebar_width = to_f32(value.num()),
        }
    }

    /// How a choice is written.
    pub fn named(self, value: &Value) -> Option<TomlValue> {
        match self {
            Self::ProfilesClick => Some(TomlValue::from(if value.choice() == 1 {
                "double"
            } else {
                "single"
            })),
            _ => None,
        }
    }
}

/// Which defaults the editor shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DefaultsKind {
    #[default]
    Ssh,
    Local,
}

/// The page's state.
#[derive(Debug, Default)]
pub struct ProfilesPage {
    /// Which defaults the editor shows.
    pub kind: DefaultsKind,
    /// The defaults editor (made when the page is first shown).
    pub defaults: Option<Box<ProfileEditor>>,
    /// Group names being typed, by group.
    pub group_names: BTreeMap<GroupId, String>,
}

/// A choice on the Profiles page.
#[derive(Clone, Debug)]
pub enum ProfilesPageMsg {
    /// The profile new tabs open (index into the dropdown).
    DefaultProfile(usize),
    /// Open a profile in a new tab.
    Connect(ProfileId),
    /// Open its editor.
    Edit(ProfileId),
    /// A new SSH (`true`) or local profile.
    New(bool),
    /// Duplicate it (or copy a `~/.ssh/config` host to the profiles).
    Duplicate(ProfileId),
    /// Delete it (asks first).
    Delete(ProfileId),
    /// A group's name is being typed.
    GroupName(GroupId, String),
    /// Enter in a group's name.
    GroupCommit(GroupId),
    /// Move a group up (`true`) or down.
    MoveGroup(GroupId, bool),
    /// Delete a group (asks first).
    DeleteGroup(GroupId),
    /// Add a group.
    AddGroup,
    /// Show the local or SSH defaults.
    Kind(DefaultsKind),
    /// A field of the defaults editor.
    Defaults(Field<FieldId>),
    /// The file picker of a defaults field answered.
    Picked(FieldId, Option<PathBuf>),
}

/// The ids of the page's blocks, for the search.
pub const DEFAULT_ANCHOR: &str = "anchor:default-profile";
pub const LIST_ANCHOR: &str = "anchor:profile-list";
pub const GROUPS_ANCHOR: &str = "anchor:groups";
pub const DEFAULTS_ANCHOR: &str = "anchor:profile-defaults";

/// A config with the built-in defaults: the defaults editor's placeholders.
static BUILT_IN: LazyLock<Config> = LazyLock::new(Config::default);

fn msg(m: ProfilesPageMsg) -> Message {
    Message::Settings(SettingsMsg::Profiles(m))
}

/// The page body; `rows` is the plain settings' block.
pub fn body<'a>(
    page: &'a ProfilesPage,
    config: &Config,
    entries: &[Entry],
    rows: Element<'a, Message>,
    look: Look,
) -> Element<'a, Message> {
    column![
        block(DEFAULT_ANCHOR, default_profile(config, entries, look)),
        rows,
        block(LIST_ANCHOR, profile_list(entries, look)),
        block(GROUPS_ANCHOR, groups(page, config, look)),
        block(DEFAULTS_ANCHOR, defaults(page, look)),
    ]
    .spacing(22)
    .into()
}

fn block<'a>(id: &'static str, content: Element<'a, Message>) -> Element<'a, Message> {
    container(content)
        .id(Id::new(id))
        .width(Length::Fill)
        .into()
}

/// The profiles the default can be: every one but quick-connect strings.
fn choices(entries: &[Entry]) -> Vec<&Entry> {
    entries
        .iter()
        .filter(|e| e.origin != Origin::Quick)
        .collect()
}

/// "Default profile": a drop-down of every profile.
fn default_profile<'a>(config: &Config, entries: &[Entry], look: Look) -> Element<'a, Message> {
    let list = choices(entries);
    let mut labels: Vec<String> = list
        .iter()
        .map(|e| match e.origin {
            Origin::Builtin => format!("{} (built-in)", e.profile.name),
            Origin::SshConfig => format!("{} (~/.ssh/config)", e.profile.name),
            Origin::Stored | Origin::Quick => e.profile.name.clone(),
        })
        .collect();
    let current = config
        .default_profile
        .clone()
        .unwrap_or_else(|| Profile::default_shell().id);
    let found = list.iter().position(|e| e.profile.id == current);
    let selected = found.unwrap_or_else(|| {
        labels.push(format!("(missing: {})", current.0));
        labels.len() - 1
    });
    column![
        controls::section_label(look, "Default profile"),
        row![
            column![
                text("New tabs and the first tab open")
                    .size(13)
                    .font(look.text.chrome)
                    .color(look.fg),
                small(
                    look,
                    "A host from ~/.ssh/config that is gone falls back to Default shell."
                ),
            ]
            .spacing(3)
            .width(Length::Fill),
            container(choice_owned(look, labels, Some(selected), |i| msg(
                ProfilesPageMsg::DefaultProfile(i)
            )))
            .width(super::render::CONTROL_W)
            .align_x(Alignment::End),
        ]
        .spacing(16)
        .align_y(Alignment::Center),
    ]
    .spacing(10)
    .padding([6, 8])
    .into()
}

/// Every profile with its actions.
fn profile_list<'a>(entries: &[Entry], look: Look) -> Element<'a, Message> {
    let mut list = column![
        row![
            controls::section_label(look, "Profiles"),
            space().width(Length::Fill),
            controls::action(
                "New SSH profile",
                ButtonKind::Secondary,
                look,
                Some(msg(ProfilesPageMsg::New(true)))
            ),
            controls::action(
                "New local profile",
                ButtonKind::Secondary,
                look,
                Some(msg(ProfilesPageMsg::New(false)))
            ),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
    ]
    .spacing(4);
    for entry in choices(entries) {
        list = list.push(profile_row(entry, look));
    }
    list.padding([6, 8]).into()
}

fn profile_row<'a>(entry: &Entry, look: Look) -> Element<'a, Message> {
    let id = entry.profile.id.clone();
    let stored = entry.origin == Origin::Stored;
    let origin = match entry.origin {
        Origin::Builtin => Some("built-in"),
        Origin::SshConfig => Some("~/.ssh/config"),
        Origin::Stored | Origin::Quick => None,
    };
    let mut title = row![
        text(fogu_term::sanitize_title(&entry.profile.name))
            .size(13)
            .font(look.text.chrome)
            .color(look.fg)
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    if let Some(origin) = origin {
        title = title.push(badge_chip(look, origin));
    }
    let flat = |label: &str, message: Option<Message>| -> Element<'a, Message> {
        button(
            text(controls::button_label(look, label))
                .size(12)
                .font(look.text.chrome),
        )
        .padding([3, 8])
        .style(move |_theme: &iced::Theme, status| controls::flat_button(look, status, false))
        .on_press_maybe(message)
        .into()
    };
    let edit = if entry.origin == Origin::SshConfig {
        "View"
    } else {
        "Edit"
    };
    let copy = if entry.origin == Origin::SshConfig {
        "Copy to mine"
    } else {
        "Duplicate"
    };
    let editable = entry.origin != Origin::Builtin;
    container(
        row![
            column![title, small(look, &entry.subtitle)]
                .spacing(2)
                .width(Length::Fill),
            flat("Connect", Some(msg(ProfilesPageMsg::Connect(id.clone())))),
            flat(
                edit,
                editable.then(|| msg(ProfilesPageMsg::Edit(id.clone())))
            ),
            flat(copy, Some(msg(ProfilesPageMsg::Duplicate(id.clone())))),
            flat("Delete", stored.then(|| msg(ProfilesPageMsg::Delete(id)))),
        ]
        .spacing(4)
        .align_y(Alignment::Center),
    )
    .padding([6, 8])
    .style(move |_theme: &iced::Theme| container::Style {
        border: Border {
            color: look.border,
            width: 1.0,
            radius: look.shape.control.into(),
        },
        ..container::Style::default()
    })
    .into()
}

/// The groups: rename, reorder, delete, add.
fn groups<'a>(page: &ProfilesPage, config: &Config, look: Look) -> Element<'a, Message> {
    let mut list = column![
        row![
            controls::section_label(look, "Groups"),
            space().width(Length::Fill),
            controls::action(
                "New group",
                ButtonKind::Secondary,
                look,
                Some(msg(ProfilesPageMsg::AddGroup))
            ),
        ]
        .align_y(Alignment::Center)
    ]
    .spacing(6);
    if config.groups.is_empty() {
        list = list.push(small(
            look,
            "No groups yet. Groups sort the sidebar; a profile picks its group in its editor.",
        ));
    }
    let final_index = config.groups.len().saturating_sub(1);
    for (i, group) in config.groups.iter().enumerate() {
        let gid = group.id.clone();
        let name = page
            .group_names
            .get(&gid)
            .cloned()
            .unwrap_or_else(|| group.name.clone());
        let members = config
            .profiles
            .iter()
            .filter(|p| p.group.as_ref() == Some(&gid))
            .count();
        let arrow = |glyph: &str, on: bool, up: bool| -> Element<'a, Message> {
            button(text(glyph.to_owned()).size(12).color(look.dim))
                .padding([3, 8])
                .style(move |_theme: &iced::Theme, status| {
                    controls::flat_button(look, status, false)
                })
                .on_press_maybe(on.then(|| msg(ProfilesPageMsg::MoveGroup(gid.clone(), up))))
                .into()
        };
        let input_id = gid.clone();
        let submit_id = gid.clone();
        list = list.push(
            row![
                field(look, "Group name", &name)
                    .on_input(move |t| msg(ProfilesPageMsg::GroupName(input_id.clone(), t)))
                    .on_submit(msg(ProfilesPageMsg::GroupCommit(submit_id)))
                    .width(Length::Fill),
                small(
                    look,
                    &match members {
                        1 => "1 profile".to_owned(),
                        n => format!("{n} profiles"),
                    }
                ),
                arrow("↑", i > 0, true),
                arrow("↓", i < final_index, false),
                controls::action(
                    "Delete",
                    ButtonKind::Secondary,
                    look,
                    Some(msg(ProfilesPageMsg::DeleteGroup(gid.clone())))
                ),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        );
    }
    list.padding([6, 8]).into()
}

/// The defaults editor: SSH or local, the profile editor's own sections.
fn defaults<'a>(page: &'a ProfilesPage, look: Look) -> Element<'a, Message> {
    let pill = |label: &str, kind: DefaultsKind| -> Element<'a, Message> {
        let on = page.kind == kind;
        let (bg, fg) = controls::row_colors(look, on, false);
        button(
            text(label.to_owned())
                .size(12)
                .font(look.text.chrome)
                .color(if on { fg } else { look.dim }),
        )
        .padding([4, 12])
        .style(move |_theme: &iced::Theme, status| {
            let mut style = controls::flat_button(look, status, false);
            if on {
                style.background = bg.map(Into::into);
            }
            style.border = Border {
                color: if on { look.accent } else { look.border },
                width: 1.0,
                radius: look.shape.control.into(),
            };
            style
        })
        .on_press(msg(ProfilesPageMsg::Kind(kind)))
        .into()
    };
    let mut body = column![
        controls::section_label(look, "Defaults for every profile"),
        small(
            look,
            "What a profile inherits when it leaves a field empty. Empty here means Fogu's own \
             default (shown in grey). Saved as you go; text is saved on a click elsewhere."
        ),
        row![
            pill("SSH", DefaultsKind::Ssh),
            pill("Local", DefaultsKind::Local)
        ]
        .spacing(6),
    ]
    .spacing(10);
    if let Some(editor) = &page.defaults {
        let form = crate::profile_editor::defaults_sections(editor, &BUILT_IN, look)
            .map(|f| msg(ProfilesPageMsg::Defaults(f)));
        body = body.push(form);
    }
    body.padding([6, 8]).into()
}

/// A new defaults editor on `config`'s `kind` defaults.
fn defaults_editor(config: &Config, kind: DefaultsKind) -> Box<ProfileEditor> {
    let (id, name, kind) = match kind {
        DefaultsKind::Ssh => (
            "defaults:ssh",
            "SSH defaults",
            ProfileKind::Ssh(config.profile_defaults.ssh.clone()),
        ),
        DefaultsKind::Local => (
            "defaults:local",
            "Local defaults",
            ProfileKind::Local(config.profile_defaults.local.clone()),
        ),
    };
    let draft = Profile {
        id: ProfileId(id.to_owned()),
        name: name.to_owned(),
        kind,
        ..Profile::default_shell()
    };
    let mut editor = ProfileEditor::new(draft, Source::Defaults);
    editor.picker = if crate::sftp::picker_available() {
        Ok(())
    } else {
        Err("No file picker on this system")
    };
    Box::new(editor)
}

impl App {
    /// The Profiles page is shown: its defaults editor is (re)made from the config.
    pub(crate) fn profiles_page_shown(&mut self) {
        let config = Arc::clone(&self.config);
        let hosts = self.profiles.ssh_hosts.profiles.clone();
        if let Some(page) = &mut self.settings {
            let mut editor = defaults_editor(&config, page.profiles.kind);
            editor.refresh(&config, &hosts);
            page.profiles.defaults = Some(editor);
        }
    }

    /// A choice on the Profiles page.
    pub(crate) fn on_profiles_page(&mut self, message: ProfilesPageMsg) -> Task<Message> {
        match message {
            ProfilesPageMsg::DefaultProfile(index) => return self.set_default_profile(index),
            ProfilesPageMsg::Connect(id) => {
                let entry = self.all_entries().into_iter().find(|e| e.profile.id == id);
                if let Some(entry) = entry {
                    let closed = self.close_settings();
                    return Task::batch([
                        closed,
                        self.launch(&entry.profile, crate::profiles::Launch::Tab),
                    ]);
                }
            }
            ProfilesPageMsg::Edit(id) => {
                let entry = self.all_entries().into_iter().find(|e| e.profile.id == id);
                if let Some(entry) = entry {
                    let closed = self.close_settings();
                    return Task::batch([closed, self.edit_entry(&entry)]);
                }
            }
            ProfilesPageMsg::New(ssh) => {
                let closed = self.close_settings();
                return Task::batch([closed, self.new_profile(ssh)]);
            }
            ProfilesPageMsg::Duplicate(id) => {
                let entry = self.all_entries().into_iter().find(|e| e.profile.id == id);
                if let Some(entry) = entry {
                    return self.duplicate(&entry);
                }
            }
            ProfilesPageMsg::Delete(id) => self.profiles.pending_delete = Some(id),
            ProfilesPageMsg::GroupName(id, name) => {
                if let Some(page) = &mut self.settings {
                    page.profiles.group_names.insert(id, name);
                }
            }
            ProfilesPageMsg::GroupCommit(id) => return self.commit_group_name(&id),
            ProfilesPageMsg::MoveGroup(id, up) => {
                return self.save_edits(&[ProfileEdit::MoveGroup { id, up }]);
            }
            ProfilesPageMsg::DeleteGroup(id) => {
                if let Some(page) = &mut self.settings {
                    page.confirm = Some(Confirm::DeleteGroup(id));
                }
            }
            ProfilesPageMsg::AddGroup => {
                let name = unique_group_name(&self.config);
                let group = fogu_core::Group {
                    id: fogu_core::config::unique_group_id(&self.config, &name),
                    name,
                };
                return self.save_edits(&[ProfileEdit::AddGroup(group)]);
            }
            ProfilesPageMsg::Kind(kind) => {
                let flushed = self.commit_defaults();
                if let Some(page) = &mut self.settings {
                    page.profiles.kind = kind;
                }
                self.profiles_page_shown();
                return flushed;
            }
            ProfilesPageMsg::Defaults(Field(id, value)) => return self.defaults_field(id, value),
            ProfilesPageMsg::Picked(id, Some(path)) => {
                return self.defaults_field(id, FieldValue::Picked(path));
            }
            ProfilesPageMsg::Picked(_, None) => {}
        }
        Task::none()
    }

    /// Sets (or, for Default shell, clears) `default_profile`.
    fn set_default_profile(&mut self, index: usize) -> Task<Message> {
        let entries = self.all_entries();
        let Some(entry) = choices(&entries).get(index).map(|e| (*e).clone()) else {
            return Task::none();
        };
        let id = entry.profile.id;
        let value = (id != Profile::default_shell().id).then_some(id);
        let edit = KeyEdit {
            key: "default_profile".to_owned(),
            value: value.as_ref().map(|id| TomlValue::Str(id.0.clone())),
        };
        let dir = config_dir(self.config_dir.as_deref());
        if let Err(err) = set_keys(&dir, &[], &[edit]) {
            return self.notify(
                Notification::new(Level::Error, "Settings not saved").detail(err.to_string()),
            );
        }
        Arc::make_mut(&mut self.config).default_profile = value;
        Task::none()
    }

    /// Saves a typed group name (a blank one is left as it was).
    fn commit_group_name(&mut self, id: &GroupId) -> Task<Message> {
        let Some(name) = self
            .settings
            .as_mut()
            .and_then(|p| p.profiles.group_names.remove(id))
        else {
            return Task::none();
        };
        let name = name.trim().to_owned();
        let unchanged = self
            .config
            .groups
            .iter()
            .any(|g| &g.id == id && g.name == name);
        if name.is_empty() || unchanged {
            return Task::none();
        }
        self.save_edits(&[ProfileEdit::RenameGroup {
            id: id.clone(),
            name,
        }])
    }

    /// Group names typed and not saved yet (a click elsewhere, a page switch, closing).
    pub(crate) fn flush_group_names(&mut self) -> Task<Message> {
        let ids: Vec<GroupId> = self
            .settings
            .as_ref()
            .map(|p| p.profiles.group_names.keys().cloned().collect())
            .unwrap_or_default();
        Task::batch(
            ids.iter()
                .map(|id| self.commit_group_name(id))
                .collect::<Vec<_>>(),
        )
    }

    /// A defaults field changed: applied to the draft; anything but typing is saved at once.
    fn defaults_field(&mut self, id: FieldId, value: FieldValue) -> Task<Message> {
        if matches!(value, FieldValue::Menu(..)) {
            return Task::none();
        }
        let typing = matches!(
            value,
            FieldValue::Text(_) | FieldValue::Item(..) | FieldValue::Value(..)
        );
        let config = Arc::clone(&self.config);
        let hosts = self.profiles.ssh_hosts.profiles.clone();
        let Some(editor) = self
            .settings
            .as_mut()
            .and_then(|p| p.profiles.defaults.as_mut())
        else {
            return Task::none();
        };
        let effect = editor.apply(id, value);
        editor.refresh(&config, &hosts);
        if let Effect::Browse(field) = effect {
            return pick(field);
        }
        if typing {
            return Task::none();
        }
        self.commit_defaults()
    }

    /// Writes the defaults draft when it changed and every field parses.
    pub(crate) fn commit_defaults(&mut self) -> Task<Message> {
        let Some(editor) = self
            .settings
            .as_mut()
            .and_then(|p| p.profiles.defaults.as_mut())
        else {
            return Task::none();
        };
        if !editor.dirty() || !editor.errors.is_empty() {
            return Task::none();
        }
        let old = self.config.profile_defaults.clone();
        let mut new = old.clone();
        match &editor.draft.kind {
            ProfileKind::Local(local) => new.local = local.clone(),
            ProfileKind::Ssh(ssh) => new.ssh = ssh.clone(),
        }
        editor.original = editor.draft.clone();
        let dir = config_dir(self.config_dir.as_deref());
        if let Err(err) = fogu_core::config::set_profile_defaults(&dir, &old, &new) {
            return self.notify(
                Notification::new(Level::Error, "Defaults not saved").detail(err.to_string()),
            );
        }
        Arc::make_mut(&mut self.config).profile_defaults = new;
        Task::none()
    }
}

/// "New group", or "New group 2", … — a name no group has.
fn unique_group_name(config: &Config) -> String {
    let taken = |name: &str| config.groups.iter().any(|g| g.name == name);
    if !taken("New group") {
        return "New group".to_owned();
    }
    (2..=u32::MAX)
        .map(|n| format!("New group {n}"))
        .find(|name| !taken(name))
        .unwrap_or_else(|| "New group".to_owned())
}

/// The file (or folder) picker of defaults field `field`.
fn pick(field: FieldId) -> Task<Message> {
    let dialog = rfd::AsyncFileDialog::new();
    let done = move |f: Option<rfd::FileHandle>| {
        msg(ProfilesPageMsg::Picked(
            field,
            f.map(|f| f.path().to_path_buf()),
        ))
    };
    match field {
        FieldId::LocalCwd => Task::perform(dialog.set_title("Start directory").pick_folder(), done),
        FieldId::PrivateKeys => {
            let mut dialog = dialog.set_title("Private key");
            if let Some(ssh_dir) = dirs::home_dir().map(|h| h.join(".ssh")) {
                dialog = dialog.set_directory(ssh_dir);
            }
            Task::perform(dialog.pick_file(), done)
        }
        _ => Task::perform(dialog.set_title("Choose a file").pick_file(), done),
    }
}
