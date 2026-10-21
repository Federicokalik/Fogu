//! The Terminal page: `[terminal]` behaviour (scrollback, selection, paste, keyboard, mouse).

use fogu_core::config::RightClickAction;
use fogu_core::{Config, Osc52Policy};

use super::Section;
use super::spec::{Applies, Kind, Spec, Unit, Value, from_usize, round, to_f32};

/// One `[terminal]` setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TermKey {
    ScrollbackLines,
    KeepHistory,
    CopyOnSelect,
    WordSeparators,
    Osc52,
    MiddlePaste,
    WarnMultiline,
    TrimPaste,
    ReplaceNewlines,
    BracketedPaste,
    AltIsMeta,
    ScrollOnInput,
    WheelLines,
    RightClick,
    ResizeStep,
}

const RIGHT_CLICK: &[&str] = &[
    "Open the menu",
    "Paste the selection",
    "Paste the clipboard",
];
const OSC52: &[&str] = &["Ask each time", "Allow", "Deny"];

impl TermKey {
    /// Every key, in page order.
    pub const ALL: [Self; 15] = [
        Self::ScrollbackLines,
        Self::KeepHistory,
        Self::CopyOnSelect,
        Self::WordSeparators,
        Self::Osc52,
        Self::MiddlePaste,
        Self::WarnMultiline,
        Self::TrimPaste,
        Self::ReplaceNewlines,
        Self::BracketedPaste,
        Self::AltIsMeta,
        Self::ScrollOnInput,
        Self::WheelLines,
        Self::RightClick,
        Self::ResizeStep,
    ];

    /// Its description.
    #[expect(
        clippy::too_many_lines,
        reason = "the page's settings table: one short entry per setting"
    )]
    pub fn spec(self) -> Spec {
        let row = |group, key, label, help, keywords, kind, applies| Spec {
            section: Section::Terminal,
            group,
            path: &["terminal"],
            key,
            label,
            help,
            keywords,
            kind,
            applies,
        };
        let toggle = |group, key, label, help, keywords| {
            row(
                group,
                key,
                label,
                help,
                keywords,
                Kind::Toggle,
                Applies::Live,
            )
        };
        match self {
            Self::ScrollbackLines => row(
                "Scrollback",
                "scrollback_lines",
                "Scrollback lines",
                "Lines of history kept per pane.",
                "history buffer lines",
                Kind::Number {
                    min: 0,
                    max: 1_000_000,
                },
                Applies::NewPanes,
            ),
            Self::KeepHistory => toggle(
                "Scrollback",
                "keep_history_on_restart",
                "Keep history on restart",
                "Restarting or reconnecting an ended pane keeps its screen and scrollback.",
                "restart reconnect history clear",
            ),
            Self::CopyOnSelect => toggle(
                "Selection",
                "copy_on_select",
                "Copy on select",
                "A finished mouse selection is copied to the primary selection.",
                "primary clipboard mouse",
            ),
            Self::WordSeparators => row(
                "Selection",
                "word_separators",
                "Word separators",
                "Characters that end a double-click word selection.",
                "double click semantic word",
                Kind::Text {
                    placeholder: " ()[]{}'\"",
                },
                Applies::NewPanes,
            ),
            Self::Osc52 => row(
                "Selection",
                "osc52_write",
                "Programs writing the clipboard",
                "OSC 52 requests from programs (tmux, editors); profiles may override it.",
                "osc52 clipboard tmux remote copy",
                Kind::Choice(OSC52),
                Applies::Live,
            ),
            Self::MiddlePaste => toggle(
                "Paste",
                "paste_on_middle_click",
                "Middle click pastes",
                "A middle click pastes the primary selection.",
                "mouse middle button primary",
            ),
            Self::WarnMultiline => toggle(
                "Paste",
                "warn_on_multiline_paste",
                "Confirm multi-line paste",
                "Asks before pasting text with more than one line.",
                "warning newline confirm",
            ),
            Self::TrimPaste => toggle(
                "Paste",
                "trim_whitespace_on_paste",
                "Trim pasted text",
                "Removes leading and trailing whitespace from pasted text.",
                "whitespace spaces",
            ),
            Self::ReplaceNewlines => toggle(
                "Paste",
                "replace_newlines_on_paste",
                "Paste newlines as spaces",
                "Replaces newlines in pasted text with spaces.",
                "newline single line",
            ),
            Self::BracketedPaste => toggle(
                "Paste",
                "bracketed_paste",
                "Bracketed paste",
                "Honours a program's bracketed-paste request; off forces plain paste.",
                "bracketed shell vim",
            ),
            Self::AltIsMeta => toggle(
                "Keyboard",
                "alt_is_meta",
                "Alt sends Meta",
                "Alt-modified keys send an ESC prefix (readline Alt+B, Alt+F, …).",
                "option meta escape alt",
            ),
            Self::ScrollOnInput => toggle(
                "Keyboard",
                "scroll_on_input",
                "Scroll to bottom on input",
                "Typing jumps back to the bottom of the scrollback.",
                "typing jump",
            ),
            Self::WheelLines => row(
                "Mouse",
                "wheel_lines",
                "Lines per wheel notch",
                "How far one notch of the mouse wheel scrolls.",
                "scroll speed wheel",
                Kind::Slider {
                    min: 1.0,
                    max: 20.0,
                    step: 1.0,
                    unit: Unit::Plain(0),
                    auto: None,
                },
                Applies::Live,
            ),
            Self::RightClick => row(
                "Mouse",
                "right_click",
                "Right click",
                "Shift+right-click always opens the menu.",
                "context menu paste mouse",
                Kind::Choice(RIGHT_CLICK),
                Applies::Live,
            ),
            Self::ResizeStep => row(
                "Panes",
                "pane_resize_step",
                "Resize step",
                "Share of a split each resize-pane shortcut moves.",
                "split resize keyboard",
                Kind::Slider {
                    min: 0.01,
                    max: 0.5,
                    step: 0.01,
                    unit: Unit::Percent,
                    auto: None,
                },
                Applies::Live,
            ),
        }
    }

    /// Its current value.
    pub fn get(self, config: &Config) -> Value {
        let t = &config.terminal;
        match self {
            Self::ScrollbackLines => Value::Num(from_usize(t.scrollback_lines)),
            Self::KeepHistory => Value::Bool(t.keep_history_on_restart),
            Self::CopyOnSelect => Value::Bool(t.copy_on_select),
            Self::WordSeparators => Value::Text(t.word_separators.clone()),
            Self::Osc52 => Value::Choice(osc52_index(t.osc52_write)),
            Self::MiddlePaste => Value::Bool(t.paste_on_middle_click),
            Self::WarnMultiline => Value::Bool(t.warn_on_multiline_paste),
            Self::TrimPaste => Value::Bool(t.trim_whitespace_on_paste),
            Self::ReplaceNewlines => Value::Bool(t.replace_newlines_on_paste),
            Self::BracketedPaste => Value::Bool(t.bracketed_paste),
            Self::AltIsMeta => Value::Bool(t.alt_is_meta),
            Self::ScrollOnInput => Value::Bool(t.scroll_on_input),
            Self::WheelLines => Value::Num(f64::from(t.wheel_lines)),
            Self::RightClick => Value::Choice(match t.right_click {
                RightClickAction::Menu => 0,
                RightClickAction::Paste => 1,
                RightClickAction::Clipboard => 2,
            }),
            Self::ResizeStep => Value::Num(f64::from(t.pane_resize_step)),
        }
    }

    /// Sets it in the running config.
    pub fn set(self, config: &mut Config, value: &Value) {
        let t = &mut config.terminal;
        match self {
            Self::ScrollbackLines => {
                t.scrollback_lines = usize::try_from(round(value.num())).unwrap_or(0);
            }
            Self::KeepHistory => t.keep_history_on_restart = value.bool(),
            Self::CopyOnSelect => t.copy_on_select = value.bool(),
            Self::WordSeparators => value.text().clone_into(&mut t.word_separators),
            Self::Osc52 => t.osc52_write = osc52(value.choice()),
            Self::MiddlePaste => t.paste_on_middle_click = value.bool(),
            Self::WarnMultiline => t.warn_on_multiline_paste = value.bool(),
            Self::TrimPaste => t.trim_whitespace_on_paste = value.bool(),
            Self::ReplaceNewlines => t.replace_newlines_on_paste = value.bool(),
            Self::BracketedPaste => t.bracketed_paste = value.bool(),
            Self::AltIsMeta => t.alt_is_meta = value.bool(),
            Self::ScrollOnInput => t.scroll_on_input = value.bool(),
            Self::WheelLines => t.wheel_lines = u32::try_from(round(value.num())).unwrap_or(1),
            Self::RightClick => {
                t.right_click = match value.choice() {
                    1 => RightClickAction::Paste,
                    2 => RightClickAction::Clipboard,
                    _ => RightClickAction::Menu,
                };
            }
            Self::ResizeStep => t.pane_resize_step = to_f32(value.num()),
        }
    }

    /// The TOML string of a choice.
    pub fn choice_value(self, value: &Value) -> Option<&'static str> {
        match self {
            Self::Osc52 => Some(osc52_name(osc52(value.choice()))),
            Self::RightClick => Some(match value.choice() {
                1 => "paste",
                2 => "clipboard",
                _ => "menu",
            }),
            _ => None,
        }
    }
}

/// `policy`'s index in a three-way OSC 52 choice (ask, allow, deny).
pub fn osc52_index(policy: Osc52Policy) -> usize {
    match policy {
        Osc52Policy::Ask => 0,
        Osc52Policy::Allow => 1,
        Osc52Policy::Deny => 2,
    }
}

/// The policy at `index` of a three-way OSC 52 choice.
pub fn osc52(index: usize) -> Osc52Policy {
    match index {
        0 => Osc52Policy::Ask,
        2 => Osc52Policy::Deny,
        _ => Osc52Policy::Allow,
    }
}

/// `policy` as written in TOML.
pub fn osc52_name(policy: Osc52Policy) -> &'static str {
    match policy {
        Osc52Policy::Ask => "ask",
        Osc52Policy::Allow => "allow",
        Osc52Policy::Deny => "deny",
    }
}
