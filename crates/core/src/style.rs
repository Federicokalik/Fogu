//! UI styles: the window chrome's structure and form (tab bar placement and anatomy, chrome
//! geometry, density, radii, shadows, typography, interaction and dialog language), independent
//! from the terminal [`ColorScheme`](crate::ColorScheme). Built-ins are embedded from
//! `assets/styles/*.toml`; custom styles are TOML files in `<config dir>/styles/` that `extends`
//! a built-in and override any section, loaded by [`StyleCatalog`].
//!
//! Everything here is plain data. The UI turns a [`StyleDef`] plus the active scheme into
//! concrete colours and widget styles; colours are recipes ([`Mix`]) over the scheme so every
//! style works with dark and light palettes.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

use crate::catalog::{CatalogWarning, read_toml_dir, write_custom};
use crate::scheme::RgbColor;

/// Directory under the Fogu config directory that holds custom style files.
pub const STYLES_DIR: &str = "styles";

/// A custom style file that was skipped (or clamped); loading continues with the others.
pub type StyleWarning = CatalogWarning;

/// A colour a [`Mix`] starts from or moves toward. Roles resolve in the order the
/// [`SurfaceSpec`] fields are listed; a role not resolved yet stands for `Base`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    /// The terminal background.
    Base,
    /// The terminal foreground.
    Fg,
    /// The chrome background ([`SurfaceSpec::bg`]).
    Bg,
    /// The title/tab bar ([`SurfaceSpec::raised`]).
    Raised,
    /// Docked panels ([`SurfaceSpec::panel`]).
    Panel,
    /// The style accent.
    Accent,
    /// Pure black.
    Black,
    /// Pure white.
    White,
}

/// `from` moved toward `toward` by `dark` (dark schemes) or `light` (light schemes), `0..=1`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mix {
    /// Starting colour.
    pub from: Role,
    /// Target colour.
    pub toward: Role,
    /// Share of `toward` on a dark scheme.
    pub dark: f32,
    /// Share of `toward` on a light scheme.
    pub light: f32,
}

/// What a shadow is tinted with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ShadowTint {
    /// Neutral black.
    Black,
    /// Black moved toward the accent by [`ShadowSpec::mix`] (`1` = a pure accent glow).
    Accent,
    /// Black moved toward the secondary accent (the accent when there is none).
    Secondary,
}

/// Where the tab strip sits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BarPlacement {
    /// The tab bar is the title bar.
    Top,
    /// Tabs form a status line under the panes; a thin strip on top keeps the window controls.
    Bottom,
}

/// How the bar is filled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BarFill {
    /// The raised surface colour.
    Solid,
    /// The terminal background (full-bleed chrome).
    Base,
    /// Raised, with the accent fading in from the top.
    Vertical,
    /// Raised, with the accent on the left fading into the secondary accent on the right.
    Horizontal,
}

/// The shape language of a tab header.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TabStyle {
    /// Floating rounded chips inside the bar.
    Pill,
    /// Folder tabs: rounded top corners, the active one merges into the terminal.
    Folder,
    /// No fill; the active tab is underlined.
    Underline,
    /// The active tab is drawn in inverse video.
    Inverse,
    /// A soft fill with a glowing underline.
    Glow,
}

/// How wide tab headers are.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TabSizing {
    /// Every header is [`TabSpec::width`] wide.
    Fixed,
    /// Headers share the strip, each between the min and max width.
    Fill,
    /// Each header fits its title, between the min and max width.
    Fit,
}

/// What precedes a tab title.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TabMarker {
    /// A small colour dot (the tab colour or the accent).
    Dot,
    /// The tab's 1-based position.
    Index,
    /// Nothing.
    None,
}

/// When a tab shows its close button.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TabClose {
    /// On every tab.
    Always,
    /// On the active and the hovered tab.
    Hover,
    /// Never (middle click and the menu still close tabs).
    Never,
}

/// Colour of the active tab's indicator line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IndicatorColor {
    /// The accent.
    Accent,
    /// The foreground (accent stays reserved for focus and selection).
    Fg,
}

/// How the chrome sits around the terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Geometry {
    /// Panels touch the window edges and each other.
    Docked,
    /// The bar, the terminal and the sidebar are rounded islands over a canvas.
    Islands,
}

/// How the profile sidebar is presented.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SidebarMode {
    /// Flush with the window edge, separated by a line.
    Docked,
    /// A detached card with its own radius and shadow.
    Floating,
}

/// Row height of lists (sidebar, picker, settings).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Density {
    /// 26 px rows.
    Compact,
    /// 30 px rows.
    Regular,
    /// 34 px rows.
    Comfy,
}

impl Density {
    /// The row height in logical px.
    #[must_use]
    pub const fn row_height(self) -> f32 {
        match self {
            Self::Compact => 26.0,
            Self::Regular => 30.0,
            Self::Comfy => 34.0,
        }
    }
}

/// The chrome's typeface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChromeFont {
    /// The bundled interface sans-serif.
    Ui,
    /// The terminal's monospace font.
    Mono,
}

/// Letter case of section labels and tab titles' chrome.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LabelCase {
    /// UPPER CASE.
    Upper,
    /// As written.
    Sentence,
}

/// A font weight for emphasis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Weight {
    /// Regular.
    Regular,
    /// Medium.
    Medium,
}

/// How hovered rows and flat buttons react.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HoverMode {
    /// A translucent fill.
    Fill,
    /// Inverse video.
    Inverse,
    /// A 1 px outline.
    Outline,
}

/// The form of dialog buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ButtonForm {
    /// The primary is filled with the accent, the others with a neutral fill.
    Filled,
    /// Outlined; the primary outline is the accent.
    Outline,
    /// Text only; the primary is accent text.
    Text,
    /// Bracketed inverse video, like a terminal UI.
    Inverse,
}

/// How selected rows are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SelectionMode {
    /// A soft accent fill with an accent border.
    Fill,
    /// Inverse video.
    Inverse,
}

/// Where small dialogs (confirmations, prompts) appear.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DialogPlacement {
    /// A centred card.
    Center,
    /// A sheet hanging from the top edge.
    TopSheet,
    /// A full-width command line at the bottom.
    CommandBar,
}

/// Generates the section struct (every field required, as in the built-in assets) and its
/// patch (every field optional, as in custom files), both rejecting unknown keys.
macro_rules! patchable {
    (
        $(#[$meta:meta])*
        pub struct $name:ident / $patch:ident {
            $( $(#[$fmeta:meta])* pub $field:ident : $ty:ty, )*
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            $( $(#[$fmeta])* pub $field: $ty, )*
        }

        /// The optional form of the section, as written in a custom style file.
        #[derive(Clone, Debug, Default, Deserialize)]
        #[serde(deny_unknown_fields)]
        struct $patch {
            $( #[serde(default)] $field: Option<$ty>, )*
        }

        impl $name {
            fn apply(&mut self, patch: $patch) {
                $( if let Some(value) = patch.$field { self.$field = value; } )*
            }
        }
    };
}

patchable! {
    /// Accent colours.
    pub struct ColorSpec / ColorPatch {
        /// The one accent: selection, focus, active controls (overridable in config).
        pub accent: RgbColor,
        /// An optional second accent, used only for focus rings and gradient ends.
        pub secondary: Option<RgbColor>,
        /// Destructive actions and errors.
        pub danger: RgbColor,
    }
}

patchable! {
    /// Chrome colours as recipes over the scheme.
    pub struct SurfaceSpec / SurfacePatch {
        /// Menus, dialogs, the picker.
        pub bg: Mix,
        /// The title / tab bar.
        pub raised: Mix,
        /// Docked panels (sidebar, settings cards).
        pub panel: Mix,
        /// Behind islands (only drawn with [`Geometry::Islands`]).
        pub canvas: Mix,
        /// 1 px borders.
        pub border: Mix,
        /// Split dividers between panes.
        pub divider: Mix,
        /// Muted text.
        pub dim: Mix,
        /// Alpha of the soft accent fill on dark schemes.
        pub soft_dark: f32,
        /// Alpha of the soft accent fill on light schemes.
        pub soft_light: f32,
        /// Alpha of the foreground used as hover fill.
        pub hover: f32,
    }
}

patchable! {
    /// Radii and stroke widths.
    pub struct ShapeSpec / ShapePatch {
        /// Cards, dialogs and floating panels.
        pub card: f32,
        /// Rows, inputs and buttons.
        pub control: f32,
        /// Top corners of tab headers.
        pub tab_top: f32,
        /// Bottom corners of tab headers.
        pub tab_bottom: f32,
        /// Colour dots (tab markers, status).
        pub dot: f32,
        /// Border width of panels, cards and fields.
        pub border: f32,
        /// Border width of a focused field.
        pub focus: f32,
    }
}

patchable! {
    /// One shadow.
    pub struct ShadowSpec / ShadowPatch {
        /// Horizontal offset.
        pub x: f32,
        /// Vertical offset.
        pub y: f32,
        /// Blur radius (0 = a hard shadow).
        pub blur: f32,
        /// Opacity (0 = no shadow).
        pub alpha: f32,
        /// Tint.
        pub tint: ShadowTint,
        /// How far black moves toward the tint colour.
        pub mix: f32,
    }
}

/// The shadows of a style.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShadowSet {
    /// Docked panels.
    pub panel: ShadowSpec,
    /// Menus, dialogs, toasts, floating cards.
    pub floating: ShadowSpec,
    /// The active tab header.
    pub tab: ShadowSpec,
}

/// [`ShadowSet`] in a custom file: each shadow is patched field by field.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ShadowSetPatch {
    #[serde(default)]
    panel: Option<ShadowPatch>,
    #[serde(default)]
    floating: Option<ShadowPatch>,
    #[serde(default)]
    tab: Option<ShadowPatch>,
}

impl ShadowSet {
    fn apply(&mut self, patch: ShadowSetPatch) {
        if let Some(p) = patch.panel {
            self.panel.apply(p);
        }
        if let Some(p) = patch.floating {
            self.floating.apply(p);
        }
        if let Some(p) = patch.tab {
            self.tab.apply(p);
        }
    }
}

patchable! {
    /// The tab / title bar.
    pub struct BarSpec / BarPatch {
        /// Top (title bar) or bottom (status line).
        pub placement: BarPlacement,
        /// Height of the tab strip.
        pub height: f32,
        /// Height of the top strip that keeps the window controls when tabs sit at the bottom.
        pub strip: f32,
        /// Fill.
        pub fill: BarFill,
        /// Accent share of a gradient fill.
        pub gradient: f32,
        /// A 1 px line between the bar and the panes.
        pub rule: bool,
    }
}

patchable! {
    /// Tab headers.
    pub struct TabSpec / TabPatch {
        /// Shape language.
        pub style: TabStyle,
        /// Width policy.
        pub sizing: TabSizing,
        /// Width with [`TabSizing::Fixed`].
        pub width: f32,
        /// Smallest width with `fill` / `fit`.
        pub min_width: f32,
        /// Largest width with `fill` / `fit`.
        pub max_width: f32,
        /// What precedes the title.
        pub marker: TabMarker,
        /// When the close button shows.
        pub close: TabClose,
        /// 1 px separators between inactive headers.
        pub separators: bool,
        /// Gap between a header and the bar edges.
        pub inset: f32,
        /// Title size.
        pub title_size: f32,
        /// Thickness of the active indicator line (0 = none).
        pub indicator: f32,
        /// Length of the indicator (0 = the header width).
        pub indicator_length: f32,
        /// Colour of the indicator.
        pub indicator_color: IndicatorColor,
    }
}

patchable! {
    /// Layout and density.
    pub struct LayoutSpec / LayoutPatch {
        /// Docked or islands.
        pub geometry: Geometry,
        /// Gap around islands.
        pub margin: f32,
        /// Sidebar presentation.
        pub sidebar: SidebarMode,
        /// List row height.
        pub density: Density,
        /// Horizontal padding of panel headers and rows.
        pub gutter: f32,
        /// Height of the drag strip above split panes.
        pub grip: f32,
    }
}

patchable! {
    /// Typography.
    pub struct TextSpec / TextPatch {
        /// Chrome typeface.
        pub chrome: ChromeFont,
        /// Section label case.
        pub labels: LabelCase,
        /// Weight of active tab titles and headings.
        pub active: Weight,
    }
}

patchable! {
    /// Interaction language.
    pub struct InteractionSpec / InteractionPatch {
        /// Hover feedback.
        pub hover: HoverMode,
        /// Dialog button form.
        pub button: ButtonForm,
        /// Selected rows.
        pub selection: SelectionMode,
    }
}

patchable! {
    /// Small dialogs.
    pub struct DialogSpec / DialogPatch {
        /// Placement.
        pub placement: DialogPlacement,
        /// Actions in a separate footer bar.
        pub footer: bool,
        /// Opacity of the dimming behind dialogs.
        pub backdrop: f32,
    }
}

/// A complete UI style.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StyleDef {
    /// Stable config value (`appearance.ui_theme`); a custom style's key is its name.
    pub key: String,
    /// Display name.
    pub name: String,
    /// Other accepted config values.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// One line for the settings page.
    pub description: String,
    /// The terminal scheme designed for this style, suggested when switching to it.
    #[serde(default)]
    pub paired_scheme: Option<String>,
    /// Accents.
    pub color: ColorSpec,
    /// Chrome colours.
    pub surface: SurfaceSpec,
    /// Radii and strokes.
    pub shape: ShapeSpec,
    /// Shadows.
    pub shadow: ShadowSet,
    /// Tab bar.
    pub bar: BarSpec,
    /// Tab headers.
    pub tabs: TabSpec,
    /// Layout and density.
    pub layout: LayoutSpec,
    /// Typography.
    pub text: TextSpec,
    /// Interaction.
    pub interaction: InteractionSpec,
    /// Dialogs.
    pub dialog: DialogSpec,
}

impl StyleDef {
    /// Whether `name` (trimmed, any case) is this style's key, name or one of its aliases.
    #[must_use]
    pub fn answers_to(&self, name: &str) -> bool {
        let name = name.trim();
        self.key.eq_ignore_ascii_case(name)
            || self.name.eq_ignore_ascii_case(name)
            || self.aliases.iter().any(|a| a.eq_ignore_ascii_case(name))
    }

    /// Clamps every number into its supported range; one message per value changed.
    pub fn sanitize(&mut self) -> Vec<String> {
        let mut notes = Vec::new();
        let mut clamp = |value: &mut f32, lo: f32, hi: f32, label: &str| {
            let clamped = if value.is_finite() {
                value.clamp(lo, hi)
            } else {
                lo
            };
            if (clamped - *value).abs() > f32::EPSILON || !value.is_finite() {
                notes.push(format!(
                    "{label} = {value} is out of range; using {clamped}"
                ));
                *value = clamped;
            }
        };
        let surface = &mut self.surface;
        for (mix, label) in [
            (&mut surface.bg, "surface.bg"),
            (&mut surface.raised, "surface.raised"),
            (&mut surface.panel, "surface.panel"),
            (&mut surface.canvas, "surface.canvas"),
            (&mut surface.border, "surface.border"),
            (&mut surface.divider, "surface.divider"),
            (&mut surface.dim, "surface.dim"),
        ] {
            clamp(&mut mix.dark, 0.0, 1.0, &format!("{label}.dark"));
            clamp(&mut mix.light, 0.0, 1.0, &format!("{label}.light"));
        }
        clamp(&mut surface.soft_dark, 0.0, 1.0, "surface.soft_dark");
        clamp(&mut surface.soft_light, 0.0, 1.0, "surface.soft_light");
        clamp(&mut surface.hover, 0.0, 1.0, "surface.hover");
        let shape = &mut self.shape;
        clamp(&mut shape.card, 0.0, 32.0, "shape.card");
        clamp(&mut shape.control, 0.0, 32.0, "shape.control");
        clamp(&mut shape.tab_top, 0.0, 32.0, "shape.tab_top");
        clamp(&mut shape.tab_bottom, 0.0, 32.0, "shape.tab_bottom");
        clamp(&mut shape.dot, 0.0, 8.0, "shape.dot");
        clamp(&mut shape.border, 0.0, 4.0, "shape.border");
        clamp(&mut shape.focus, 0.0, 4.0, "shape.focus");
        for (shadow, label) in [
            (&mut self.shadow.panel, "shadow.panel"),
            (&mut self.shadow.floating, "shadow.floating"),
            (&mut self.shadow.tab, "shadow.tab"),
        ] {
            clamp(&mut shadow.x, -32.0, 32.0, &format!("{label}.x"));
            clamp(&mut shadow.y, -32.0, 32.0, &format!("{label}.y"));
            clamp(&mut shadow.blur, 0.0, 48.0, &format!("{label}.blur"));
            clamp(&mut shadow.alpha, 0.0, 1.0, &format!("{label}.alpha"));
            clamp(&mut shadow.mix, 0.0, 1.0, &format!("{label}.mix"));
        }
        let bar = &mut self.bar;
        clamp(&mut bar.height, 24.0, 64.0, "bar.height");
        clamp(&mut bar.strip, 20.0, 48.0, "bar.strip");
        clamp(&mut bar.gradient, 0.0, 1.0, "bar.gradient");
        let tabs = &mut self.tabs;
        clamp(&mut tabs.width, 60.0, 400.0, "tabs.width");
        clamp(&mut tabs.min_width, 40.0, 400.0, "tabs.min_width");
        clamp(&mut tabs.max_width, tabs.min_width, 600.0, "tabs.max_width");
        clamp(&mut tabs.inset, 0.0, 12.0, "tabs.inset");
        clamp(&mut tabs.title_size, 9.0, 18.0, "tabs.title_size");
        clamp(&mut tabs.indicator, 0.0, 6.0, "tabs.indicator");
        clamp(
            &mut tabs.indicator_length,
            0.0,
            200.0,
            "tabs.indicator_length",
        );
        let layout = &mut self.layout;
        clamp(&mut layout.margin, 0.0, 24.0, "layout.margin");
        clamp(&mut layout.gutter, 4.0, 24.0, "layout.gutter");
        clamp(&mut layout.grip, 2.0, 16.0, "layout.grip");
        clamp(&mut self.dialog.backdrop, 0.0, 1.0, "dialog.backdrop");
        notes
    }
}

/// A custom style file: a name, the built-in it extends, and any section overrides.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StyleFile {
    name: String,
    #[serde(default)]
    extends: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    paired_scheme: Option<String>,
    #[serde(default)]
    color: Option<ColorPatch>,
    #[serde(default)]
    surface: Option<SurfacePatch>,
    #[serde(default)]
    shape: Option<ShapePatch>,
    #[serde(default)]
    shadow: Option<ShadowSetPatch>,
    #[serde(default)]
    bar: Option<BarPatch>,
    #[serde(default)]
    tabs: Option<TabPatch>,
    #[serde(default)]
    layout: Option<LayoutPatch>,
    #[serde(default)]
    text: Option<TextPatch>,
    #[serde(default)]
    interaction: Option<InteractionPatch>,
    #[serde(default)]
    dialog: Option<DialogPatch>,
}

/// The built-in styles, default first, parsed once from the embedded assets.
static BUILTINS: LazyLock<Vec<StyleDef>> = LazyLock::new(load_builtins);

/// The embedded built-in style assets, default first.
const BUILTIN_SOURCES: &[&str] = &[
    include_str!("../../../assets/styles/ember.toml"),
    include_str!("../../../assets/styles/mica.toml"),
    include_str!("../../../assets/styles/ink.toml"),
    include_str!("../../../assets/styles/phosphor.toml"),
    include_str!("../../../assets/styles/synthwave.toml"),
    include_str!("../../../assets/styles/glass.toml"),
];

#[expect(
    clippy::expect_used,
    reason = "the asset strings are compile-time-embedded and fixed; a typo in one is a build-\
              time bug caught immediately by `builtins_parse_and_are_distinct`, never a runtime \
              failure mode for users"
)]
fn load_builtins() -> Vec<StyleDef> {
    BUILTIN_SOURCES
        .iter()
        .map(|src| toml::from_str(src).expect("builtin UI style asset"))
        .collect()
}

/// Built-in styles plus every valid TOML file in `<config dir>/styles/`.
///
/// Files are applied in file-name order. A custom style cannot answer to a built-in's key, name
/// or alias, and the first custom claim on a name wins. Invalid files, clamped values and name
/// collisions are reported in [`warnings`](Self::warnings) without failing startup.
#[derive(Clone, Debug)]
pub struct StyleCatalog {
    styles: Vec<StyleDef>,
    warnings: Vec<StyleWarning>,
    /// The file each custom style came from, by key.
    sources: Vec<(String, PathBuf)>,
}

impl Default for StyleCatalog {
    fn default() -> Self {
        Self::builtins()
    }
}

impl StyleCatalog {
    /// Only the embedded styles.
    #[must_use]
    pub fn builtins() -> Self {
        Self {
            styles: BUILTINS.clone(),
            warnings: Vec::new(),
            sources: Vec::new(),
        }
    }

    /// Loads `dir/styles/*.toml` on top of the built-ins.
    #[must_use]
    pub fn load(dir: &Path) -> Self {
        let mut catalog = Self::builtins();
        let (files, warnings) = read_toml_dir(&dir.join(STYLES_DIR));
        catalog.warnings = warnings;
        for (path, text) in files {
            match parse_custom(&text) {
                Ok((style, notes)) => {
                    for message in notes {
                        catalog.warnings.push(StyleWarning {
                            path: path.clone(),
                            message,
                        });
                    }
                    let key = style.key.clone();
                    match catalog.install(style) {
                        Ok(()) => catalog.sources.push((key, path)),
                        Err(message) => catalog.warnings.push(StyleWarning { path, message }),
                    }
                }
                Err(message) => catalog.warnings.push(StyleWarning { path, message }),
            }
        }
        catalog
    }

    /// Every style, built-ins first, then custom files in file-name order.
    #[must_use]
    pub fn styles(&self) -> &[StyleDef] {
        &self.styles
    }

    /// The style answering to `name` (key, display name or alias; trimmed, any case).
    #[must_use]
    pub fn resolve(&self, name: &str) -> Option<&StyleDef> {
        self.styles.iter().find(|style| style.answers_to(name))
    }

    /// The default style (the first built-in).
    #[must_use]
    pub fn default_style(&self) -> &StyleDef {
        // `styles` always starts with the built-ins, which are never empty.
        match self.styles.first() {
            Some(style) => style,
            None => default_builtin(),
        }
    }

    /// Whether `style` is one of the embedded styles.
    #[must_use]
    pub fn is_builtin(style: &StyleDef) -> bool {
        BUILTINS.iter().any(|b| b.key == style.key)
    }

    /// Non-fatal problems found while loading custom files.
    #[must_use]
    pub fn warnings(&self) -> &[StyleWarning] {
        &self.warnings
    }

    /// The file custom style `key` was loaded from.
    #[must_use]
    pub fn source(&self, key: &str) -> Option<&Path> {
        self.sources
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, path)| path.as_path())
    }

    /// Checks custom style `text` as [`load`](Self::load) would: it parses, its name is not
    /// a built-in's, and no other custom file claims it (`editing` is the file being replaced).
    /// Returns the style and the clamp notes.
    pub fn check(
        &self,
        text: &str,
        editing: Option<&Path>,
    ) -> Result<(StyleDef, Vec<String>), String> {
        let (style, notes) = parse_custom(text)?;
        if BUILTINS.iter().any(|b| b.answers_to(&style.name)) {
            return Err(format!(
                "the name {:?} is reserved by a built-in style",
                style.name
            ));
        }
        let taken = self.resolve(&style.name).is_some_and(|other| {
            self.source(&other.key)
                .is_none_or(|path| Some(path) != editing)
        });
        if taken {
            return Err(format!("another style is already named {:?}", style.name));
        }
        Ok((style, notes))
    }

    /// Validates `text` with [`check`](Self::check) and writes it to `dir/styles/` (over
    /// `editing` when given). Returns the style's key and the file written.
    pub fn save(
        &self,
        dir: &Path,
        text: &str,
        editing: Option<&Path>,
    ) -> Result<(String, PathBuf), String> {
        let (style, _) = self.check(text, editing)?;
        let path = write_custom(&dir.join(STYLES_DIR), &style.name, text, editing)?;
        Ok((style.key, path))
    }

    fn install(&mut self, style: StyleDef) -> Result<(), String> {
        if BUILTINS.iter().any(|b| b.answers_to(&style.name)) {
            return Err(format!(
                "style name {:?} is reserved by a built-in style",
                style.name
            ));
        }
        if self.resolve(&style.name).is_some() {
            return Err(format!("duplicate style name {:?}", style.name));
        }
        self.styles.push(style);
        Ok(())
    }
}

#[expect(
    clippy::indexing_slicing,
    reason = "BUILTIN_SOURCES is a non-empty constant, so the parsed list always has a first entry"
)]
fn default_builtin() -> &'static StyleDef {
    &BUILTINS[0]
}

/// Parses a custom style file into a complete style plus the clamp notes (values moved into
/// range). Does not check the name against other styles; [`StyleCatalog::check`] does.
pub fn parse_custom(text: &str) -> Result<(StyleDef, Vec<String>), String> {
    let file: StyleFile = toml::from_str(text).map_err(|err| err.to_string())?;
    let name = file.name.trim();
    if name.is_empty() {
        return Err("the name is empty".to_owned());
    }
    let base_name = file
        .extends
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty());
    let mut style = match base_name {
        Some(base) => BUILTINS
            .iter()
            .find(|b| b.answers_to(base))
            .cloned()
            .ok_or_else(|| format!("extends unknown built-in style {base:?}"))?,
        None => default_builtin().clone(),
    };
    let based_on = std::mem::take(&mut style.name);
    name.clone_into(&mut style.key);
    name.clone_into(&mut style.name);
    style.aliases.clear();
    style.description = file
        .description
        .unwrap_or_else(|| format!("Custom style based on {based_on}"));
    if let Some(scheme) = file.paired_scheme {
        style.paired_scheme = Some(scheme).filter(|s| !s.trim().is_empty());
    }
    if let Some(p) = file.color {
        style.color.apply(p);
    }
    if let Some(p) = file.surface {
        style.surface.apply(p);
    }
    if let Some(p) = file.shape {
        style.shape.apply(p);
    }
    if let Some(p) = file.shadow {
        style.shadow.apply(p);
    }
    if let Some(p) = file.bar {
        style.bar.apply(p);
    }
    if let Some(p) = file.tabs {
        style.tabs.apply(p);
    }
    if let Some(p) = file.layout {
        style.layout.apply(p);
    }
    if let Some(p) = file.text {
        style.text.apply(p);
    }
    if let Some(p) = file.interaction {
        style.interaction.apply(p);
    }
    if let Some(p) = file.dialog {
        style.dialog.apply(p);
    }
    let notes = style.sanitize();
    Ok((style, notes))
}

#[cfg(test)]
#[path = "style_tests.rs"]
mod tests;
