//! The Shortcuts page: every action with its chords, grouped and searchable. A chord is added or
//! replaced by pressing it; a chord another action uses, or one the shell needs, asks first.
//! `[hotkeys]` keeps only what differs from the platform's defaults.

mod catalog;

use std::sync::Arc;

use fogu_core::{Action, Hotkeys, KeyChord};
use iced::keyboard::key::Named;
use iced::keyboard::{Key, Modifiers};
use iced::widget::{Id, button, column, container, row, space, text};
use iced::{Alignment, Border, Element, Length, Task};

use super::render::{self, field};
use super::{Confirm, SettingsMsg};
use crate::app::App;
use crate::controls::{self, ButtonKind};
use crate::fuzzy_list::{self, FuzzyRow};
use crate::message::Message;
use crate::notifications::{Level, Notification};
use crate::palette::Look;

pub use catalog::label;
use catalog::{Severity, category, check};

/// The page's state.
#[derive(Debug, Default)]
pub struct ShortcutsState {
    /// The filter text.
    pub query: String,
    /// The action waiting for a key press.
    pub capture: Option<Capture>,
    /// A question about the chord just pressed.
    pub notice: Option<Notice>,
}

/// An action waiting for a key press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capture {
    /// The action.
    pub action: Action,
    /// The chord being replaced (`None`: a new one is added).
    pub replace: Option<KeyChord>,
}

/// What the page asks about the chord just pressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notice {
    /// `chord` already runs `other`.
    Conflict {
        capture: Capture,
        chord: KeyChord,
        other: Action,
    },
    /// `chord` is a key the shell needs.
    Shell {
        capture: Capture,
        chord: KeyChord,
        why: &'static str,
    },
    /// `chord` cannot be a shortcut.
    Refused { chord: KeyChord, why: &'static str },
}

/// A choice on the Shortcuts page.
#[derive(Clone, Debug)]
pub enum ShortcutMsg {
    /// The filter changed.
    Query(String),
    /// Wait for a key press for `action` (replacing `.1`, or adding).
    Capture(Action, Option<KeyChord>),
    /// Unbind `.1` from `.0`.
    Remove(Action, KeyChord),
    /// Take the chord from the action that has it.
    Reassign,
    /// Bind the shell key anyway.
    BindAnyway,
    /// Drop the question.
    Dismiss,
    /// Back to `.0`'s default chords.
    Reset(Action),
    /// Back to every default (asks first).
    ResetAll,
}

/// The id of the page's first block, for the search.
pub const ANCHOR: &str = "anchor:shortcuts";

/// The page body.
pub fn body<'a>(state: &'a ShortcutsState, hotkeys: &Hotkeys, look: Look) -> Element<'a, Message> {
    let msg = |m: ShortcutMsg| Message::Settings(SettingsMsg::Shortcut(m));
    let defaults = Hotkeys::default_keymap();
    let top = row![
        field(look, "Filter by name or key", &state.query)
            .on_input(move |q| msg(ShortcutMsg::Query(q)))
            .width(Length::Fill),
        controls::action(
            "Reset all",
            ButtonKind::Secondary,
            look,
            Some(msg(ShortcutMsg::ResetAll))
        ),
    ]
    .spacing(10)
    .align_y(Alignment::Center);
    let mut page = column![
        render::small(
            look,
            "Click + and press the keys to add a shortcut, or click one to replace it. Esc \
             cancels, Backspace removes the one being replaced."
        ),
        top,
    ]
    .spacing(12);
    if let Some(notice) = &state.notice {
        page = page.push(notice_view(notice, look));
    }
    let actions: Vec<Action> = hotkeys.bindings().keys().copied().collect();
    let rows: Vec<FuzzyRow> = actions
        .iter()
        .map(|&a| FuzzyRow {
            title: label(a),
            subtitle: category(a).label().to_owned(),
            keywords: hotkeys
                .chords_for(a)
                .iter()
                .map(ToString::to_string)
                .chain(std::iter::once(a.to_string()))
                .collect(),
            ..FuzzyRow::default()
        })
        .collect();
    let mut shown: Vec<Action> = fuzzy_list::rank(&state.query, &rows)
        .into_iter()
        .filter_map(|i| actions.get(i).copied())
        .collect();
    if state.query.trim().is_empty() {
        shown.sort_by_key(|&a| (category(a), a));
    }
    let mut group = None;
    for action in shown {
        let cat = category(action);
        if group != Some(cat) && state.query.trim().is_empty() {
            group = Some(cat);
            page = page.push(space().height(4));
            page = page.push(controls::section_label(look, cat.label()));
        }
        page = page.push(action_row(state, hotkeys, &defaults, action, look));
    }
    container(page).id(Id::new(ANCHOR)).into()
}

/// One action: its name, its chords, `+` and reset.
fn action_row<'a>(
    state: &ShortcutsState,
    hotkeys: &Hotkeys,
    defaults: &Hotkeys,
    action: Action,
    look: Look,
) -> Element<'a, Message> {
    let msg = |m: ShortcutMsg| Message::Settings(SettingsMsg::Shortcut(m));
    let chords = hotkeys.chords_for(action);
    let capturing = state.capture.filter(|c| c.action == action);
    let mut keys = row![].spacing(6).align_y(Alignment::Center);
    for &chord in chords {
        let replacing = capturing.is_some_and(|c| c.replace == Some(chord));
        keys = keys.push(chord_chip(chord, replacing, action, look));
    }
    if capturing.is_some_and(|c| c.replace.is_none()) {
        keys = keys.push(waiting(look));
    } else {
        keys = keys.push(icon_button(
            "+",
            "Add a shortcut",
            look,
            msg(ShortcutMsg::Capture(action, None)),
        ));
    }
    let changed = chords != defaults.chords_for(action);
    let reset: Element<'a, Message> = if changed {
        icon_button(
            "↺",
            "Back to the default",
            look,
            msg(ShortcutMsg::Reset(action)),
        )
    } else {
        space().width(26).into()
    };
    let (bg, _) = controls::row_colors(look, capturing.is_some(), false);
    container(
        row![
            text(label(action))
                .size(13)
                .font(look.text.chrome)
                .color(look.fg)
                .width(Length::FillPortion(2)),
            container(keys.wrap()).width(Length::FillPortion(3)),
            reset,
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .padding([4, 8])
    .style(move |_theme: &iced::Theme| container::Style {
        background: bg.map(Into::into),
        border: Border::default().rounded(look.shape.control),
        ..container::Style::default()
    })
    .into()
}

/// A bound chord: click to replace it, `×` to remove it.
fn chord_chip<'a>(
    chord: KeyChord,
    replacing: bool,
    action: Action,
    look: Look,
) -> Element<'a, Message> {
    let msg = |m: ShortcutMsg| Message::Settings(SettingsMsg::Shortcut(m));
    if replacing {
        return waiting(look);
    }
    let name = button(
        text(chord.to_string())
            .size(12)
            .font(look.text.mono)
            .color(look.fg),
    )
    .padding([3, 8])
    .style(move |_theme: &iced::Theme, status| keycap(look, status))
    .on_press(msg(ShortcutMsg::Capture(action, Some(chord))));
    let remove = button(text("×").size(12).color(look.dim))
        .padding([3, 6])
        .style(move |_theme: &iced::Theme, status| controls::flat_button(look, status, false))
        .on_press(msg(ShortcutMsg::Remove(action, chord)));
    row![name, remove]
        .spacing(0)
        .align_y(Alignment::Center)
        .into()
}

/// The "press keys" placeholder of the action being captured.
fn waiting<'a>(look: Look) -> Element<'a, Message> {
    container(
        text("Press keys…")
            .size(12)
            .font(look.text.chrome)
            .color(look.accent_text),
    )
    .padding([3, 10])
    .style(move |_theme: &iced::Theme| container::Style {
        border: Border {
            color: look.accent,
            width: 1.0,
            radius: look.shape.control.into(),
        },
        ..container::Style::default()
    })
    .into()
}

fn keycap(look: Look, status: button::Status) -> button::Style {
    let mut style = controls::flat_button(look, status, false);
    style.border = Border {
        color: look.border,
        width: 1.0,
        radius: look.shape.control.into(),
    };
    if style.background.is_none() {
        style.background = Some(crate::palette::mix(look.panel, look.fg, 0.05).into());
    }
    style
}

fn icon_button<'a>(glyph: &str, tip: &str, look: Look, on_press: Message) -> Element<'a, Message> {
    controls::tip(
        button(text(glyph.to_owned()).size(13).color(look.dim))
            .padding([2, 8])
            .style(move |_theme: &iced::Theme, status| controls::flat_button(look, status, false))
            .on_press(on_press),
        tip.to_owned(),
        iced::widget::tooltip::Position::Top,
        look,
    )
}

/// The question about the chord just pressed.
fn notice_view<'a>(notice: &Notice, look: Look) -> Element<'a, Message> {
    let msg = |m: ShortcutMsg| Message::Settings(SettingsMsg::Shortcut(m));
    let (line, accept): (String, Option<(&str, ShortcutMsg)>) = match *notice {
        Notice::Conflict { chord, other, .. } => (
            format!("{chord} already runs \"{}\".", label(other)),
            Some(("Reassign", ShortcutMsg::Reassign)),
        ),
        Notice::Shell { chord, why, .. } => (
            format!("{chord}: {why}."),
            Some(("Use it anyway", ShortcutMsg::BindAnyway)),
        ),
        Notice::Refused { chord, why } => (format!("{chord} can't be a shortcut: {why}."), None),
    };
    let mut line_row = row![
        text(line)
            .size(12)
            .font(look.text.chrome)
            .color(look.fg)
            .width(Length::Fill)
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    if let Some((label, message)) = accept {
        line_row = line_row.push(controls::action(
            label,
            ButtonKind::Primary,
            look,
            Some(msg(message)),
        ));
    }
    line_row = line_row.push(controls::action(
        "Cancel",
        ButtonKind::Secondary,
        look,
        Some(msg(ShortcutMsg::Dismiss)),
    ));
    container(line_row)
        .padding([8, 10])
        .style(move |_theme: &iced::Theme| container::Style {
            background: Some(
                iced::Color {
                    a: 0.12,
                    ..crate::forms::WARN
                }
                .into(),
            ),
            border: Border {
                color: crate::forms::WARN,
                width: 1.0,
                radius: look.shape.control.into(),
            },
            ..container::Style::default()
        })
        .into()
}

impl App {
    /// A choice on the Shortcuts page.
    pub(crate) fn on_shortcut(&mut self, message: ShortcutMsg) -> Task<Message> {
        let Some(page) = &mut self.settings else {
            return Task::none();
        };
        let state = &mut page.shortcuts;
        match message {
            ShortcutMsg::Query(query) => state.query = query,
            ShortcutMsg::Capture(action, replace) => {
                state.capture = Some(Capture { action, replace });
                state.notice = None;
                // No field may keep the keys the capture waits for.
                return iced::advanced::widget::operate(
                    iced::advanced::widget::operation::focusable::unfocus(),
                );
            }
            ShortcutMsg::Remove(action, chord) => {
                state.capture = None;
                let chords: Vec<KeyChord> = self
                    .config
                    .hotkeys
                    .chords_for(action)
                    .iter()
                    .copied()
                    .filter(|c| *c != chord)
                    .collect();
                let mut hotkeys = self.config.hotkeys.clone();
                hotkeys.set_chords(action, chords);
                return self.save_hotkeys(hotkeys);
            }
            ShortcutMsg::Reassign | ShortcutMsg::BindAnyway => {
                if let Some(
                    Notice::Conflict { capture, chord, .. } | Notice::Shell { capture, chord, .. },
                ) = state.notice.take()
                {
                    return self.bind(capture, chord);
                }
            }
            ShortcutMsg::Dismiss => {
                state.notice = None;
                state.capture = None;
            }
            ShortcutMsg::Reset(action) => {
                let mut hotkeys = self.config.hotkeys.clone();
                let defaults = Hotkeys::default_keymap();
                let chords = defaults.chords_for(action).to_vec();
                // A default chord another action took now goes back to this one.
                for &chord in &chords {
                    if let Some(other) = hotkeys.lookup(chord).filter(|o| *o != action) {
                        let kept = hotkeys
                            .chords_for(other)
                            .iter()
                            .copied()
                            .filter(|c| *c != chord)
                            .collect();
                        hotkeys.set_chords(other, kept);
                    }
                }
                hotkeys.set_chords(action, chords);
                return self.save_hotkeys(hotkeys);
            }
            ShortcutMsg::ResetAll => page.confirm = Some(Confirm::ResetShortcuts),
        }
        Task::none()
    }

    /// A key while an action waits for one: checked, then bound (or a question is asked).
    /// `None` when no capture is running.
    pub(crate) fn shortcut_key(&mut self, key: &Key, mods: Modifiers) -> Option<Task<Message>> {
        let page = self.settings.as_mut()?;
        let capture = page.shortcuts.capture?;
        let state = &mut page.shortcuts;
        if let Key::Named(Named::Backspace) = key
            && !(mods.control() || mods.alt() || mods.logo() || mods.shift())
        {
            state.capture = None;
            let replaced = capture.replace?;
            return Some(self.on_shortcut(ShortcutMsg::Remove(capture.action, replaced)));
        }
        // Modifier keys alone wait for the rest of the chord.
        let chord = crate::keys::chord(key, mods)?;
        state.capture = None;
        if let Some((severity, why)) = check(chord, self.config.terminal.alt_is_meta) {
            let notice = match severity {
                Severity::Forbidden => Notice::Refused { chord, why },
                Severity::Warn => Notice::Shell {
                    capture,
                    chord,
                    why,
                },
            };
            state.notice = Some(notice);
            return Some(Task::none());
        }
        if let Some(other) = self
            .config
            .hotkeys
            .lookup(chord)
            .filter(|o| *o != capture.action)
        {
            state.notice = Some(Notice::Conflict {
                capture,
                chord,
                other,
            });
            return Some(Task::none());
        }
        Some(self.bind(capture, chord))
    }

    /// Binds `chord` to the captured action (in place of the chord it replaces), taking it from
    /// any other action that had it.
    fn bind(&mut self, capture: Capture, chord: KeyChord) -> Task<Message> {
        let mut hotkeys = self.config.hotkeys.clone();
        if let Some(other) = hotkeys.lookup(chord).filter(|o| *o != capture.action) {
            let kept = hotkeys
                .chords_for(other)
                .iter()
                .copied()
                .filter(|c| *c != chord)
                .collect();
            hotkeys.set_chords(other, kept);
        }
        let mut chords: Vec<KeyChord> = hotkeys.chords_for(capture.action).to_vec();
        match capture
            .replace
            .and_then(|r| chords.iter().position(|c| *c == r))
        {
            Some(i) => {
                if let Some(slot) = chords.get_mut(i) {
                    *slot = chord;
                }
            }
            None => chords.push(chord),
        }
        let mut seen = Vec::new();
        chords.retain(|c| {
            let new = !seen.contains(c);
            seen.push(*c);
            new
        });
        hotkeys.set_chords(capture.action, chords);
        self.save_hotkeys(hotkeys)
    }

    /// Writes `hotkeys`' differences from the defaults to `[hotkeys]`, then uses them.
    pub(crate) fn save_hotkeys(&mut self, hotkeys: Hotkeys) -> Task<Message> {
        let dir = fogu_core::config::config_dir(self.config_dir.as_deref());
        if let Err(err) = fogu_core::config::set_hotkeys(&dir, &hotkeys.overrides()) {
            return self.notify(
                Notification::new(Level::Error, "Settings not saved").detail(err.to_string()),
            );
        }
        Arc::make_mut(&mut self.config).hotkeys = hotkeys;
        Task::none()
    }
}
