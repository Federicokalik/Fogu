//! The Terminal page: the grid's font, cursor, spacing and bell (`[appearance]`, `[terminal]`)
//! and `[terminal]` behaviour (scrollback, selection, paste, keyboard, mouse).

use fogu_core::config::{BellMode, CursorShapeCfg, RightClickAction, TomlValue};
use fogu_core::{Config, Osc52Policy};

use super::Section;
use super::spec::{Applies, Kind, Spec, Unit, Value, from_u64, from_usize, round, to_f32};

/// One Terminal page setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TermKey {
    FontFamily,
    FontSize,
    FallbackFonts,
    LineHeight,
    CursorShape,
    CursorBlink,
    BlinkInterval,
    PaddingX,
    PaddingY,
    BgAlpha,
    Bell,
    BellVolume,
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
const CURSOR: &[&str] = &["Block", "Beam", "Underline"];
const BELL: &[&str] = &[
    "Flash the pane",
    "Dot on the tab",
    "Flash the taskbar",
    "Play a sound",
];
/// The bell modes in [`BELL`]'s order.
const BELL_MODES: [BellMode; 4] = [
    BellMode::Visual,
    BellMode::Badge,
    BellMode::Attention,
    BellMode::Sound,
];

impl TermKey {
    /// Every key, in page order.
    pub const ALL: [Self; 27] = [
        Self::FontFamily,
        Self::FontSize,
        Self::FallbackFonts,
        Self::LineHeight,
        Self::CursorShape,
        Self::CursorBlink,
        Self::BlinkInterval,
        Self::PaddingX,
        Self::PaddingY,
        Self::BgAlpha,
        Self::Bell,
        Self::BellVolume,
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
        let row = |group, (path, key), label, help, keywords, kind| Spec {
            section: Section::Terminal,
            group,
            path,
            key,
            label,
            help,
            keywords,
            kind,
            applies: Applies::Live,
        };
        let look: &'static [&'static str] = &["appearance"];
        let term: &'static [&'static str] = &["terminal"];
        let toggle = |group, key, label, help, keywords| {
            row(group, (term, key), label, help, keywords, Kind::Toggle)
        };
        let slider = |min, max, step, unit| Kind::Slider {
            min,
            max,
            step,
            unit,
            auto: None,
        };
        match self {
            Self::FontFamily => row(
                "Font",
                (look, "font_family"),
                "Font family",
                "A monospace family; \"monospace\" is the system's default one.",
                "typeface font face monospace",
                Kind::Font,
            ),
            Self::FontSize => row(
                "Font",
                (look, "font_size"),
                "Font size",
                "Zoom shortcuts change the open window only; Reset zoom returns here.",
                "text size zoom points",
                slider(6.0, 40.0, 0.5, Unit::Pt),
            ),
            Self::FallbackFonts => row(
                "Font",
                (look, "fallback_fonts"),
                "Fallback fonts",
                "Checked and reported when missing; glyphs the main font lacks are found \
                 automatically.",
                "fallback glyphs icons emoji",
                Kind::Fonts,
            ),
            Self::LineHeight => row(
                "Font",
                (look, "line_height"),
                "Line height",
                "Space between rows, as a multiple of the font's own; text stays centred.",
                "line spacing leading rows",
                slider(0.8, 2.0, 0.05, Unit::Times),
            ),
            Self::CursorShape => row(
                "Cursor",
                (look, "cursor_shape"),
                "Cursor shape",
                "Until a program picks its own shape.",
                "block beam bar underline caret",
                Kind::Choice(CURSOR),
            ),
            Self::CursorBlink => row(
                "Cursor",
                (look, "cursor_blink"),
                "Blinking cursor",
                "The cursor blinks while you type and stops after ten idle seconds.",
                "blink caret",
                Kind::Toggle,
            ),
            Self::BlinkInterval => row(
                "Cursor",
                (look, "cursor_blink_interval_ms"),
                "Blink speed",
                "How long the cursor stays on and off.",
                "blink rate interval",
                slider(100.0, 2000.0, 10.0, Unit::Ms),
            ),
            Self::PaddingX => row(
                "Spacing",
                (look, "padding_x"),
                "Side padding",
                "Room between a pane's left and right edges and the text.",
                "margin padding horizontal",
                slider(0.0, 32.0, 1.0, Unit::Px),
            ),
            Self::PaddingY => row(
                "Spacing",
                (look, "padding_y"),
                "Top and bottom padding",
                "Room between a pane's top and bottom edges and the text.",
                "margin padding vertical",
                slider(0.0, 32.0, 1.0, Unit::Px),
            ),
            Self::BgAlpha => row(
                "Spacing",
                (look, "terminal_bg_alpha"),
                "Background opacity",
                "Over a tab background image or colour; automatic is 75% over an image.",
                "transparency alpha opacity background",
                Kind::Slider {
                    min: 0.0,
                    max: 1.0,
                    step: 0.05,
                    unit: Unit::Percent,
                    auto: Some("Automatic"),
                },
            ),
            Self::Bell => row(
                "Bell",
                (term, "bell"),
                "When a program rings the bell",
                "Any combination; none ignores the bell.",
                "bell beep alert notification sound flash",
                Kind::Flags(BELL),
            ),
            Self::BellVolume => row(
                "Bell",
                (term, "bell_volume"),
                "Bell volume",
                "Loudness of the bell sound.",
                "sound volume loud",
                slider(0.0, 1.0, 0.05, Unit::Percent),
            ),
            Self::ScrollbackLines => row(
                "Scrollback",
                (term, "scrollback_lines"),
                "Scrollback lines",
                "Lines of history kept per pane; lowering it drops older lines in open panes.",
                "history buffer lines",
                Kind::Number {
                    min: 0,
                    max: 1_000_000,
                },
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
                (term, "word_separators"),
                "Word separators",
                "Characters that end a double-click word selection.",
                "double click semantic word",
                Kind::Text {
                    placeholder: " ()[]{}'\"",
                },
            ),
            Self::Osc52 => row(
                "Selection",
                (term, "osc52_write"),
                "Programs writing the clipboard",
                "OSC 52 requests from programs (tmux, editors); profiles may override it.",
                "osc52 clipboard tmux remote copy",
                Kind::Choice(OSC52),
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
                (term, "wheel_lines"),
                "Lines per wheel notch",
                "How far one notch of the mouse wheel scrolls.",
                "scroll speed wheel",
                slider(1.0, 20.0, 1.0, Unit::Plain(0)),
            ),
            Self::RightClick => row(
                "Mouse",
                (term, "right_click"),
                "Right click",
                "Shift+right-click always opens the menu.",
                "context menu paste mouse",
                Kind::Choice(RIGHT_CLICK),
            ),
            Self::ResizeStep => row(
                "Panes",
                (term, "pane_resize_step"),
                "Resize step",
                "Share of a split each resize-pane shortcut moves.",
                "split resize keyboard",
                slider(0.01, 0.5, 0.01, Unit::Percent),
            ),
        }
    }

    /// Its current value.
    pub fn get(self, config: &Config) -> Value {
        let a = &config.appearance;
        let t = &config.terminal;
        match self {
            Self::FontFamily => Value::Text(a.font_family.clone()),
            Self::FontSize => Value::Num(f64::from(a.font_size)),
            Self::FallbackFonts => Value::List(a.fallback_fonts.clone()),
            Self::LineHeight => Value::Num(f64::from(a.line_height)),
            Self::CursorShape => Value::Choice(match a.cursor_shape {
                CursorShapeCfg::Block => 0,
                CursorShapeCfg::Beam => 1,
                CursorShapeCfg::Underline => 2,
            }),
            Self::CursorBlink => Value::Bool(a.cursor_blink),
            Self::BlinkInterval => Value::Num(from_u64(a.cursor_blink_interval_ms)),
            Self::PaddingX => Value::Num(f64::from(a.padding_x)),
            Self::PaddingY => Value::Num(f64::from(a.padding_y)),
            Self::BgAlpha => a
                .terminal_bg_alpha
                .map_or(Value::Auto, |v| Value::Num(f64::from(v))),
            Self::Bell => Value::Flags(BELL_MODES.iter().map(|m| t.bell.contains(m)).collect()),
            Self::BellVolume => Value::Num(f64::from(t.bell_volume)),
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
        let a = &mut config.appearance;
        let t = &mut config.terminal;
        let count = |v: &Value| usize::try_from(round(v.num())).unwrap_or(0);
        match self {
            Self::FontFamily => value.text().trim().clone_into(&mut a.font_family),
            Self::FontSize => a.font_size = to_f32(value.num()),
            Self::FallbackFonts => a.fallback_fonts = value.list().to_vec(),
            Self::LineHeight => a.line_height = to_f32(value.num()),
            Self::CursorShape => a.cursor_shape = cursor(value.choice()),
            Self::CursorBlink => a.cursor_blink = value.bool(),
            Self::BlinkInterval => {
                a.cursor_blink_interval_ms = u64::try_from(round(value.num())).unwrap_or(530);
            }
            Self::PaddingX => a.padding_x = to_f32(value.num()),
            Self::PaddingY => a.padding_y = to_f32(value.num()),
            Self::BgAlpha => {
                a.terminal_bg_alpha = match value {
                    Value::Auto => None,
                    v => Some(to_f32(v.num())),
                };
            }
            Self::Bell => {
                t.bell = BELL_MODES
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| value.flag(*i))
                    .map(|(_, m)| *m)
                    .collect();
            }
            Self::BellVolume => t.bell_volume = to_f32(value.num()),
            Self::ScrollbackLines => t.scrollback_lines = count(value),
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

    /// How a choice or flag set is written.
    pub fn named(self, value: &Value) -> Option<TomlValue> {
        let name = match self {
            Self::Osc52 => osc52_name(osc52(value.choice())),
            Self::CursorShape => match cursor(value.choice()) {
                CursorShapeCfg::Block => "block",
                CursorShapeCfg::Beam => "beam",
                CursorShapeCfg::Underline => "underline",
            },
            Self::RightClick => match value.choice() {
                1 => "paste",
                2 => "clipboard",
                _ => "menu",
            },
            Self::Bell => {
                let names = BELL_MODES
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| value.flag(*i))
                    .map(|(_, m)| bell_name(*m).to_owned())
                    .collect();
                return Some(TomlValue::List(names));
            }
            _ => return None,
        };
        Some(TomlValue::from(name))
    }
}

fn cursor(index: usize) -> CursorShapeCfg {
    match index {
        1 => CursorShapeCfg::Beam,
        2 => CursorShapeCfg::Underline,
        _ => CursorShapeCfg::Block,
    }
}

fn bell_name(mode: BellMode) -> &'static str {
    match mode {
        BellMode::Visual => "visual",
        BellMode::Badge => "badge",
        BellMode::Attention => "attention",
        BellMode::Sound => "sound",
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
