//! The Advanced page: tab recovery, the config file itself, and diagnostics (log level, log
//! folder, renderer).

use std::sync::Arc;

use fogu_core::config::{LogLevel, Renderer, TomlValue, config_dir};
use fogu_core::{Action, Config};
use iced::widget::{Id, column, container, row, text};
use iced::{Alignment, Element, Length, Task};

use super::render::small;
use super::spec::{Applies, Kind, Spec, Value, from_usize, round};
use super::{Confirm, Section, SettingsMsg};
use crate::app::App;
use crate::controls::{self, ButtonKind};
use crate::message::Message;
use crate::notifications::{Level, Notification};
use crate::palette::Look;

/// One Advanced page setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AdvKey {
    RecoveryEnabled,
    RestoreScrollback,
    RecoveryLines,
    LogLevel,
    Renderer,
}

const LEVELS: &[&str] = &["Errors only", "Warnings", "Info", "Debug", "Trace"];
const RENDERERS: &[&str] = &["GPU", "Software (CPU)"];

impl AdvKey {
    /// Every key, in page order.
    pub const ALL: [Self; 5] = [
        Self::RecoveryEnabled,
        Self::RestoreScrollback,
        Self::RecoveryLines,
        Self::LogLevel,
        Self::Renderer,
    ];

    /// Its description.
    pub fn spec(self) -> Spec {
        let row = |group, (path, key), label, help, keywords, kind, applies| Spec {
            section: Section::Advanced,
            group,
            path,
            key,
            label,
            help,
            keywords,
            kind,
            applies,
        };
        let recovery: &'static [&'static str] = &["recovery"];
        let advanced: &'static [&'static str] = &["advanced"];
        match self {
            Self::RecoveryEnabled => row(
                "Tab recovery",
                (recovery, "enabled"),
                "Restore tabs on restart",
                "Tabs, splits and their profiles come back; turning it off deletes what was \
                 saved.",
                "restore session recovery restart",
                Kind::Toggle,
                Applies::Live,
            ),
            Self::RestoreScrollback => row(
                "Tab recovery",
                (recovery, "restore_scrollback"),
                "Restore scrollback too",
                "Saved unencrypted, readable only by your user: it may hold secrets shown in a \
                 terminal. Profiles may override it.",
                "history scrollback restore",
                Kind::Toggle,
                Applies::Live,
            ),
            Self::RecoveryLines => row(
                "Tab recovery",
                (recovery, "scrollback_lines"),
                "Lines saved per pane",
                "Of each pane's scrollback.",
                "lines history restore",
                Kind::Number {
                    min: 0,
                    max: 100_000,
                },
                Applies::Live,
            ),
            Self::LogLevel => row(
                "Diagnostics",
                (advanced, "log_level"),
                "Log level",
                "How much goes to the log file; RUST_LOG wins when it is set.",
                "log logging debug trace verbose",
                Kind::Choice(LEVELS),
                Applies::Live,
            ),
            Self::Renderer => row(
                "Diagnostics",
                (advanced, "renderer"),
                "Renderer",
                "Software draws on the CPU, for broken GPU drivers and virtual machines.",
                "gpu cpu software tiny-skia wgpu render",
                Kind::Choice(RENDERERS),
                Applies::NextStart,
            ),
        }
    }

    /// Its current value.
    pub fn get(self, config: &Config) -> Value {
        let r = &config.recovery;
        match self {
            Self::RecoveryEnabled => Value::Bool(r.enabled),
            Self::RestoreScrollback => Value::Bool(r.restore_scrollback),
            Self::RecoveryLines => Value::Num(from_usize(r.scrollback_lines)),
            Self::LogLevel => Value::Choice(match config.advanced.log_level {
                LogLevel::Error => 0,
                LogLevel::Warn => 1,
                LogLevel::Info => 2,
                LogLevel::Debug => 3,
                LogLevel::Trace => 4,
            }),
            Self::Renderer => Value::Choice(match config.advanced.renderer {
                Renderer::Gpu => 0,
                Renderer::Software => 1,
            }),
        }
    }

    /// Sets it in the running config.
    pub fn set(self, config: &mut Config, value: &Value) {
        let r = &mut config.recovery;
        match self {
            Self::RecoveryEnabled => r.enabled = value.bool(),
            Self::RestoreScrollback => r.restore_scrollback = value.bool(),
            Self::RecoveryLines => {
                r.scrollback_lines = usize::try_from(round(value.num())).unwrap_or(0);
            }
            Self::LogLevel => config.advanced.log_level = level(value.choice()),
            Self::Renderer => {
                config.advanced.renderer = if value.choice() == 1 {
                    Renderer::Software
                } else {
                    Renderer::Gpu
                };
            }
        }
    }

    /// How a choice is written.
    pub fn named(self, value: &Value) -> Option<TomlValue> {
        let name = match self {
            Self::LogLevel => match level(value.choice()) {
                LogLevel::Error => "error",
                LogLevel::Warn => "warn",
                LogLevel::Info => "info",
                LogLevel::Debug => "debug",
                LogLevel::Trace => "trace",
            },
            Self::Renderer if value.choice() == 1 => "software",
            Self::Renderer => "gpu",
            _ => return None,
        };
        Some(TomlValue::from(name))
    }
}

fn level(index: usize) -> LogLevel {
    match index {
        0 => LogLevel::Error,
        1 => LogLevel::Warn,
        3 => LogLevel::Debug,
        4 => LogLevel::Trace,
        _ => LogLevel::Info,
    }
}

/// A button on the Advanced page.
#[derive(Clone, Copy, Debug)]
pub enum AdvancedMsg {
    /// Delete the saved tabs (asks first).
    ClearSessions,
    /// Open the config folder in the file manager.
    OpenConfigDir,
    /// Open `config.toml` in an editor.
    OpenConfigFile,
    /// Read `config.toml` again and apply it.
    Reload,
    /// Every setting back to its default (asks first).
    Reset,
    /// Open the log folder.
    OpenLogDir,
}

/// The id of the config tools block, for the search.
pub const TOOLS_ANCHOR: &str = "anchor:config-tools";

/// The page body; `rows` is the plain settings' block.
pub fn body(rows: Element<'_, Message>, log_overridden: bool, look: Look) -> Element<'_, Message> {
    let button = |label: &str, kind: ButtonKind, m: AdvancedMsg| {
        controls::action(
            label,
            kind,
            look,
            Some(Message::Settings(SettingsMsg::Advanced(m))),
        )
    };
    let mut page = column![
        rows,
        row![
            button(
                "Clear saved sessions…",
                ButtonKind::Secondary,
                AdvancedMsg::ClearSessions
            ),
            button(
                "Open log folder",
                ButtonKind::Secondary,
                AdvancedMsg::OpenLogDir
            ),
        ]
        .spacing(8),
    ]
    .spacing(14);
    if log_overridden {
        page = page.push(small(
            look,
            "RUST_LOG is set for this run: it decides the log level until Fogu restarts.",
        ));
    }
    let tools = column![
        controls::section_label(look, "Config file"),
        text("config.toml holds every setting and profile; edits made by hand show up after a reload.")
            .size(12)
            .font(look.text.chrome)
            .color(look.dim),
        row![
            button("Open folder", ButtonKind::Secondary, AdvancedMsg::OpenConfigDir),
            button("Edit config.toml", ButtonKind::Secondary, AdvancedMsg::OpenConfigFile),
            button("Reload config.toml", ButtonKind::Primary, AdvancedMsg::Reload),
            button("Reset settings…", ButtonKind::Danger, AdvancedMsg::Reset),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    ]
    .spacing(10);
    page.push(
        container(tools)
            .id(Id::new(TOOLS_ANCHOR))
            .padding([6, 8])
            .width(Length::Fill),
    )
    .into()
}

impl App {
    /// A button on the Advanced page.
    pub(crate) fn on_advanced(&mut self, message: AdvancedMsg) -> Task<Message> {
        let dir = config_dir(self.config_dir.as_deref());
        match message {
            AdvancedMsg::ClearSessions => return self.on_action(Action::ClearSavedSessions),
            AdvancedMsg::OpenConfigDir => return self.open_path(&dir, false),
            AdvancedMsg::OpenConfigFile => {
                let file = dir.join("config.toml");
                if !file.exists()
                    && let Err(err) = std::fs::create_dir_all(&dir).and_then(|()| {
                        fogu_core::fsutil::write_private_atomic(
                            &file,
                            b"# Fogu settings: see the README.\n",
                        )
                    })
                {
                    return self.notify(
                        Notification::new(Level::Error, "Cannot create config.toml")
                            .detail(err.to_string()),
                    );
                }
                return self.open_path(&file, true);
            }
            AdvancedMsg::Reload => return self.reload_config(),
            AdvancedMsg::Reset => {
                if let Some(page) = &mut self.settings {
                    page.confirm = Some(Confirm::ResetSettings);
                }
            }
            AdvancedMsg::OpenLogDir => {
                let Some(state) = self.state_dir.clone() else {
                    return self.notify(Notification::new(Level::Warn, "No log folder"));
                };
                return self.open_path(&state, false);
            }
        }
        Task::none()
    }

    /// Opens `path` with the desktop's default application; a text file without one opens in
    /// Notepad on Windows.
    fn open_path(&mut self, path: &std::path::Path, text_file: bool) -> Task<Message> {
        match open::that(path) {
            Ok(()) => Task::none(),
            Err(_) if text_file && cfg!(windows) => match open::with(path, "notepad.exe") {
                Ok(()) => Task::none(),
                Err(err) => self.cannot_open(path, &err),
            },
            Err(err) => self.cannot_open(path, &err),
        }
    }

    fn cannot_open(&mut self, path: &std::path::Path, err: &std::io::Error) -> Task<Message> {
        self.notify(
            Notification::new(Level::Error, "Cannot open")
                .detail(format!("{}: {err}", path.display())),
        )
    }

    /// Reads `config.toml` again and applies everything in it; a file that does not load
    /// leaves the running settings as they are.
    pub(crate) fn reload_config(&mut self) -> Task<Message> {
        let dir = config_dir(self.config_dir.as_deref());
        let (loaded, err) = fogu_core::config::load(&dir);
        if let Some(err) = err {
            return self.notify(
                Notification::new(Level::Error, "config.toml not reloaded").detail(err.to_string()),
            );
        }
        let mut tasks: Vec<Task<Message>> =
            fogu_core::config::validate_with_catalog(&loaded, &self.schemes, &self.styles)
                .into_iter()
                .filter(|issue| {
                    !matches!(
                        issue,
                        fogu_core::config::ConfigIssue::UnknownColorScheme(_)
                            | fogu_core::config::ConfigIssue::UnknownUiStyle(_)
                    )
                })
                .map(|issue| {
                    Notification::new(Level::Warn, "Config issue").detail(issue.to_string())
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|note| self.notify(note))
                .collect();
        let old = std::mem::replace(&mut self.config, Arc::new(loaded));
        tasks.push(self.apply_config_change(&old));
        if let Some(page) = &mut self.settings {
            page.pending.clear();
            page.errors.clear();
        }
        self.profiles_page_shown();
        tasks.push(self.notify(Notification::new(Level::Info, "config.toml reloaded")));
        Task::batch(tasks)
    }

    /// "Reset settings": backup, remove the settings tables, reload.
    pub(crate) fn reset_settings(&mut self) -> Task<Message> {
        let dir = config_dir(self.config_dir.as_deref());
        match fogu_core::config::reset_settings(&dir) {
            Ok(backup) => {
                let reloaded = self.reload_config();
                let note = Notification::new(Level::Info, "Settings reset")
                    .detail(format!("The previous file is {}", backup.display()));
                Task::batch([reloaded, self.notify(note)])
            }
            Err(err) => self.notify(
                Notification::new(Level::Error, "Settings not reset").detail(err.to_string()),
            ),
        }
    }

    /// Turns tab recovery on (saving starts within a second) or off (saved files deleted).
    pub(crate) fn set_recovery(&mut self, on: bool) -> Task<Message> {
        let Some(dir) = self.state_dir.clone().filter(|_| !self.no_recovery) else {
            return Task::none();
        };
        if !on {
            self.recovery = None;
            if let Err(err) = fogu_core::recovery::clear(&dir) {
                return self.notify(
                    Notification::new(Level::Warn, "Saved sessions not deleted")
                        .detail(err.to_string()),
                );
            }
            return Task::none();
        }
        if self.recovery.is_some() {
            return Task::none();
        }
        self.recovery = Some(crate::recovery::Recovery::new(dir));
        if self.recovery_ticking {
            return Task::none();
        }
        self.recovery_ticking = true;
        self.after(std::time::Duration::from_secs(1), Message::RecoveryTick)
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::unwrap_used, reason = "tests")]

    use crate::profiles::tests::{app_in, tmp};

    #[test]
    fn recovery_turns_on_and_off_while_running() {
        let t = tmp();
        let mut app = app_in(&t.0);
        let state = t.0.join("state");
        std::fs::create_dir_all(&state).unwrap();
        app.state_dir = Some(state.clone());
        std::fs::write(state.join(fogu_core::recovery::SESSION_FILE), "x").unwrap();

        let _ = app.set_recovery(false);
        assert!(app.recovery.is_none());
        assert!(!state.join(fogu_core::recovery::SESSION_FILE).exists());

        let _ = app.set_recovery(true);
        assert!(app.recovery.is_some());
        assert!(app.recovery_ticking);
        // A second switch on while one chain runs starts no other.
        let _ = app.set_recovery(true);
        assert!(app.recovery_ticking);
    }
}
