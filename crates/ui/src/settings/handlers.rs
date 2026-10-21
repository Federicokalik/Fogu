//! The settings page's update logic: one path writes any setting to `config.toml` and applies
//! it to the running app; plus search, the code editor, the outside click and the keyboard.

use std::sync::Arc;

use fogu_core::config::{KeyEdit, config_dir, set_keys};
use fogu_core::{Config, SchemeCatalog, StyleCatalog};
use iced::Task;
use iced::keyboard::key::Named;
use iced::keyboard::{Key, Modifiers};
use iced::widget::Id;

use super::index;
use super::spec::{Kind, SettingId, Value, from_i64, parse_number};
use super::{Confirm, Edit, HIGHLIGHT, SCROLL_ID, SettingsMsg, SettingsPage, Target};
use crate::app::App;
use crate::design::resolve_style;
use crate::message::Message;
use crate::modal::{Modal, ModalButton};
use crate::notifications::{Level, Notification};
use crate::style_code::CodeKind;

impl App {
    /// Opens the settings page (or brings it back to its first page).
    pub(crate) fn open_settings(&mut self) -> Task<Message> {
        self.settings = Some(SettingsPage {
            fonts: Some(super::FontPickers::new(crate::metrics::monospace_families())),
            ..SettingsPage::default()
        });
        Task::none()
    }

    /// Closes the settings page, saving what was typed into its fields.
    pub(crate) fn close_settings(&mut self) -> Task<Message> {
        let task = self.flush_settings();
        self.settings = None;
        task
    }

    /// The Open Settings shortcut: opens the page, or closes it when open.
    pub(crate) fn toggle_settings(&mut self) -> Task<Message> {
        if self.settings.is_some() {
            return self.close_settings();
        }
        self.open_settings()
    }

    /// Applies and persists one settings choice.
    #[expect(
        clippy::too_many_lines,
        reason = "one exhaustive settings dispatch; each arm is a short delegation"
    )]
    pub(crate) fn on_settings(&mut self, message: SettingsMsg) -> Task<Message> {
        let mut tasks = Vec::new();
        match message {
            SettingsMsg::Section(section) => {
                tasks.push(self.flush_settings());
                if let Some(page) = &mut self.settings {
                    page.section = section;
                    page.query.clear();
                    page.highlight = None;
                }
                if section == super::Section::Profiles {
                    self.profiles_page_shown();
                }
                tasks.push(iced::widget::operation::snap_to(
                    Id::new(SCROLL_ID),
                    iced::widget::operation::RelativeOffset::START,
                ));
            }
            SettingsMsg::Query(query) => {
                if let Some(page) = &mut self.settings {
                    page.query = query;
                    page.selected = 0;
                }
            }
            SettingsMsg::SearchSubmit => {
                let hit = self.settings.as_ref().and_then(|page| {
                    index::search(&page.query)
                        .get(page.selected)
                        .map(|hit| hit.target)
                });
                if let Some(target) = hit {
                    return self.jump(target);
                }
            }
            SettingsMsg::Jump(target) => return self.jump(target),
            SettingsMsg::Reveal(target) => {
                return super::reveal::reveal(Id::new(SCROLL_ID), target.id());
            }
            SettingsMsg::Unhighlight(generation) => {
                if let Some(page) = self
                    .settings
                    .as_mut()
                    .filter(|p| p.highlight_gen == generation)
                {
                    page.highlight = None;
                }
            }
            SettingsMsg::Set(id, edit) => return self.edit_setting(id, edit),
            SettingsMsg::Shortcut(message) => return self.on_shortcut(message),
            SettingsMsg::Profiles(message) => return self.on_profiles_page(message),
            SettingsMsg::Outside => return self.settings_outside(),
            SettingsMsg::Confirm(yes) => {
                let confirm = self.settings.as_mut().and_then(|p| p.confirm.take());
                match confirm {
                    Some(Confirm::DeleteGroup(id)) if yes => {
                        return self.save_edits(&[fogu_core::config::ProfileEdit::DeleteGroup(id)]);
                    }
                    Some(Confirm::DiscardCode) if yes => {
                        if let Some(page) = &mut self.settings {
                            page.editor = None;
                        }
                        return self.close_settings();
                    }
                    Some(Confirm::ResetShortcuts) if yes => {
                        return self.save_hotkeys(fogu_core::Hotkeys::default_keymap());
                    }
                    _ => {}
                }
            }
            SettingsMsg::Style(key) => {
                if let Some(task) = self.switch_style(&key) {
                    tasks.push(task);
                }
            }
            SettingsMsg::NewCode(kind) => {
                if let Some(note) = self.open_code(kind, None) {
                    tasks.push(self.notify(note));
                }
            }
            SettingsMsg::EditCode(kind, name) => {
                if let Some(note) = self.open_code(kind, Some(&name)) {
                    tasks.push(self.notify(note));
                }
            }
            SettingsMsg::Code(action) => {
                let (styles, schemes) = (&self.styles, &self.schemes);
                if let Some(editor) = self.settings.as_mut().and_then(|p| p.editor.as_mut()) {
                    let edit = action.is_edit();
                    editor.content.perform(action);
                    if edit {
                        editor.recheck(styles, schemes);
                    }
                }
            }
            SettingsMsg::SaveCode => tasks.extend(self.save_code()),
            SettingsMsg::CloseCode => {
                if let Some(page) = &mut self.settings {
                    page.editor = None;
                }
            }
            SettingsMsg::UseMatchingPalette => {
                let (style, _) = resolve_style(&self.config.appearance.ui_theme, &self.styles);
                if let Some(paired) = style.paired_scheme.clone() {
                    return self.on_settings(SettingsMsg::Scheme(paired));
                }
            }
            SettingsMsg::Accent(accent) => {
                Arc::make_mut(&mut self.config).appearance.accent = accent;
                self.refresh_look();
                if let Some(note) = self.save_ui() {
                    tasks.push(self.notify(note));
                }
            }
            SettingsMsg::Scheme(name) => {
                Arc::make_mut(&mut self.config).appearance.color_scheme = name;
                if let Some(note) = self.apply_scheme() {
                    tasks.push(self.notify(note));
                }
                let dir = config_dir(self.config_dir.as_deref());
                if let Err(err) = fogu_core::config::set_appearance_scheme(
                    &dir,
                    &self.config.appearance.color_scheme,
                ) {
                    tasks.push(self.notify(not_saved(&err)));
                }
            }
            SettingsMsg::Reload => {
                for note in self.reload_catalogs() {
                    tasks.push(self.notify(note));
                }
            }
        }
        Task::batch(tasks)
    }

    /// Shows `target`'s page, highlights it and scrolls it into view.
    fn jump(&mut self, target: Target) -> Task<Message> {
        let flush = self.flush_settings();
        let Some(page) = &mut self.settings else {
            return flush;
        };
        page.section = target.section();
        page.query.clear();
        if let Target::Action(action) = target {
            page.shortcuts.query = super::shortcuts::label(action);
        }
        page.highlight = Some(target);
        page.highlight_gen += 1;
        let generation = page.highlight_gen;
        if target.section() == super::Section::Profiles {
            self.profiles_page_shown();
        }
        // The reveal waits a frame: the new page must be laid out before it is measured.
        Task::batch([
            flush,
            self.after(
                std::time::Duration::from_millis(30),
                Message::Settings(SettingsMsg::Reveal(target)),
            ),
            self.after(
                HIGHLIGHT,
                Message::Settings(SettingsMsg::Unhighlight(generation)),
            ),
        ])
    }

    /// One control of setting `id` changed.
    fn edit_setting(&mut self, id: SettingId, edit: Edit) -> Task<Message> {
        let spec = id.spec();
        match edit {
            Edit::Bool(on) => self.commit_setting(id, &Value::Bool(on)),
            Edit::Choice(index) => self.commit_setting(id, &Value::Choice(index)),
            Edit::Auto(on) => {
                let value = if on {
                    Value::Auto
                } else {
                    match spec.kind {
                        Kind::Slider { min, max, .. } => Value::Num(f64::from(min.midpoint(max))),
                        _ => Value::Auto,
                    }
                };
                self.commit_setting(id, &value)
            }
            Edit::Slide(n) => {
                // Applied while dragging; written once on release.
                let old = (*self.config).clone();
                id.set(Arc::make_mut(&mut self.config), &Value::Num(n));
                self.apply_config_change(&old)
            }
            Edit::SlideDone => {
                let value = id.get(&self.config);
                self.write_setting(id, &value)
                    .map_or_else(Task::none, |note| self.notify(note))
            }
            Edit::Text(text) => {
                if let Some(page) = &mut self.settings {
                    match check(id, &text) {
                        Ok(_) => page.errors.remove(&id),
                        Err(why) => page.errors.insert(id, why),
                    };
                    page.pending.insert(id, text);
                }
                Task::none()
            }
            Edit::Submit => self.flush_setting(id),
            Edit::Flag(i) => {
                let Value::Flags(mut flags) = id.get(&self.config) else {
                    return Task::none();
                };
                if let Some(flag) = flags.get_mut(i) {
                    *flag = !*flag;
                }
                self.commit_setting(id, &Value::Flags(flags))
            }
            Edit::Pick(name) => {
                if let Some(page) = &mut self.settings {
                    page.pending.remove(&id);
                    page.errors.remove(&id);
                }
                self.commit_setting(id, &Value::Text(name))
            }
            Edit::Add(name) => {
                let mut items = id.get(&self.config).list().to_vec();
                if items.contains(&name) {
                    return Task::none();
                }
                items.push(name);
                self.commit_setting(id, &Value::List(items))
            }
            Edit::Remove(i) => {
                let mut items = id.get(&self.config).list().to_vec();
                if i < items.len() {
                    items.remove(i);
                }
                self.commit_setting(id, &Value::List(items))
            }
        }
    }

    /// Saves every field's typed text that is valid (a page switch, closing).
    pub(crate) fn flush_settings(&mut self) -> Task<Message> {
        let ids: Vec<SettingId> = self
            .settings
            .as_ref()
            .map(|p| p.pending.keys().copied().collect())
            .unwrap_or_default();
        let fields: Vec<Task<Message>> = ids.into_iter().map(|id| self.flush_setting(id)).collect();
        Task::batch(
            fields
                .into_iter()
                .chain([self.flush_group_names(), self.commit_defaults()]),
        )
    }

    /// A click anywhere: saves the typed text of plain fields. A font field is left alone: the
    /// click may be on one of its suggestions, and it saves itself when its list closes.
    pub(crate) fn flush_settings_on_click(&mut self) -> Task<Message> {
        let ids: Vec<SettingId> = self
            .settings
            .as_ref()
            .map(|p| {
                p.pending
                    .keys()
                    .copied()
                    .filter(|id| !matches!(id.spec().kind, Kind::Font))
                    .collect()
            })
            .unwrap_or_default();
        let fields: Vec<Task<Message>> = ids.into_iter().map(|id| self.flush_setting(id)).collect();
        Task::batch(
            fields
                .into_iter()
                .chain([self.flush_group_names(), self.commit_defaults()]),
        )
    }

    /// Saves field `id`'s typed text when valid; an invalid text stays with its error.
    fn flush_setting(&mut self, id: SettingId) -> Task<Message> {
        let Some(text) = self
            .settings
            .as_ref()
            .and_then(|p| p.pending.get(&id))
            .cloned()
        else {
            return Task::none();
        };
        match check(id, &text) {
            Ok(value) => {
                if let Some(page) = &mut self.settings {
                    page.pending.remove(&id);
                    page.errors.remove(&id);
                }
                if value == id.get(&self.config) {
                    return Task::none();
                }
                self.commit_setting(id, &value)
            }
            Err(why) => {
                if let Some(page) = &mut self.settings {
                    page.errors.insert(id, why);
                }
                Task::none()
            }
        }
    }

    /// Writes setting `id` = `value` to the file, then applies it. A failed write changes
    /// nothing and says why.
    pub(crate) fn commit_setting(&mut self, id: SettingId, value: &Value) -> Task<Message> {
        if let Some(note) = self.write_setting(id, value) {
            return self.notify(note);
        }
        let old = (*self.config).clone();
        id.set(Arc::make_mut(&mut self.config), value);
        self.apply_config_change(&old)
    }

    /// Writes setting `id` = `value`; the error toast when it failed.
    fn write_setting(&self, id: SettingId, value: &Value) -> Option<Notification> {
        let spec = id.spec();
        let edit = KeyEdit {
            key: spec.key.to_owned(),
            value: id.toml(value),
        };
        let dir = config_dir(self.config_dir.as_deref());
        set_keys(&dir, spec.path, &[edit])
            .err()
            .map(|err| not_saved(&err))
    }

    /// Brings the running app in line with the config after it changed from `old`.
    pub(crate) fn apply_config_change(&mut self, old: &Config) -> Task<Message> {
        let new = Arc::clone(&self.config);
        let (a, was) = (&new.appearance, &old.appearance);
        let mut notes = Vec::new();
        if a.color_scheme != was.color_scheme {
            notes.extend(self.apply_scheme());
        } else if a.ui_theme != was.ui_theme || a.accent != was.accent {
            self.refresh_look();
        }
        if a.font_family != was.font_family || a.fallback_fonts != was.fallback_fonts {
            notes.extend(self.reresolve_font());
        }
        // A new size in Settings also ends any zoom.
        if a.font_size.to_bits() != was.font_size.to_bits() {
            self.set_font_size(a.font_size);
        } else if a.line_height.to_bits() != was.line_height.to_bits()
            || a.padding_x.to_bits() != was.padding_x.to_bits()
            || a.padding_y.to_bits() != was.padding_y.to_bits()
        {
            self.apply_metrics();
        }
        if a.profiles_sidebar_width.to_bits() != was.profiles_sidebar_width.to_bits() {
            self.profiles.width = a.profiles_sidebar_width.max(crate::profiles::MIN_WIDTH);
        }
        let mut tasks = Vec::new();
        if new.ssh.read_ssh_config != old.ssh.read_ssh_config {
            self.profiles.ssh_hosts.forget();
            tasks.push(self.refresh_ssh_hosts());
        }
        let (t, before) = (&new.terminal, &old.terminal);
        if t.scrollback_lines != before.scrollback_lines
            || t.word_separators != before.word_separators
            || a.cursor_shape != was.cursor_shape
        {
            self.apply_term_options();
        }
        tasks.extend(notes.into_iter().map(|n| self.notify(n)));
        Task::batch(tasks)
    }

    /// A press on the dimmed window around the page: closes it, unless a shortcut is being
    /// recorded (that is cancelled instead) or the code editor holds unsaved changes (then it
    /// asks).
    fn settings_outside(&mut self) -> Task<Message> {
        let Some(page) = &mut self.settings else {
            return Task::none();
        };
        if page.confirm.is_some() {
            return Task::none();
        }
        if page.shortcuts.capture.take().is_some() {
            return Task::none();
        }
        if page.editor_dirty() {
            page.confirm = Some(Confirm::DiscardCode);
            return Task::none();
        }
        self.close_settings()
    }

    /// The page's question, drawn over it.
    pub(crate) fn settings_modal(&self) -> Option<Modal> {
        let answer = |yes| Message::Settings(SettingsMsg::Confirm(yes));
        match self.settings.as_ref()?.confirm.clone()? {
            Confirm::DeleteGroup(id) => {
                let name = self
                    .config
                    .groups
                    .iter()
                    .find(|g| g.id == id)
                    .map_or_else(|| id.0.clone(), |g| g.name.clone());
                Some(Modal {
                    title: "Delete group?",
                    body: format!(
                        "\"{}\" is removed; its profiles stay, without a group.",
                        fogu_term::sanitize_title(&name)
                    ),
                    buttons: vec![
                        ModalButton {
                            label: "Delete",
                            on_press: answer(true),
                        },
                        ModalButton {
                            label: "Cancel",
                            on_press: answer(false),
                        },
                    ],
                })
            }
            Confirm::ResetShortcuts => Some(Modal {
                title: "Reset all shortcuts?",
                body: "Every action gets its default keys back; your changes in [hotkeys] are \
                       removed."
                    .to_owned(),
                buttons: vec![
                    ModalButton {
                        label: "Reset",
                        on_press: answer(true),
                    },
                    ModalButton {
                        label: "Cancel",
                        on_press: answer(false),
                    },
                ],
            }),
            Confirm::DiscardCode => Some(Modal {
                title: "Discard changes?",
                body: "The code editor has unsaved changes.".to_owned(),
                buttons: vec![
                    ModalButton {
                        label: "Discard",
                        on_press: answer(true),
                    },
                    ModalButton {
                        label: "Cancel",
                        on_press: answer(false),
                    },
                ],
            }),
        }
    }

    /// Escape while the page is open: a shortcut being recorded or its question, the search
    /// text, then the code editor (asking when it has unsaved changes), then the page.
    pub(crate) fn settings_escape(&mut self) -> Task<Message> {
        let Some(page) = &mut self.settings else {
            return Task::none();
        };
        if page.shortcuts.capture.take().is_some() || page.shortcuts.notice.take().is_some() {
            return Task::none();
        }
        if !page.query.is_empty() {
            page.query.clear();
            return Task::none();
        }
        if page.editor.is_some() {
            if page.editor_dirty() {
                page.confirm = Some(Confirm::DiscardCode);
            } else {
                page.editor = None;
            }
            return Task::none();
        }
        self.close_settings()
    }

    /// A key no widget took while the page is open: a shortcut being recorded, Open Settings
    /// (closes the page), search result navigation. Everything else is swallowed so nothing
    /// hidden behind the page reacts.
    pub(crate) fn settings_key(
        &mut self,
        key: &Key,
        modified: &Key,
        mods: Modifiers,
    ) -> Task<Message> {
        if let Some(task) = self.shortcut_key(key, mods) {
            return task;
        }
        if crate::keys::hotkey(&self.config.hotkeys, key, modified, mods)
            == Some(fogu_core::Action::OpenSettings)
        {
            return self.close_settings();
        }
        let Some(page) = &mut self.settings else {
            return Task::none();
        };
        let hits = index::search(&page.query);
        match key {
            Key::Named(Named::ArrowDown) if !hits.is_empty() => {
                page.selected = (page.selected + 1).min(hits.len() - 1);
            }
            Key::Named(Named::ArrowUp) => page.selected = page.selected.saturating_sub(1),
            Key::Named(Named::Enter) => {
                if let Some(hit) = hits.get(page.selected) {
                    let target = hit.target;
                    return self.jump(target);
                }
            }
            _ => {}
        }
        Task::none()
    }

    fn save_ui(&self) -> Option<Notification> {
        let dir = config_dir(self.config_dir.as_deref());
        let (theme, accent) = {
            let appearance = &self.config.appearance;
            (appearance.ui_theme.clone(), appearance.accent)
        };
        fogu_core::config::set_appearance_ui(&dir, &theme, accent)
            .err()
            .map(|err| not_saved(&err))
    }

    /// Applies and saves UI style `key`. The palette follows the style while it is the default
    /// or another style's pair; a palette the user picked stays.
    fn switch_style(&mut self, key: &str) -> Option<Task<Message>> {
        let follow = self.styles.resolve(key).and_then(|style| {
            crate::design::suggested_scheme(
                &self.config.appearance.color_scheme,
                style,
                &self.styles,
                &self.schemes,
            )
        });
        let appearance = &mut Arc::make_mut(&mut self.config).appearance;
        key.clone_into(&mut appearance.ui_theme);
        let mut notes = Vec::new();
        if let Some(scheme) = &follow {
            appearance.color_scheme.clone_from(scheme);
            notes.extend(self.apply_scheme());
        } else {
            self.refresh_look();
        }
        let dir = config_dir(self.config_dir.as_deref());
        let appearance = &self.config.appearance;
        if let Err(err) = fogu_core::config::set_appearance(
            &dir,
            &appearance.ui_theme,
            appearance.accent,
            follow.as_deref(),
        ) {
            notes.push(not_saved(&err));
        }
        let tasks: Vec<Task<Message>> = notes.into_iter().map(|note| self.notify(note)).collect();
        (!tasks.is_empty()).then(|| Task::batch(tasks))
    }

    /// Re-reads `styles/` and `themes/` and re-applies the configured style and scheme; returns
    /// what to report (skipped files, a style or scheme that is gone).
    fn reload_catalogs(&mut self) -> Vec<Notification> {
        let dir = config_dir(self.config_dir.as_deref());
        self.schemes = SchemeCatalog::load(&dir);
        self.styles = StyleCatalog::load(&dir);
        let mut notes: Vec<Notification> = self
            .schemes
            .warnings()
            .iter()
            .map(|w| Notification::new(Level::Warn, "Colour scheme skipped").detail(w.to_string()))
            .chain(
                self.styles.warnings().iter().map(|w| {
                    Notification::new(Level::Warn, "UI style skipped").detail(w.to_string())
                }),
            )
            .collect();
        notes.extend(self.apply_scheme());
        if let (_, Some(note)) = resolve_style(&self.config.appearance.ui_theme, &self.styles) {
            notes.push(note);
        }
        notes
    }

    /// Opens the settings code editor: on the file of custom style (key) or palette (name)
    /// `name`, or on a new file based on the active style or palette.
    fn open_code(&mut self, kind: CodeKind, name: Option<&str>) -> Option<Notification> {
        use crate::style_code::{CodeEditor, palette_template, style_template};
        let (text, editing) = if let Some(name) = name {
            let path = match kind {
                CodeKind::Style => self.styles.source(name),
                CodeKind::Palette => self.schemes.source(name),
            }?
            .to_path_buf();
            match std::fs::read_to_string(&path) {
                Ok(text) => (text, Some(path)),
                Err(err) => {
                    return Some(
                        Notification::new(Level::Error, "Cannot open the file")
                            .detail(format!("{}: {err}", path.display())),
                    );
                }
            }
        } else {
            let text = match kind {
                CodeKind::Style => {
                    let (style, _) = resolve_style(&self.config.appearance.ui_theme, &self.styles);
                    style_template(style, &self.styles)
                }
                CodeKind::Palette => {
                    let (scheme, _) = crate::appearance::resolve_scheme(
                        &self.config.appearance.color_scheme,
                        &self.schemes,
                    );
                    palette_template(&scheme, &self.schemes)
                }
            };
            (text, None)
        };
        let editor = CodeEditor::new(kind, &text, editing, (&self.styles, &self.schemes));
        let page = self.settings.get_or_insert_with(Default::default);
        page.editor_original = editor.content.text();
        page.editor = Some(editor);
        None
    }

    /// Writes the code editor's file, reloads the catalogs and applies what was saved. A failed
    /// write stays in the editor with the reason.
    fn save_code(&mut self) -> Vec<Task<Message>> {
        let Some(editor) = self.settings.as_mut().and_then(|p| p.editor.as_mut()) else {
            return Vec::new();
        };
        let dir = config_dir(self.config_dir.as_deref());
        let text = editor.content.text();
        let kind = editor.kind;
        let saved = match kind {
            CodeKind::Style => self.styles.save(&dir, &text, editor.editing.as_deref()),
            CodeKind::Palette => self.schemes.save(&dir, &text, editor.editing.as_deref()),
        };
        let (name, path) = match saved {
            Ok(saved) => saved,
            Err(why) => {
                editor.error = Some(why);
                return Vec::new();
            }
        };
        if let Some(page) = &mut self.settings {
            page.editor = None;
        }
        let mut notes = self.reload_catalogs();
        notes.push(Notification::new(Level::Info, "Saved").detail(path.display().to_string()));
        let mut tasks: Vec<Task<Message>> = notes.into_iter().map(|n| self.notify(n)).collect();
        let apply = match kind {
            CodeKind::Style => SettingsMsg::Style(name),
            CodeKind::Palette => SettingsMsg::Scheme(name),
        };
        tasks.push(self.on_settings(apply));
        tasks
    }
}

/// The value field `id`'s `text` stands for, or why it is not valid.
fn check(id: SettingId, text: &str) -> Result<Value, String> {
    match id.spec().kind {
        Kind::Number { min, max } => parse_number(text, min, max).map(|n| Value::Num(from_i64(n))),
        Kind::Font if text.trim().is_empty() => Err("a font family name".to_owned()),
        _ => Ok(Value::Text(text.to_owned())),
    }
}

/// The toast for a failed config write.
fn not_saved(err: &fogu_core::config::ConfigError) -> Notification {
    Notification::new(Level::Error, "Settings not saved").detail(err.to_string())
}
