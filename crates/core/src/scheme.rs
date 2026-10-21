//! Terminal colour schemes. Four built-ins are embedded from `assets/schemes/*.toml`;
//! additional schemes are TOML files in `<config dir>/themes/`, loaded by [`SchemeCatalog`].

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// An sRGB colour, 8 bits per channel (`Profile.color`); (de)serialises as a `"#RRGGBB"` TOML
/// string.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct RgbColor {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
}

impl RgbColor {
    /// Builds a colour from a `0xRRGGBB` literal.
    pub const fn from_hex(hex: u32) -> Self {
        Self {
            r: (hex >> 16) as u8,
            g: (hex >> 8) as u8,
            b: hex as u8,
        }
    }

    /// Parses `"#RRGGBB"` (case-insensitive hex digits, `#` required).
    pub fn parse_hex(s: &str) -> Result<Self, RgbColorError> {
        let digits = s
            .strip_prefix('#')
            .ok_or_else(|| RgbColorError(s.to_owned()))?;
        if digits.len() != 6 {
            return Err(RgbColorError(s.to_owned()));
        }
        let byte =
            |i: usize| u8::from_str_radix(digits.get(i..i + 2).ok_or(())?, 16).map_err(|_| ());
        let (r, g, b) = (byte(0), byte(2), byte(4));
        match (r, g, b) {
            (Ok(r), Ok(g), Ok(b)) => Ok(Self { r, g, b }),
            _ => Err(RgbColorError(s.to_owned())),
        }
    }
}

impl fmt::Display for RgbColor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

/// `s` was not a valid `"#RRGGBB"` colour.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("invalid colour {0:?}, expected \"#RRGGBB\"")]
pub struct RgbColorError(String);

impl Serialize for RgbColor {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for RgbColor {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::parse_hex(&s).map_err(D::Error::custom)
    }
}

/// Foreground/background/cursor/selection plus the 16 ANSI colours (0–7 normal, 8–15 bright).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColorScheme {
    /// Display name, matched by [`ColorScheme::by_name`] and `Config.appearance.color_scheme`.
    pub name: String,
    /// Default text colour.
    pub fg: RgbColor,
    /// Default background colour.
    pub bg: RgbColor,
    /// Cursor colour.
    pub cursor: RgbColor,
    /// Text colour drawn under a filled block cursor.
    pub cursor_text: RgbColor,
    /// Selection background (not drawn yet; the selection overlay is `fg` at low alpha,
    /// `crates/ui/src/terminal_view.rs` `draw_selection`).
    pub selection_bg: RgbColor,
    /// Selection text colour.
    pub selection_fg: RgbColor,
    /// ANSI colours 0–15.
    pub ansi: [RgbColor; 16],
}

/// The 4 built-in schemes, parsed once from the embedded TOML assets.
static BUILTINS: LazyLock<[ColorScheme; 4]> = LazyLock::new(load_builtins);

#[expect(
    clippy::expect_used,
    reason = "the asset strings are compile-time-embedded and fixed; a typo in one is a build-\
              time bug caught immediately by `four_builtins_with_distinct_names` and every other \
              test in this module, never a runtime failure mode for users"
)]
fn load_builtins() -> [ColorScheme; 4] {
    const SOURCES: [&str; 4] = [
        include_str!("../../../assets/schemes/fogu-dark.toml"),
        include_str!("../../../assets/schemes/fogu-light.toml"),
        include_str!("../../../assets/schemes/solarized-dark.toml"),
        include_str!("../../../assets/schemes/dracula.toml"),
    ];
    SOURCES.map(|src| toml::from_str(src).expect("builtin colour scheme asset"))
}

impl ColorScheme {
    /// Looks up a built-in scheme by its exact (case-sensitive) `name`.
    ///
    /// Custom schemes live in [`SchemeCatalog`]; use its [`by_name`](SchemeCatalog::by_name)
    /// when both are valid choices.
    #[must_use]
    pub fn by_name(name: &str) -> Option<Self> {
        BUILTINS.iter().find(|s| s.name == name).cloned()
    }

    /// All built-in schemes, default first.
    #[must_use]
    pub fn builtins() -> &'static [Self] {
        &*BUILTINS
    }
}

/// Directory under the Fogu config directory that holds custom scheme files.
pub const THEMES_DIR: &str = "themes";
/// Largest custom scheme file accepted, so one bad file cannot stall startup.
const MAX_SCHEME_BYTES: u64 = 256 * 1024;

/// A custom scheme file that was skipped; loading continues with the other schemes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemeWarning {
    /// The file (or themes directory) that caused the warning.
    pub path: PathBuf,
    /// Why it was skipped.
    pub message: String,
}

impl fmt::Display for SchemeWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.message)
    }
}

/// Built-in schemes plus every valid TOML file in `<config dir>/themes/`.
///
/// Custom files are sorted by file name for deterministic precedence. A custom scheme cannot
/// reuse a built-in name, and the first custom claim on a name wins. Invalid files and name
/// collisions are reported in [`warnings`](Self::warnings) without failing startup.
#[derive(Clone, Debug)]
pub struct SchemeCatalog {
    schemes: Vec<ColorScheme>,
    warnings: Vec<SchemeWarning>,
}

impl Default for SchemeCatalog {
    fn default() -> Self {
        Self::builtins()
    }
}

impl SchemeCatalog {
    /// Only the four embedded schemes.
    #[must_use]
    pub fn builtins() -> Self {
        Self {
            schemes: BUILTINS.to_vec(),
            warnings: Vec::new(),
        }
    }

    /// Loads `dir/themes/*.toml` on top of the built-ins.
    ///
    /// A missing `themes` directory is normal. Unreadable or invalid entries are skipped and
    /// reported, so a typo in one custom theme never prevents Fogu from starting.
    #[must_use]
    pub fn load(dir: &Path) -> Self {
        let mut catalog = Self::builtins();
        let themes = dir.join(THEMES_DIR);
        let entries = match std::fs::read_dir(&themes) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return catalog,
            Err(err) => {
                catalog.warnings.push(SchemeWarning {
                    path: themes,
                    message: format!("cannot read themes directory: {err}"),
                });
                return catalog;
            }
        };
        let mut files: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .and_then(std::ffi::OsStr::to_str)
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("toml"))
            })
            .collect();
        files.sort();
        for path in files {
            match Self::parse_file(&path) {
                Ok(scheme) => {
                    if let Err(message) = catalog.install(scheme) {
                        catalog.warnings.push(SchemeWarning { path, message });
                    }
                }
                Err(message) => catalog.warnings.push(SchemeWarning { path, message }),
            }
        }
        catalog
    }

    /// Every scheme, built-ins first, then custom files in file-name order.
    #[must_use]
    pub fn schemes(&self) -> &[ColorScheme] {
        &self.schemes
    }

    /// Exact (case-sensitive) lookup across built-ins and custom schemes.
    #[must_use]
    pub fn by_name(&self, name: &str) -> Option<&ColorScheme> {
        self.schemes.iter().find(|scheme| scheme.name == name)
    }

    /// Non-fatal problems found while loading custom files.
    #[must_use]
    pub fn warnings(&self) -> &[SchemeWarning] {
        &self.warnings
    }

    fn install(&mut self, scheme: ColorScheme) -> Result<(), String> {
        if ColorScheme::by_name(&scheme.name).is_some() {
            return Err(format!(
                "scheme name {:?} is reserved by a built-in scheme",
                scheme.name
            ));
        }
        if self.by_name(&scheme.name).is_some() {
            return Err(format!("duplicate scheme name {:?}", scheme.name));
        }
        self.schemes.push(scheme);
        Ok(())
    }

    fn parse_file(path: &Path) -> Result<ColorScheme, String> {
        let size = std::fs::metadata(path)
            .map_err(|err| format!("cannot inspect file: {err}"))?
            .len();
        if size > MAX_SCHEME_BYTES {
            return Err(format!(
                "file is {size} bytes, over the {MAX_SCHEME_BYTES} byte limit"
            ));
        }
        let text =
            std::fs::read_to_string(path).map_err(|err| format!("cannot read file: {err}"))?;
        let file: SchemeFile = toml::from_str(&text).map_err(|err| err.to_string())?;
        let name = file.name.trim();
        if name.is_empty() {
            return Err("the name is empty".to_owned());
        }
        let base = file
            .extends
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(|name| {
                ColorScheme::by_name(name)
                    .ok_or_else(|| format!("extends unknown built-in scheme {name:?}"))
            })
            .transpose()?;
        let mut scheme = match base {
            Some(base) => base,
            None => ColorScheme {
                name: name.to_owned(),
                fg: require(file.fg, "fg")?,
                bg: require(file.bg, "bg")?,
                cursor: require(file.cursor, "cursor")?,
                cursor_text: require(file.cursor_text, "cursor_text")?,
                selection_bg: require(file.selection_bg, "selection_bg")?,
                selection_fg: require(file.selection_fg, "selection_fg")?,
                ansi: require(file.ansi, "ansi")?,
            },
        };
        name.clone_into(&mut scheme.name);
        if let Some(value) = file.fg {
            scheme.fg = value;
        }
        if let Some(value) = file.bg {
            scheme.bg = value;
        }
        if let Some(value) = file.cursor {
            scheme.cursor = value;
        }
        if let Some(value) = file.cursor_text {
            scheme.cursor_text = value;
        }
        if let Some(value) = file.selection_bg {
            scheme.selection_bg = value;
        }
        if let Some(value) = file.selection_fg {
            scheme.selection_fg = value;
        }
        if let Some(value) = file.ansi {
            scheme.ansi = value;
        }
        Ok(scheme)
    }
}

fn require<T>(value: Option<T>, field: &str) -> Result<T, String> {
    value.ok_or_else(|| format!("missing {field} (required without extends)"))
}

/// A custom scheme file. Every colour is optional so `extends` can inherit it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SchemeFile {
    name: String,
    #[serde(default)]
    extends: Option<String>,
    #[serde(default)]
    fg: Option<RgbColor>,
    #[serde(default)]
    bg: Option<RgbColor>,
    #[serde(default)]
    cursor: Option<RgbColor>,
    #[serde(default)]
    cursor_text: Option<RgbColor>,
    #[serde(default)]
    selection_bg: Option<RgbColor>,
    #[serde(default)]
    selection_fg: Option<RgbColor>,
    #[serde(default)]
    ansi: Option<[RgbColor; 16]>,
}

impl Default for ColorScheme {
    /// "Fogu Dark", the same values as `assets/schemes/fogu-dark.toml` (kept as a
    /// hardcoded fallback — unlike [`by_name`](Self::by_name) this can never fail to parse — and
    /// checked against the asset by the `default_matches_builtin_asset` test below).
    fn default() -> Self {
        const ANSI: [u32; 16] = [
            0x00_0000, 0xff_615a, 0xb1_e969, 0xeb_d99c, 0x5d_a9f6, 0xe8_6aff, 0x82_fff7, 0xde_dacf,
            0x76_7676, 0xf5_8c80, 0xdd_f88f, 0xee_e5b2, 0xa5_c7ff, 0xdd_aaff, 0xb7_fff9, 0xff_ffff,
        ];
        Self {
            name: "Fogu Dark".to_owned(),
            fg: RgbColor::from_hex(0xca_caca),
            bg: RgbColor::from_hex(0x17_1717),
            cursor: RgbColor::from_hex(0xbb_bbbb),
            cursor_text: RgbColor::from_hex(0x17_1717),
            selection_bg: RgbColor::from_hex(0xca_caca),
            selection_fg: RgbColor::from_hex(0x17_1717),
            ansi: ANSI.map(RgbColor::from_hex),
        }
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::unwrap_used, reason = "tests")]

    use super::*;

    #[test]
    fn from_hex_splits_channels() {
        assert_eq!(
            RgbColor::from_hex(0x12_3456),
            RgbColor {
                r: 0x12,
                g: 0x34,
                b: 0x56
            }
        );
    }

    #[test]
    fn parses_and_displays_hash_rrggbb() {
        assert_eq!(
            RgbColor::parse_hex("#ca1234").unwrap(),
            RgbColor {
                r: 0xca,
                g: 0x12,
                b: 0x34
            }
        );
        assert_eq!(RgbColor::from_hex(0x00_ff80).to_string(), "#00ff80");
        for bad in ["ca1234", "#ca123", "#ca12345", "#zzzzzz", ""] {
            assert!(RgbColor::parse_hex(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn default_matches_builtin_asset() {
        assert_eq!(
            ColorScheme::default(),
            ColorScheme::by_name("Fogu Dark").unwrap()
        );
    }

    #[test]
    fn four_builtins_with_distinct_names() {
        let builtins = ColorScheme::builtins();
        assert_eq!(builtins.len(), 4);
        let names: Vec<&str> = builtins.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names.iter().collect::<std::collections::HashSet<_>>().len(),
            4
        );
        assert!(ColorScheme::by_name("Dracula").is_some());
        assert!(ColorScheme::by_name("nonexistent scheme").is_none());
    }

    struct Tmp(PathBuf);
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn tmp() -> Tmp {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("fogu-scheme-{}-{n}", std::process::id()));
        std::fs::create_dir_all(dir.join(THEMES_DIR)).unwrap();
        Tmp(dir)
    }

    fn write(t: &Tmp, file: &str, text: &str) {
        std::fs::write(t.0.join(THEMES_DIR).join(file), text).unwrap();
    }

    const FULL: &str = r##"
name = "Test Full"
fg = "#111111"
bg = "#222222"
cursor = "#333333"
cursor_text = "#444444"
selection_bg = "#555555"
selection_fg = "#666666"
ansi = [
  "#000000", "#010101", "#020202", "#030303",
  "#040404", "#050505", "#060606", "#070707",
  "#080808", "#090909", "#0a0a0a", "#0b0b0b",
  "#0c0c0c", "#0d0d0d", "#0e0e0e", "#0f0f0f",
]
"##;

    #[test]
    fn loads_a_complete_custom_scheme() {
        let t = tmp();
        write(&t, "full.toml", FULL);
        let catalog = SchemeCatalog::load(&t.0);
        assert!(catalog.warnings().is_empty(), "{:?}", catalog.warnings());
        assert_eq!(catalog.schemes().len(), 5);
        let scheme = catalog.by_name("Test Full").unwrap();
        assert_eq!(scheme.bg, RgbColor::from_hex(0x22_2222));
        assert_eq!(scheme.ansi[15], RgbColor::from_hex(0x0f_0f0f));
    }

    #[test]
    fn extends_inherits_every_missing_colour() {
        let t = tmp();
        write(
            &t,
            "extended.toml",
            r##"
name = "Test Extended"
extends = "Fogu Dark"
bg = "#010203"
fg = "#040506"
cursor = "#070809"
"##,
        );
        let catalog = SchemeCatalog::load(&t.0);
        assert!(catalog.warnings().is_empty(), "{:?}", catalog.warnings());
        let base = ColorScheme::by_name("Fogu Dark").unwrap();
        let scheme = catalog.by_name("Test Extended").unwrap();
        assert_eq!(scheme.name, "Test Extended");
        assert_eq!(scheme.bg, RgbColor::from_hex(0x01_0203));
        assert_eq!(scheme.fg, RgbColor::from_hex(0x04_0506));
        assert_eq!(scheme.cursor, RgbColor::from_hex(0x07_0809));
        assert_eq!(scheme.cursor_text, base.cursor_text);
        assert_eq!(scheme.selection_bg, base.selection_bg);
        assert_eq!(scheme.ansi, base.ansi);
    }

    #[test]
    fn invalid_reserved_and_incomplete_files_are_skipped() {
        let t = tmp();
        write(&t, "bad.toml", "not valid toml [[[");
        write(
            &t,
            "reserved.toml",
            "name = \"Dracula\"\nextends = \"Fogu Dark\"\n",
        );
        write(&t, "missing.toml", "name = \"No Colours\"\n");
        let catalog = SchemeCatalog::load(&t.0);
        assert_eq!(catalog.schemes().len(), 4);
        assert_eq!(catalog.warnings().len(), 3);
        let messages: Vec<&str> = catalog
            .warnings()
            .iter()
            .map(|warning| warning.message.as_str())
            .collect();
        assert!(
            messages.iter().any(|m| m.contains("missing fg")),
            "{messages:?}"
        );
        assert!(
            messages.iter().any(|m| m.contains("reserved")),
            "{messages:?}"
        );
    }

    #[test]
    fn first_custom_name_wins_deterministically() {
        let t = tmp();
        write(
            &t,
            "a.toml",
            "name = \"Same\"\nextends = \"Fogu Dark\"\nbg = \"#010101\"\n",
        );
        write(
            &t,
            "b.toml",
            "name = \"Same\"\nextends = \"Fogu Dark\"\nbg = \"#020202\"\n",
        );
        let catalog = SchemeCatalog::load(&t.0);
        assert_eq!(catalog.schemes().len(), 5);
        assert_eq!(catalog.warnings().len(), 1);
        assert!(
            catalog
                .warnings()
                .first()
                .is_some_and(|warning| warning.message.contains("duplicate")),
            "{:?}",
            catalog.warnings()
        );
        assert_eq!(
            catalog.by_name("Same").unwrap().bg,
            RgbColor::from_hex(0x01_0101)
        );
    }

    #[test]
    fn unknown_extends_is_rejected() {
        let t = tmp();
        write(
            &t,
            "unknown.toml",
            "name = \"Orphan\"\nextends = \"No Such Base\"\n",
        );
        let catalog = SchemeCatalog::load(&t.0);
        assert_eq!(catalog.schemes().len(), 4);
        assert!(
            catalog
                .warnings()
                .first()
                .is_some_and(|warning| warning.message.contains("unknown built-in")),
            "{:?}",
            catalog.warnings()
        );
    }
}
