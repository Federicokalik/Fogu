//! Application state, `update` dispatch, `view` and subscriptions. Tab lifecycle and actions live
//! in `actions.rs`, split/pane operations and resizing in `panes.rs`; pane-level input (keys,
//! paste, selection) here.

use std::sync::Arc;

use iced::keyboard;
use std::time::{Duration, Instant};

use fogu_core::{Config, SchemeCatalog, StyleCatalog};
use fogu_session::SessionRuntime;
use fogu_term::input;
use iced::{Point, Task, clipboard, window};

use crate::RunOptions;
use crate::appearance::resolve_scheme;
use crate::background;
use crate::blink::{self, Blink};
use crate::context_menu::ContextMenu;
use crate::design::{Env, resolve_style};
use crate::lifecycle::PendingClipboard;
use crate::message::{Message, PasteSource, SelectEvent, WindowCmd};
use crate::metrics::{self, CellMetrics, FontSpec, ResolvedFont, viewport_to_point};
use crate::notifications::{AUTO_DISMISS, Level, Notification, Toasts};
use crate::palette::{Colors, Look};
use crate::panes::{CloseTarget, CwdQuery};
use crate::paste::PendingPaste;
use crate::profile_picker::ProfilePicker;
use crate::profiles::ProfilesView;
use crate::restart;
use crate::sftp;
use crate::tab::{PaneId, Tab, TabId};
use crate::tab_bar::Rename;

pub(crate) const DEFAULT_TITLE: &str = "Fogu";
/// Smallest font size zoom allows, in points.
const MIN_FONT_PT: f32 = 6.0;
/// Largest font size zoom allows, in points.
const MAX_FONT_PT: f32 = 40.0;
/// Initial window size (also the menu-clamping size until the first resize event).
pub(crate) const WINDOW_SIZE: iced::Size = iced::Size::new(1000.0, 650.0);

/// The whole application state.
#[derive(Debug)]
pub struct App {
    pub(crate) runtime: Arc<SessionRuntime>,
    pub(crate) config: Arc<Config>,
    pub(crate) tabs: Vec<Tab>,
    /// The visible tab; meaningless only while `tabs` is empty (the app is exiting).
    pub(crate) active: TabId,
    pub(crate) menu: Option<ContextMenu>,
    pub(crate) renaming: Option<Rename>,
    pub(crate) hovered_tab: Option<TabId>,
    /// Last pointer position over the tab bar: the tab menu's anchor.
    pub(crate) tab_bar_cursor: Point,
    /// Where the bar's empty area was pressed, until the pointer moves (then the window moves).
    pub(crate) bar_press: Option<Point>,
    /// The tab strip's last reported viewport (`None` until it first overflows).
    pub(crate) tab_strip: Option<iced::widget::scrollable::Viewport>,
    /// A press on a tab, pane grip or sidebar profile that may become a drag.
    pub(crate) drag: Option<crate::drag::Drag>,
    pub(crate) window_size: iced::Size,
    pub(crate) fullscreen: bool,
    /// The cwd query in flight (`NewTabHere`, splits).
    pub(crate) pending_cwd: Option<CwdQuery>,
    /// Source of every `TabId`/`PaneId`.
    pub(crate) next_id: u64,
    pub(crate) font: FontSpec,
    /// `config.appearance.font_size` at boot, in points; `ZoomReset`'s target.
    default_font_size_pt: f32,
    pub(crate) resolved: ResolvedFont,
    pub(crate) metrics: CellMetrics,
    pub(crate) scale: f32,
    pub(crate) colors: Colors,
    /// Built-in plus custom terminal colour schemes, reloadable from Settings.
    pub(crate) schemes: SchemeCatalog,
    /// Built-in plus custom UI styles, reloadable from Settings.
    pub(crate) styles: StyleCatalog,
    /// What the UI style needs besides the scheme (terminal font, software rendering).
    pub(crate) env: Env,
    /// Chrome colours and the matching iced theme, both from `colors`.
    pub(crate) look: Look,
    pub(crate) theme: iced::Theme,
    pub(crate) focused: bool,
    pub(crate) pending_paste: Option<PendingPaste>,
    pub(crate) toasts: Toasts,
    pub(crate) pending_clipboard: Vec<PendingClipboard>,
    pub(crate) blink: Blink,
    pub(crate) picker: Option<ProfilePicker>,
    /// The profile manager sidebar.
    pub(crate) profiles: ProfilesView,
    /// The settings page, when open.
    pub(crate) settings: Option<crate::settings::SettingsPage>,
    /// The "Port forwarding" dialog of an SSH pane.
    pub(crate) forwards: Option<crate::forwards_dialog::ForwardsDialog>,
    /// The editor's "Set stored secret" dialog.
    pub(crate) secret_dialog: Option<crate::secret_dialog::SecretDialog>,
    pub(crate) search: Option<crate::search::Search>,
    /// A close waiting for the "close live SSH session?" confirmation.
    pub(crate) pending_close: Option<CloseTarget>,
    /// `--config-dir` (SSH `known_hosts` location); `None` = the default.
    pub(crate) config_dir: Option<std::path::PathBuf>,
    /// Modifier keys currently held (from the window's keyboard events).
    pub(crate) modifiers: keyboard::Modifiers,
    /// Dropped files and desktop hints (SFTP panel).
    pub(crate) desktop: sftp::Desktop,
    /// Decoded tab background images.
    pub(crate) backgrounds: background::Cache,
    pub(crate) exit_after_first_frame: bool,
    /// Tab recovery saving state (`None`: off, no state directory, or `--no-recovery`).
    pub(crate) recovery: Option<crate::recovery::Recovery>,
    /// The pane whose context menu is open (its "Current process" line updates).
    pub(crate) pane_menu: Option<PaneId>,
    /// A profile editor tab with unsaved changes waits for "Discard changes?".
    pub(crate) pending_discard: Option<TabId>,
    /// Test builds: input ends of the stub sessions (what the UI sent) and the effective
    /// profile each stub session was opened with.
    #[cfg(test)]
    pub(crate) stub_inputs: Vec<tokio::sync::mpsc::Receiver<fogu_session::SessionInput>>,
    #[cfg(test)]
    pub(crate) stub_profiles: Vec<fogu_core::EffectiveProfile>,
    /// Test builds: command ends of the SSH stubs' SFTP handles.
    #[cfg(test)]
    pub(crate) stub_sftp: Vec<tokio::sync::mpsc::Receiver<fogu_session::SftpRequest>>,
}

/// `~/.ssh/config` hosts; test builds don't read the machine's file.
fn ssh_hosts() -> crate::profiles::SshHosts {
    if cfg!(test) {
        return crate::profiles::SshHosts::default();
    }
    crate::profiles::SshHosts::from_process()
}

/// "Default shell" + the installed shells; test builds don't depend on the machine's shell
/// list.
fn builtins() -> Vec<fogu_core::Profile> {
    if cfg!(test) {
        return vec![fogu_core::Profile::default_shell()];
    }
    fogu_core::builtin::builtin_shell_profiles()
}

impl App {
    /// State with no tabs yet.
    #[cfg(test)]
    pub(crate) fn new(
        runtime: Arc<SessionRuntime>,
        config: Arc<Config>,
        resolved: ResolvedFont,
    ) -> Self {
        Self::with_catalogs(
            runtime,
            config,
            resolved,
            SchemeCatalog::builtins(),
            StyleCatalog::builtins(),
            false,
        )
    }

    /// [`new`](Self::new) with the custom terminal schemes and UI styles found at startup.
    pub(crate) fn with_catalogs(
        runtime: Arc<SessionRuntime>,
        config: Arc<Config>,
        resolved: ResolvedFont,
        schemes: SchemeCatalog,
        styles: StyleCatalog,
        software: bool,
    ) -> Self {
        let font = FontSpec {
            family: config.appearance.font_family.clone(),
            size_pt: config.appearance.font_size,
        };
        let metrics = CellMetrics::new(resolved.units, font.size_px(), 1.0);
        let (scheme, scheme_note) = resolve_scheme(&config.appearance.color_scheme, &schemes);
        let colors = Colors::new(&scheme);
        let env = Env {
            mono: resolved.font,
            software,
        };
        let (style, preset_note) = resolve_style(&config.appearance.ui_theme, &styles);
        let look = Look::new(&colors, style, config.appearance.accent, env);
        let mut toasts = Toasts::default();
        if let Some(note) = scheme_note {
            toasts.push(note);
        }
        if let Some(note) = preset_note {
            toasts.push(note);
        }
        Self {
            runtime,
            tabs: Vec::new(),
            active: TabId(0),
            menu: None,
            renaming: None,
            hovered_tab: None,
            tab_bar_cursor: Point::ORIGIN,
            bar_press: None,
            tab_strip: None,
            drag: None,
            window_size: WINDOW_SIZE,
            fullscreen: false,
            pending_cwd: None,
            next_id: 1,
            default_font_size_pt: font.size_pt,
            font,
            resolved,
            metrics,
            scale: 1.0,
            look,
            theme: look.theme(&colors),
            colors,
            schemes,
            styles,
            env,
            focused: true,
            pending_paste: None,
            toasts,
            pending_clipboard: Vec::new(),
            blink: Blink::new(Instant::now()),
            picker: None,
            profiles: ProfilesView {
                ssh_hosts: ssh_hosts(),
                ..ProfilesView::new(config.appearance.profiles_sidebar_width, builtins())
            },
            settings: None,
            pending_close: None,
            forwards: None,
            secret_dialog: None,
            search: None,
            config_dir: None,
            modifiers: keyboard::Modifiers::default(),
            desktop: sftp::Desktop::default(),
            backgrounds: background::Cache::default(),
            exit_after_first_frame: false,
            recovery: None,
            pane_menu: None,
            pending_discard: None,
            config,
            #[cfg(test)]
            stub_inputs: Vec::new(),
            #[cfg(test)]
            stub_profiles: Vec::new(),
            #[cfg(test)]
            stub_sftp: Vec::new(),
        }
    }

    /// Opens the default profile in the first tab, queues the startup toasts and asks for the
    /// window's scale factor.
    pub fn boot(
        runtime: Arc<SessionRuntime>,
        config: Arc<Config>,
        options: RunOptions,
    ) -> (Self, Task<Message>) {
        let (resolved, font_problems) = metrics::resolve(
            &config.appearance.font_family,
            &config.appearance.fallback_fonts,
        );
        let mut app = Self::with_catalogs(
            runtime,
            config,
            resolved,
            options.schemes,
            options.styles,
            options.software_render,
        );
        app.exit_after_first_frame = options.exit_after_first_frame;
        app.config_dir = options.config_dir;
        let mut tasks = vec![app.refresh_ssh_hosts()];
        let mut restored = false;
        if let Some(dir) = options.state_dir {
            app.load_recent(dir.join(fogu_core::recent::RECENT_FILE));
            if !options.no_recovery {
                let enabled = app.config.recovery.enabled;
                restored = app.restore_session(&dir, enabled);
                if enabled {
                    app.recovery = Some(crate::recovery::Recovery::new(dir));
                    tasks.push(app.after(Duration::from_secs(1), Message::RecoveryTick));
                }
            }
        }
        let font_notes = font_problems
            .into_iter()
            .map(|p| Notification::new(Level::Warn, "Font").detail(p));
        for note in options.notices.into_iter().chain(font_notes) {
            tasks.push(app.notify(note));
        }
        if !app.toasts.is_empty() {
            tasks.push(app.after(AUTO_DISMISS, Message::ToastExpire));
        }
        if !restored {
            let profile = app.default_profile();
            app.open_tab(&profile, None);
        }
        tasks.push(
            window::oldest()
                .and_then(window::scale_factor)
                .map(Message::ScaleFactor),
        );
        (app, Task::batch(tasks))
    }

    /// Title bar buttons, moves and resize edges.
    fn on_window(&mut self, cmd: WindowCmd) -> Task<Message> {
        self.bar_press = matches!(cmd, WindowCmd::Press).then_some(self.tab_bar_cursor);
        let act = move |id| match cmd {
            WindowCmd::Press | WindowCmd::Release => Task::none(),
            WindowCmd::Resize(dir) => window::drag_resize(id, dir),
            WindowCmd::Minimize => window::minimize(id, true),
            WindowCmd::ToggleMaximize => window::toggle_maximize(id),
        };
        window::latest().and_then(act)
    }

    /// The iced theme: stock widgets in the scheme's colours.
    pub fn theme(&self) -> iced::Theme {
        self.theme.clone()
    }

    /// Recomputes the chrome look and stock-widget theme after an appearance change.
    fn refresh_look(&mut self) {
        let (style, _) = resolve_style(&self.config.appearance.ui_theme, &self.styles);
        self.look = Look::new(&self.colors, style, self.config.appearance.accent, self.env);
        self.theme = self.look.theme(&self.colors);
    }

    /// Re-resolves the selected terminal scheme against the current catalog.
    fn apply_scheme(&mut self) -> Option<Notification> {
        let (scheme, note) = resolve_scheme(&self.config.appearance.color_scheme, &self.schemes);
        self.colors = Colors::new(&scheme);
        self.refresh_look();
        note
    }

    fn save_ui(&self) -> Option<Notification> {
        let dir = fogu_core::config::config_dir(self.config_dir.as_deref());
        let (theme, accent) = {
            let appearance = &self.config.appearance;
            (appearance.ui_theme.clone(), appearance.accent)
        };
        fogu_core::config::set_appearance_ui(&dir, &theme, accent)
            .err()
            .map(|err| {
                Notification::new(Level::Error, "Settings not saved").detail(err.to_string())
            })
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
        let dir = fogu_core::config::config_dir(self.config_dir.as_deref());
        let appearance = &self.config.appearance;
        if let Err(err) = fogu_core::config::set_appearance(
            &dir,
            &appearance.ui_theme,
            appearance.accent,
            follow.as_deref(),
        ) {
            notes.push(
                Notification::new(Level::Error, "Settings not saved").detail(err.to_string()),
            );
        }
        let tasks: Vec<Task<Message>> = notes.into_iter().map(|note| self.notify(note)).collect();
        (!tasks.is_empty()).then(|| Task::batch(tasks))
    }

    /// Re-reads `styles/` and `themes/` and re-applies the configured style and scheme; returns
    /// what to report (skipped files, a style or scheme that is gone).
    fn reload_catalogs(&mut self) -> Vec<Notification> {
        let dir = fogu_core::config::config_dir(self.config_dir.as_deref());
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
    fn open_code(
        &mut self,
        kind: crate::style_code::CodeKind,
        name: Option<&str>,
    ) -> Option<Notification> {
        use crate::style_code::{CodeEditor, CodeKind, palette_template, style_template};
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
        self.settings.get_or_insert_with(Default::default).editor = Some(editor);
        None
    }

    /// Writes the code editor's file, reloads the catalogs and applies what was saved. A failed
    /// write stays in the editor with the reason.
    fn save_code(&mut self) -> Vec<Task<Message>> {
        use crate::style_code::CodeKind;
        let Some(editor) = self.settings.as_mut().and_then(|p| p.editor.as_mut()) else {
            return Vec::new();
        };
        let dir = fogu_core::config::config_dir(self.config_dir.as_deref());
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
            CodeKind::Style => crate::settings::SettingsMsg::Style(name),
            CodeKind::Palette => crate::settings::SettingsMsg::Scheme(name),
        };
        tasks.push(self.on_settings(apply));
        tasks
    }

    /// Applies and persists one settings choice.
    fn on_settings(&mut self, message: crate::settings::SettingsMsg) -> Task<Message> {
        use crate::settings::SettingsMsg;
        let mut tasks = Vec::new();
        match message {
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
                let dir = fogu_core::config::config_dir(self.config_dir.as_deref());
                if let Err(err) = fogu_core::config::set_appearance_scheme(
                    &dir,
                    &self.config.appearance.color_scheme,
                ) {
                    tasks.push(
                        self.notify(
                            Notification::new(Level::Error, "Settings not saved")
                                .detail(err.to_string()),
                        ),
                    );
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

    /// Window title = the active tab's title.
    pub fn title(&self) -> String {
        match self.active_tab().map(Tab::title) {
            Some(title) if !title.is_empty() => title.to_owned(),
            _ => DEFAULT_TITLE.to_owned(),
        }
    }

    /// Handles one message, then (re)starts the cursor blink timer if it is due.
    pub fn update(&mut self, message: Message) -> Task<Message> {
        let now = Instant::now();
        if matches!(
            message,
            Message::Pane(..) | Message::Key(_) | Message::Text(_) | Message::Focused(true)
        ) {
            self.blink.activity(now);
        }
        let (active, tab_count) = (self.active, self.tabs.len());
        let mut task = Task::batch([self.dispatch(message), self.sync_backgrounds()]);
        if self.active != active || self.tabs.len() != tab_count {
            task = Task::batch([task, self.reveal_active_tab()]);
        }
        let blink = &self.config.appearance;
        if self.blink.arm(now, blink.cursor_blink, self.focused) {
            return Task::batch([task, self.after(blink::PERIOD, Message::BlinkTick)]);
        }
        task
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one exhaustive message dispatch; grouping arms hides the message flow"
    )]
    fn dispatch(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::DismissToast(id) => {
                self.dismiss_toast(id);
                Task::none()
            }
            Message::ToastAccept(id) => self.accept_toast(id),
            Message::ToastExpire => {
                self.expire_toasts();
                Task::none()
            }
            Message::ReconnectTick(pane) => self.reconnect_tick(pane),
            Message::RecoveryTick => self.recovery_tick(),
            Message::PaneMenu(_)
            | Message::MenuSplit
            | Message::MenuNewWith
            | Message::PaneNotify(..)
            | Message::PaneSaveAsProfile(_)
            | Message::PaneExport(_)
            | Message::PaneExportTo(..)
            | Message::DividerMenu(..)
            | Message::TabBarMenu
            | Message::TabSaveToProfile
            | Message::ConfirmDiscard(_) => self.on_menu_message(message),
            Message::Window(cmd) => self.on_window(cmd),
            Message::Drag(message) => self.on_drag(message),
            Message::LinkHover(..) | Message::OpenLink(..) | Message::CopyLink(_) => {
                self.on_link_message(message)
            }
            Message::CloseRequested => {
                self.save_on_exit();
                iced::exit()
            }
            Message::ConfirmClearSessions(yes) => self.confirm_clear_sessions(yes),
            Message::BlinkTick => {
                let (blink, focused) = (self.config.appearance.cursor_blink, self.focused);
                if self.blink.tick(Instant::now(), blink, focused) {
                    return self.after(blink::PERIOD, Message::BlinkTick);
                }
                Task::none()
            }
            Message::Frame => iced::exit(),
            Message::Prompt(message) => self.on_prompt_msg(message),
            Message::Picker(message) => self.on_picker(message),
            Message::Profiles(message) => self.on_profiles(message),
            Message::OpenSettings => {
                self.settings = Some(crate::settings::SettingsPage::default());
                Task::none()
            }
            Message::CloseSettings => {
                self.settings = None;
                Task::none()
            }
            Message::Settings(message) => self.on_settings(message),
            Message::Editor(tab, message) => self.on_editor(tab, message),
            Message::Enter => self.on_enter(),
            Message::ConfirmClose(confirmed) => self.on_confirm_close(confirmed),
            Message::Sftp(message) => self.on_sftp(message),
            Message::Forwards(message) => self.on_forwards(message),
            Message::SecretDialog(message) => self.on_secret_dialog(message),
            Message::Search(message) => self.on_search(message),
            Message::Desktop(message) => self.on_desktop(message),
            Message::Background(message) => self.on_background(message),
            Message::KeyIgnored(key, modified, mods) => self.on_key_ignored(&key, &modified, mods),
            Message::Modifiers(mods) => {
                self.modifiers = mods;
                Task::none()
            }
            Message::MousePressed => {
                self.on_mouse_pressed();
                Task::none()
            }
            Message::Session(pane, event) => self.on_session(pane, event),
            Message::Action(action) => self.on_action(action),
            Message::GridResized { pane, cols, rows } => self.on_grid_resized(pane, cols, rows),
            Message::ResizeTimer(pane, generation) => self.on_resize_timer(pane, generation),
            Message::ScaleFactor(scale) => {
                self.scale = scale;
                self.apply_metrics();
                Task::none()
            }
            Message::WindowResized(size) => self.on_window_resized(size),
            Message::QueryWindowMode => query_window_mode(),
            Message::WindowMode(mode) => {
                self.fullscreen = mode == window::Mode::Fullscreen;
                Task::none()
            }
            Message::Focused(focused) => {
                self.on_focused(focused);
                Task::none()
            }
            Message::SplitRatio(..)
            | Message::Equalize(_)
            | Message::FocusPane(_)
            | Message::CwdTimeout(_) => self.on_split_ui(message),
            Message::Pane(pane, inner) => self.on_pane_input(Some(pane), *inner),
            Message::Key(_)
            | Message::Text(_)
            | Message::MouseInput(_)
            | Message::Select(_)
            | Message::Scroll(_)
            | Message::Paste(_)
            | Message::PasteReady(_)
            | Message::ConfirmPaste
            | Message::CancelPaste => self.on_pane_input(None, message),
            other => self.on_tab_ui(other),
        }
    }

    /// Keys, mouse and paste for pane `target` (default: the active tab's focused pane).
    fn on_pane_input(&mut self, target: Option<PaneId>, message: Message) -> Task<Message> {
        let (alt_is_meta, scroll) = (
            self.config.terminal.alt_is_meta,
            self.config.terminal.scroll_on_input,
        );
        let Some(id) = target.or_else(|| self.active_pane().map(|p| p.id)) else {
            return Task::none();
        };
        // Enter answers a pending "set the clipboard?" toast (Copy) instead of reaching the shell.
        if let Message::Key(key) = &message
            && key.key == input::KeyCode::Enter
            && key.mods == input::Mods::default()
            && let Some(toast) = self.pending_clipboard.last().map(|p| p.toast)
        {
            return self.accept_toast(toast);
        }
        let Some(pane) = self.pane_mut(id) else {
            return Task::none();
        };
        match message {
            Message::Key(_) | Message::Text(_) if !pane.state.is_running() => {
                if pane.reconnect_in.is_some() && restart::is_escape(&message, alt_is_meta) {
                    pane.reconnect_in = None;
                    pane.status = pane.state.status_line(pane.is_ssh());
                    pane.refresh();
                } else if restart::restarts(&message, alt_is_meta) {
                    return self.restart_pane(id);
                }
            }
            Message::Key(key) => {
                if let Some(bytes) = input::encode_key(&key, pane.term.mode(), alt_is_meta) {
                    pane.send_input(bytes, scroll);
                }
            }
            Message::Text(text) => {
                if let Some(bytes) = input::encode_text(&text) {
                    pane.send_input(bytes, scroll);
                }
            }
            Message::MouseInput(bytes) => pane.write_input(bytes),
            Message::Select(event) => return self.on_select(id, event),
            Message::Scroll(scroll) => {
                pane.term.scroll_display(scroll);
                pane.refresh();
            }
            Message::Paste(source) => {
                return match source {
                    PasteSource::Clipboard => clipboard::read(),
                    PasteSource::Primary => crate::lifecycle::read_selection(),
                }
                .map(move |text| Message::Pane(id, Box::new(Message::PasteReady(text))));
            }
            Message::PasteReady(text) => self.on_paste_ready(id, text),
            Message::ConfirmPaste => {
                if let Some(pending) = self.pending_paste.take() {
                    self.paste_now(pending.pane, &pending.text);
                }
            }
            Message::CancelPaste => self.pending_paste = None,
            _ => {}
        }
        Task::none()
    }

    /// Keeps the size; on macOS also asks for the mode, as the green button enters full screen
    /// without the action.
    fn on_window_resized(&mut self, size: iced::Size) -> Task<Message> {
        self.window_size = size;
        if cfg!(target_os = "macos") {
            // Again once the animation is over: leaving full screen, winit clears its state only
            // after the last resize of the animation.
            let settled = self.after(FULLSCREEN_SETTLE, Message::QueryWindowMode);
            return Task::batch([query_window_mode(), settled]);
        }
        Task::none()
    }

    /// Window focus, or a click into a pane (which also commits an inline rename).
    pub(crate) fn on_focused(&mut self, focused: bool) {
        if focused && self.renaming.is_some() {
            self.commit_rename();
        }
        if !focused {
            self.menu = None;
        }
        if focused != self.focused
            && let Some(pane) = self.active_pane()
            && let Some(bytes) = input::encode_focus(focused, pane.term.mode())
        {
            pane.write_input(bytes);
        }
        self.focused = focused;
    }

    /// Clamps to [`MIN_FONT_PT`]..=[`MAX_FONT_PT`] and recomputes cell metrics; the next frame's
    /// `GridResized` picks up the new grid size and resizes the
    /// PTY, same as a window resize.
    pub(crate) fn set_font_size(&mut self, size_pt: f32) {
        self.font.size_pt = size_pt.clamp(MIN_FONT_PT, MAX_FONT_PT);
        self.apply_metrics();
    }

    pub(crate) fn zoom_reset(&mut self) {
        self.set_font_size(self.default_font_size_pt);
    }

    /// Recomputes cell metrics from font + scale and tells every pane's terminal.
    fn apply_metrics(&mut self) {
        self.metrics = CellMetrics::new(self.resolved.units, self.font.size_px(), self.scale);
        let m = &self.metrics;
        for pane in self.tabs.iter_mut().flat_map(|t| t.panes.values_mut()) {
            pane.term.set_cell_size_px(m.cell_w_px, m.cell_h_px);
        }
    }

    /// Applies a selection gesture; viewport cells map to grid lines with the current scroll.
    fn on_select(&mut self, pane: PaneId, event: SelectEvent) -> Task<Message> {
        let copy_on_select = self.config.terminal.copy_on_select;
        let Some(pane) = self.pane_mut(pane) else {
            return Task::none();
        };
        let offset = pane.snapshot.display_offset;
        let term = &mut pane.term;
        match event {
            SelectEvent::Start { kind, at } => {
                term.start_selection(kind, viewport_to_point(at, offset), at.side);
            }
            SelectEvent::Update(at) => {
                term.update_selection(viewport_to_point(at, offset), at.side);
            }
            SelectEvent::Clear => term.clear_selection(),
            SelectEvent::Finish => {
                return match term.selection_text() {
                    Some(text) if copy_on_select => crate::lifecycle::write_selection(text),
                    _ => Task::none(),
                };
            }
        }
        pane.refresh();
        Task::none()
    }

    /// `message` after `delay` (timer on the session runtime).
    pub(crate) fn after(&self, delay: Duration, message: Message) -> Task<Message> {
        // The sleep is created inside the task: it needs the runtime's timer.
        let timer = self
            .runtime
            .handle()
            .spawn(async move { tokio::time::sleep(delay).await });
        Task::perform(timer, move |_| message.clone())
    }
}

/// How long macOS's full-screen animation may take.
const FULLSCREEN_SETTLE: Duration = Duration::from_secs(1);

/// The window's mode as [`Message::WindowMode`].
fn query_window_mode() -> Task<Message> {
    window::oldest()
        .and_then(window::mode)
        .map(Message::WindowMode)
}
