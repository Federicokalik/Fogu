//! The settings table: every plain setting's place in `config.toml`, its label, help, search
//! keywords and control. Pages render rows from it and one handler writes and applies any of
//! them, so a new setting is one entry in a page's key enum.

use fogu_core::Config;
use fogu_core::config::TomlValue;

use super::Section;
use super::profiles::ProfKey;
use super::terminal::TermKey;

/// How a setting is edited.
#[derive(Clone, Copy, Debug)]
pub enum Kind {
    /// A switch.
    Toggle,
    /// A slider over `min..=max` in `step`s, shown with `unit`. `auto`: the label of an extra
    /// "use the automatic value" switch (the key is then removed).
    Slider {
        min: f32,
        max: f32,
        step: f32,
        unit: Unit,
        auto: Option<&'static str>,
    },
    /// A whole number typed in a field.
    Number { min: i64, max: i64 },
    /// One line of text.
    Text { placeholder: &'static str },
    /// One of these labels.
    Choice(&'static [&'static str]),
    /// A font family: the installed monospace families, or any name typed.
    Font,
    /// An ordered list of font families (chips, plus a picker to add one).
    Fonts,
    /// Any of these labels.
    Flags(&'static [&'static str]),
}

/// How a slider shows its value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    /// A number with this many decimals.
    Plain(usize),
    /// Logical pixels.
    Px,
    /// `0.0..=1.0` shown as a percentage.
    Percent,
    /// Milliseconds.
    Ms,
    /// Points.
    Pt,
    /// A multiplier (`1.25×`).
    Times,
}

impl Unit {
    /// `value` as the slider's label shows it.
    pub fn show(self, value: f64) -> String {
        match self {
            Self::Plain(decimals) => format!("{value:.decimals$}"),
            Self::Px => format!("{value:.0} px"),
            Self::Percent => format!("{:.0}%", value * 100.0),
            Self::Ms => format!("{value:.0} ms"),
            Self::Pt => format!("{value:.1} pt"),
            Self::Times => format!("{value:.2}×"),
        }
    }
}

/// When a change takes effect (anything but `Live` is shown as a badge).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Applies {
    /// Right away.
    Live,
}

impl Applies {
    /// The badge shown next to the label (`None` when live).
    pub fn badge(self) -> Option<&'static str> {
        match self {
            Self::Live => None,
        }
    }
}

/// A setting's static description.
#[derive(Clone, Copy, Debug)]
pub struct Spec {
    /// The page it is on.
    pub section: Section,
    /// The heading it is grouped under on that page.
    pub group: &'static str,
    /// Its table in `config.toml` (`[]` = the root).
    pub path: &'static [&'static str],
    /// Its key in that table.
    pub key: &'static str,
    /// The row's label.
    pub label: &'static str,
    /// One line under the label.
    pub help: &'static str,
    /// Extra words the settings search matches.
    pub keywords: &'static str,
    /// Its control.
    pub kind: Kind,
    /// When a change takes effect.
    pub applies: Applies,
}

/// A setting's value as its control shows it.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// A switch.
    Bool(bool),
    /// A slider or number.
    Num(f64),
    /// Text.
    Text(String),
    /// The index of a choice.
    Choice(usize),
    /// A list of names.
    List(Vec<String>),
    /// Which of a flag set's labels are on.
    Flags(Vec<bool>),
    /// The automatic value of an optional slider (no key in the file).
    Auto,
}

impl Value {
    /// The switch state (`false` for any other value).
    pub fn bool(&self) -> bool {
        matches!(self, Self::Bool(true))
    }

    /// The number (0 for any other value).
    pub fn num(&self) -> f64 {
        match self {
            Self::Num(n) => *n,
            _ => 0.0,
        }
    }

    /// The text (empty for any other value).
    pub fn text(&self) -> &str {
        match self {
            Self::Text(t) => t,
            _ => "",
        }
    }

    /// The choice index (0 for any other value).
    pub fn choice(&self) -> usize {
        match self {
            Self::Choice(i) => *i,
            _ => 0,
        }
    }

    /// The list (empty for any other value).
    pub fn list(&self) -> &[String] {
        match self {
            Self::List(items) => items,
            _ => &[],
        }
    }

    /// Whether flag `i` is on (`false` for any other value).
    pub fn flag(&self, i: usize) -> bool {
        match self {
            Self::Flags(flags) => flags.get(i).copied().unwrap_or(false),
            _ => false,
        }
    }
}

/// One plain setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SettingId {
    /// A Terminal page setting.
    Term(TermKey),
    /// A Profiles page setting.
    Prof(ProfKey),
}

impl SettingId {
    /// Every setting, in page order.
    pub fn all() -> impl Iterator<Item = Self> {
        TermKey::ALL
            .iter()
            .copied()
            .map(Self::Term)
            .chain(ProfKey::ALL.iter().copied().map(Self::Prof))
    }

    /// Its description.
    pub fn spec(self) -> Spec {
        match self {
            Self::Term(key) => key.spec(),
            Self::Prof(key) => key.spec(),
        }
    }

    /// Its current value in `config`.
    pub fn get(self, config: &Config) -> Value {
        match self {
            Self::Term(key) => key.get(config),
            Self::Prof(key) => key.get(config),
        }
    }

    /// Sets it in `config` (the running copy, after the file was written).
    pub fn set(self, config: &mut Config, value: &Value) {
        match self {
            Self::Term(key) => key.set(config, value),
            Self::Prof(key) => key.set(config, value),
        }
    }

    /// How `value` is written; `None` removes the key.
    pub fn toml(self, value: &Value) -> Option<TomlValue> {
        let spec = self.spec();
        match (spec.kind, value) {
            (_, Value::Auto) => None,
            (Kind::Toggle, v) => Some(TomlValue::Bool(v.bool())),
            (Kind::Slider { step, .. }, Value::Num(n)) if step >= 1.0 => {
                Some(TomlValue::Int(round(*n)))
            }
            (Kind::Slider { .. }, Value::Num(n)) => Some(TomlValue::float(to_f32(*n))),
            (Kind::Number { .. }, Value::Num(n)) => Some(TomlValue::Int(round(*n))),
            (Kind::Choice(_) | Kind::Flags(_), _) => match self {
                Self::Term(key) => key.named(value),
                Self::Prof(key) => key.named(value),
            },
            (_, Value::List(items)) => Some(TomlValue::List(items.clone())),
            (_, Value::Text(t)) => Some(TomlValue::Str(t.clone())),
            _ => None,
        }
    }

    /// The id of its row, for scrolling it into view.
    pub fn widget_id(self) -> iced::widget::Id {
        let spec = self.spec();
        iced::widget::Id::from(format!("setting:{}.{}", spec.path.join("."), spec.key))
    }
}

/// `n` as a whole number (settings ranges are far inside `i64`).
#[expect(
    clippy::cast_possible_truncation,
    reason = "every numeric setting's range is checked before it is written"
)]
pub fn round(n: f64) -> i64 {
    n.round() as i64
}

/// A count as a control value.
#[expect(
    clippy::cast_precision_loss,
    reason = "settings counts stay far below 2^52"
)]
pub fn from_usize(n: usize) -> f64 {
    n as f64
}

/// A number field's value as a control value.
#[expect(
    clippy::cast_precision_loss,
    reason = "settings numbers stay far below 2^52"
)]
pub fn from_i64(n: i64) -> f64 {
    n as f64
}

/// `n` as the `f32` the config stores.
#[expect(
    clippy::cast_possible_truncation,
    reason = "slider values are small and written through their shortest f32 form"
)]
pub fn to_f32(n: f64) -> f32 {
    n as f32
}

/// Parses a number field's text against `min..=max`.
pub fn parse_number(text: &str, min: i64, max: i64) -> Result<i64, String> {
    let digits: String = text.trim().chars().filter(|c| *c != '_').collect();
    match digits.parse::<i64>() {
        Ok(n) if (min..=max).contains(&n) => Ok(n),
        _ => Err(format!("a whole number from {min} to {max}")),
    }
}
