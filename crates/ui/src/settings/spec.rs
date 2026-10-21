//! The settings table: every plain setting's place in `config.toml`, its label, help, search
//! keywords and control. Pages render rows from it and one handler writes and applies any of
//! them, so a new setting is one entry in a page's key enum.

use fogu_core::Config;
use fogu_core::config::TomlValue;

use super::Section;
use super::advanced::AdvKey;
use super::profiles::ProfKey;
use super::sftp::SftpKey;
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
    /// A size in bytes typed as `2 GiB`, `500 MB` or a plain number; `optional`: empty is "no
    /// limit" (the key is removed).
    Bytes { optional: bool },
    /// A folder, typed or picked; empty is the default (the key is removed).
    Folder { placeholder: &'static str },
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
    /// Seconds.
    Secs,
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
            Self::Secs => format!("{value:.0} s"),
        }
    }
}

/// When a change takes effect (anything but `Live` is shown as a badge).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Applies {
    /// Right away.
    Live,
    /// To SFTP panels opened from now on.
    NewPanels,
    /// At the next start of Fogu.
    NextStart,
}

impl Applies {
    /// The badge shown next to the label (`None` when live).
    pub fn badge(self) -> Option<&'static str> {
        match self {
            Self::Live => None,
            Self::NewPanels => Some("New panels"),
            Self::NextStart => Some("Next start"),
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
    /// An SFTP page setting.
    Sftp(SftpKey),
    /// An Advanced page setting.
    Adv(AdvKey),
}

impl SettingId {
    /// Every setting, in page order.
    pub fn all() -> impl Iterator<Item = Self> {
        TermKey::ALL
            .iter()
            .copied()
            .map(Self::Term)
            .chain(ProfKey::ALL.iter().copied().map(Self::Prof))
            .chain(SftpKey::ALL.iter().copied().map(Self::Sftp))
            .chain(AdvKey::ALL.iter().copied().map(Self::Adv))
    }

    /// Its description.
    pub fn spec(self) -> Spec {
        match self {
            Self::Term(key) => key.spec(),
            Self::Prof(key) => key.spec(),
            Self::Sftp(key) => key.spec(),
            Self::Adv(key) => key.spec(),
        }
    }

    /// Its current value in `config`.
    pub fn get(self, config: &Config) -> Value {
        match self {
            Self::Term(key) => key.get(config),
            Self::Prof(key) => key.get(config),
            Self::Sftp(key) => key.get(config),
            Self::Adv(key) => key.get(config),
        }
    }

    /// Sets it in `config` (the running copy, after the file was written).
    pub fn set(self, config: &mut Config, value: &Value) {
        match self {
            Self::Term(key) => key.set(config, value),
            Self::Prof(key) => key.set(config, value),
            Self::Sftp(key) => key.set(config, value),
            Self::Adv(key) => key.set(config, value),
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
            (Kind::Number { .. } | Kind::Bytes { .. }, Value::Num(n)) => {
                Some(TomlValue::Int(round(*n)))
            }
            (Kind::Choice(_) | Kind::Flags(_), _) => match self {
                Self::Term(key) => key.named(value),
                Self::Prof(key) => key.named(value),
                Self::Adv(key) => key.named(value),
                Self::Sftp(_) => None,
            },
            (Kind::Folder { .. }, Value::Text(t)) if t.trim().is_empty() => None,
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

/// A count of bytes or seconds as a control value.
pub fn from_u64(n: u64) -> f64 {
    from_usize(usize::try_from(n).unwrap_or(usize::MAX))
}

/// `bytes` as people write it: `1 GiB`, `512 MiB`, `300 KiB`, `12 B`.
pub fn show_bytes(bytes: f64) -> String {
    const UNITS: [(&str, f64); 4] = [
        ("TiB", 1_099_511_627_776.0),
        ("GiB", 1_073_741_824.0),
        ("MiB", 1_048_576.0),
        ("KiB", 1024.0),
    ];
    for (unit, size) in UNITS {
        if bytes >= size {
            let n = bytes / size;
            return if (n - n.round()).abs() < 0.005 {
                format!("{n:.0} {unit}")
            } else {
                format!("{n:.2} {unit}")
            };
        }
    }
    format!("{bytes:.0} B")
}

/// Parses a size: a number with an optional unit (`B`, `K`/`KB`/`KiB`, `M…`, `G…`, `T…`;
/// all binary multiples, as file managers show them).
pub fn parse_bytes(text: &str) -> Result<f64, String> {
    let why = || "a size like 512 MiB, 2 GiB or 1048576".to_owned();
    let text = text.trim().replace('_', "");
    let split = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    let number: f64 = number.trim().parse().map_err(|_| why())?;
    let multiple = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1.0,
        "k" | "kb" | "kib" => 1024.0,
        "m" | "mb" | "mib" => 1_048_576.0,
        "g" | "gb" | "gib" => 1_073_741_824.0,
        "t" | "tb" | "tib" => 1_099_511_627_776.0,
        _ => return Err(why()),
    };
    let bytes = (number * multiple).round();
    if (0.0..9.0e18).contains(&bytes) {
        Ok(bytes)
    } else {
        Err(why())
    }
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
