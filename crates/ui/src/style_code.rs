//! Settings → Appearance → "from code": write a custom UI style or terminal palette as TOML in
//! the app. Every edit is checked the way the catalogs load files (parse, name clashes, values
//! clamped) and previewed live; Save writes the file to `styles/` or `themes/` and applies it.

use std::fmt::Write as _;
use std::path::PathBuf;

use fogu_core::{ColorScheme, RgbColor, SchemeCatalog, StyleCatalog, StyleDef};
use iced::widget::{column, container, row, scrollable, space, text, text_editor};
use iced::{Alignment, Border, Element, Length};
use serde::Serialize;

use crate::controls::{self, ButtonKind};
use crate::design::Env;
use crate::message::Message;
use crate::palette::Look;
use crate::settings::SettingsMsg;
use crate::style_preview;

/// What the editor writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeKind {
    /// A UI style (`styles/`).
    Style,
    /// A terminal colour scheme (`themes/`).
    Palette,
}

/// The result of checking the editor's text.
#[derive(Debug)]
pub enum Check {
    /// A valid style, with the values that were clamped into range.
    Style(Box<StyleDef>, Vec<String>),
    /// A valid palette.
    Palette(Box<ColorScheme>),
    /// Why the text cannot be saved.
    Invalid(String),
}

/// The open code editor.
#[derive(Debug)]
pub struct CodeEditor {
    /// What it writes.
    pub kind: CodeKind,
    /// The TOML being edited.
    pub content: text_editor::Content,
    /// The custom file being edited (`None` = a new file).
    pub editing: Option<PathBuf>,
    /// The last check of `content`.
    pub check: Check,
    /// Why the last Save failed.
    pub error: Option<String>,
}

impl CodeEditor {
    /// An editor on `text`, checked right away.
    pub fn new(
        kind: CodeKind,
        text: &str,
        editing: Option<PathBuf>,
        (styles, schemes): (&StyleCatalog, &SchemeCatalog),
    ) -> Self {
        let mut editor = Self {
            kind,
            content: text_editor::Content::with_text(text),
            editing,
            check: Check::Invalid(String::new()),
            error: None,
        };
        editor.recheck(styles, schemes);
        editor
    }

    /// Re-checks the text after an edit.
    pub fn recheck(&mut self, styles: &StyleCatalog, schemes: &SchemeCatalog) {
        let text = self.content.text();
        let editing = self.editing.as_deref();
        self.check = match self.kind {
            CodeKind::Style => match styles.check(&text, editing) {
                Ok((style, notes)) => Check::Style(Box::new(style), notes),
                Err(why) => Check::Invalid(why),
            },
            CodeKind::Palette => match schemes.check(&text, editing) {
                Ok(scheme) => Check::Palette(Box::new(scheme)),
                Err(why) => Check::Invalid(why),
            },
        };
        self.error = None;
    }

    /// Whether Save can write the text.
    pub fn valid(&self) -> bool {
        !matches!(self.check, Check::Invalid(_))
    }
}

/// A config word for an enum value (`"top-sheet"`), as serde writes it.
fn word<T: Serialize>(value: T) -> String {
    match toml::Value::try_from(value) {
        Ok(toml::Value::String(s)) => s,
        Ok(other) => other.to_string(),
        Err(_) => String::new(),
    }
}

/// `base` unless `taken`, else `base 2`, `base 3`…
fn unused_name(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_owned();
    }
    (2..1000)
        .map(|n| format!("{base} {n}"))
        .find(|name| !taken(name))
        .unwrap_or_else(|| base.to_owned())
}

/// A new custom style extending `style`, listing its most structural keys with their values
/// and the accepted words, so it can be changed by editing.
pub fn style_template(style: &StyleDef, styles: &StyleCatalog) -> String {
    let base = if StyleCatalog::is_builtin(style) {
        style.key.clone()
    } else {
        "fogu".to_owned()
    };
    let name = unused_name(&format!("My {}", style.name), |n| {
        styles.resolve(n).is_some()
    });
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# Custom UI style: edit it, then Save. It is written to styles/ and applied.\n\
         # Keys you delete are inherited from `extends` (fogu, fluent, minimal, retro,\n\
         # synthwave or glass). README.md lists every section and key.\n\
         name = {name:?}\n\
         extends = {base:?}"
    );
    if let Some(scheme) = &style.paired_scheme {
        let _ = writeln!(out, "paired_scheme = {scheme:?}");
    }
    let accent = style.color.accent;
    let (bar, tabs, layout, shape) = (style.bar, style.tabs, style.layout, style.shape);
    let (text, interaction, dialog) = (style.text, style.interaction, style.dialog);
    let _ = write!(
        out,
        "
[color]
accent = \"{accent}\"

[bar]
placement = \"{}\"        # top, bottom
height = {}
fill = \"{}\"             # solid, base, vertical, horizontal
gradient = {}

[tabs]
style = \"{}\"            # pill, folder, underline, inverse, glow
sizing = \"{}\"           # fixed, fill, fit
marker = \"{}\"           # dot, index, none
close = \"{}\"            # always, hover, never
indicator = {}

[layout]
geometry = \"{}\"         # docked, islands
margin = {}
sidebar = \"{}\"          # docked, floating
density = \"{}\"          # compact, regular, comfy

[shape]
card = {}
control = {}
tab_top = {}
tab_bottom = {}

[text]
chrome = \"{}\"           # ui, mono
labels = \"{}\"           # upper, sentence

[interaction]
hover = \"{}\"            # fill, inverse, outline
button = \"{}\"           # filled, outline, text, inverse
selection = \"{}\"        # fill, inverse

[dialog]
placement = \"{}\"        # center, top-sheet, command-bar
footer = {}
",
        word(bar.placement),
        bar.height,
        word(bar.fill),
        bar.gradient,
        word(tabs.style),
        word(tabs.sizing),
        word(tabs.marker),
        word(tabs.close),
        tabs.indicator,
        word(layout.geometry),
        layout.margin,
        word(layout.sidebar),
        word(layout.density),
        shape.card,
        shape.control,
        shape.tab_top,
        shape.tab_bottom,
        word(text.chrome),
        word(text.labels),
        word(interaction.hover),
        word(interaction.button),
        word(interaction.selection),
        word(dialog.placement),
        dialog.footer,
    );
    out
}

/// A new custom palette: every colour of `scheme`, ready to change.
pub fn palette_template(scheme: &ColorScheme, schemes: &SchemeCatalog) -> String {
    let name = unused_name(&format!("My {}", scheme.name), |n| {
        schemes.by_name(n).is_some()
    });
    let mut out = format!(
        "# Custom terminal palette: edit it, then Save. It is written to themes/ and applied.\n\
         # Colours are \"#rrggbb\"; ansi lists black, red, green, yellow, blue, magenta, cyan,\n\
         # white, then their bright variants.\n\
         name = {name:?}\n\
         fg = \"{}\"\n\
         bg = \"{}\"\n\
         cursor = \"{}\"\n\
         cursor_text = \"{}\"\n\
         selection_bg = \"{}\"\n\
         selection_fg = \"{}\"\n\
         ansi = [\n",
        scheme.fg,
        scheme.bg,
        scheme.cursor,
        scheme.cursor_text,
        scheme.selection_bg,
        scheme.selection_fg,
    );
    for chunk in scheme.ansi.chunks(4) {
        let line: Vec<String> = chunk.iter().map(|c| format!("\"{c}\"")).collect();
        let _ = writeln!(out, "    {},", line.join(", "));
    }
    out.push_str("]\n");
    out
}

fn swatch<'a>(color: RgbColor, look: Look) -> Element<'a, Message> {
    let radius = look.shape.dot.min(3.0);
    container(space().width(16).height(16))
        .style(move |_theme: &iced::Theme| container::Style {
            background: Some(iced::Color::from_rgb8(color.r, color.g, color.b).into()),
            border: Border {
                color: look.border,
                width: 1.0,
                radius: radius.into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// The editor panel: code on the left, live preview and the check's verdict on the right.
#[expect(
    clippy::too_many_lines,
    reason = "one panel: header, editor, preview and verdict laid out together"
)]
pub fn view<'a>(
    editor: &'a CodeEditor,
    look: Look,
    (current, scheme): (&StyleDef, &ColorScheme),
    schemes: &SchemeCatalog,
    env: Env,
) -> Element<'a, Message> {
    let msg = |m: SettingsMsg| Message::Settings(m);
    let what = match editor.kind {
        CodeKind::Style => "UI style",
        CodeKind::Palette => "terminal palette",
    };
    let title = match &editor.editing {
        Some(path) => format!(
            "Edit {}",
            path.file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
        ),
        None => format!("New {what} from code"),
    };
    let actions = row![
        controls::action(
            "Cancel",
            ButtonKind::Secondary,
            look,
            Some(msg(SettingsMsg::CloseCode))
        ),
        controls::action(
            "Save and apply",
            ButtonKind::Primary,
            look,
            editor.valid().then(|| msg(SettingsMsg::SaveCode))
        ),
    ]
    .spacing(8);
    let header = row![
        column![
            text(title).size(15).font(look.text.strong).color(look.fg),
            text(format!(
                "Edit the TOML; it is checked and previewed as you type. Save writes it to \
                 the {} folder and applies it.",
                match editor.kind {
                    CodeKind::Style => "styles",
                    CodeKind::Palette => "themes",
                }
            ))
            .size(12)
            .font(look.text.chrome)
            .color(look.dim),
        ]
        .spacing(3)
        .width(Length::Fill),
        actions,
    ]
    .spacing(12)
    .align_y(Alignment::Center);

    let code = text_editor(&editor.content)
        .on_action(|action| Message::Settings(SettingsMsg::Code(action)))
        .font(look.text.mono)
        .size(12.5)
        .padding(10)
        .height(Length::Fill)
        .style(move |_theme: &iced::Theme, status| {
            let focused = matches!(status, text_editor::Status::Focused { .. });
            text_editor::Style {
                background: look.base.into(),
                border: Border {
                    color: if focused { look.focus } else { look.border },
                    width: if focused {
                        look.shape.focus.max(1.0)
                    } else {
                        1.0
                    },
                    radius: look.shape.control.into(),
                },
                placeholder: look.dim,
                value: look.fg,
                selection: look.accent_soft,
            }
        });

    let (verdict_color, verdict): (iced::Color, String) = match &editor.check {
        Check::Invalid(why) if why.is_empty() => (look.dim, String::new()),
        Check::Invalid(why) => (look.danger, why.clone()),
        Check::Style(style, notes) if notes.is_empty() => {
            (look.accent_text, format!("Valid: \"{}\"", style.name))
        }
        Check::Style(style, notes) => (
            look.accent_text,
            format!("Valid: \"{}\"\n{}", style.name, notes.join("\n")),
        ),
        Check::Palette(scheme) => (look.accent_text, format!("Valid: \"{}\"", scheme.name)),
    };
    let preview: Element<'a, Message> = match &editor.check {
        Check::Style(style, _) => {
            let paired = style
                .paired_scheme
                .as_deref()
                .and_then(|name| schemes.by_name(name))
                .unwrap_or(scheme);
            style_preview::preview(style, paired, None, env)
        }
        Check::Palette(palette) => {
            let swatches = palette
                .ansi
                .chunks(8)
                .fold(column![].spacing(4), |col, chunk| {
                    col.push(
                        chunk
                            .iter()
                            .fold(row![].spacing(4), |r, c| r.push(swatch(*c, look))),
                    )
                });
            column![
                style_preview::preview(current, palette, None, env),
                swatches
            ]
            .spacing(10)
            .into()
        }
        Check::Invalid(_) => container(
            text("The preview shows once the file is valid.")
                .size(12)
                .font(look.text.chrome)
                .color(look.dim),
        )
        .height(style_preview::PREVIEW_H)
        .center_y(style_preview::PREVIEW_H)
        .into(),
    };
    let mut side = column![
        controls::section_label(look, "Preview"),
        container(preview).style(move |_theme: &iced::Theme| container::Style {
            border: Border {
                color: look.border,
                width: 1.0,
                radius: look.shape.control.into(),
            },
            ..container::Style::default()
        }),
        controls::section_label(look, "Check"),
        text(verdict)
            .size(12)
            .font(look.text.chrome)
            .color(verdict_color),
    ]
    .spacing(8);
    if let Some(error) = &editor.error {
        side = side.push(
            text(format!("Not saved: {error}"))
                .size(12)
                .font(look.text.chrome)
                .color(look.danger),
        );
    }
    let side = scrollable(side).width(Length::FillPortion(2));
    column![
        header,
        row![container(code).width(Length::FillPortion(3)), side]
            .spacing(16)
            .height(Length::Fill),
    ]
    .spacing(14)
    .height(Length::Fill)
    .into()
}

#[cfg(test)]
#[expect(clippy::panic, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn templates_are_valid_files_with_free_names() {
        let styles = StyleCatalog::builtins();
        let schemes = SchemeCatalog::builtins();
        for style in styles.styles() {
            let text = style_template(style, &styles);
            let (parsed, notes) = styles
                .check(&text, None)
                .unwrap_or_else(|why| panic!("{}: {why}\n{text}", style.key));
            assert!(notes.is_empty(), "{notes:?}");
            assert_eq!(parsed.tabs, style.tabs, "{}", style.key);
            assert_eq!(parsed.bar, style.bar, "{}", style.key);
            assert_eq!(parsed.layout, style.layout, "{}", style.key);
        }
        for scheme in schemes.schemes() {
            let text = palette_template(scheme, &schemes);
            let parsed = schemes
                .check(&text, None)
                .unwrap_or_else(|why| panic!("{}: {why}\n{text}", scheme.name));
            assert_eq!(parsed.ansi, scheme.ansi);
            assert_eq!(parsed.bg, scheme.bg);
        }
    }

    #[test]
    fn checking_reports_why_a_text_cannot_be_saved() {
        let styles = StyleCatalog::builtins();
        let schemes = SchemeCatalog::builtins();
        let mut editor = CodeEditor::new(
            CodeKind::Style,
            "name = \"Glass\"\n",
            None,
            (&styles, &schemes),
        );
        assert!(!editor.valid());
        assert!(matches!(&editor.check, Check::Invalid(why) if why.contains("reserved")));
        editor.content = text_editor::Content::with_text("name = \"Mine\"\n[tabs]\nstyel = 1\n");
        editor.recheck(&styles, &schemes);
        assert!(matches!(&editor.check, Check::Invalid(why) if why.contains("styel")));
        editor.content = text_editor::Content::with_text("name = \"Mine\"\nextends = \"glass\"\n");
        editor.recheck(&styles, &schemes);
        assert!(editor.valid());
    }
}
