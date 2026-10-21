//! The settings page: a nav of sections (Appearance, Terminal, …), a search over every setting,
//! and live controls that apply at once and are saved to `config.toml`.

mod appearance;
mod handlers;
mod index;
mod render;
mod reveal;
mod shortcuts;
mod spec;
mod terminal;

use std::collections::BTreeMap;

use fogu_core::{Config, RgbColor, SchemeCatalog, StyleCatalog};
use iced::widget::{
    Id, column, container, mouse_area, row, scrollable, space, stack, text, text_editor,
};
use iced::{Alignment, Border, Element, Length};

use crate::controls;
use crate::design::{Env, resolve_style};
use crate::icons::{Icon, fixed, icon};
use crate::message::Message;
use crate::palette::Look;
use crate::style_code::{self, CodeEditor, CodeKind};

pub use shortcuts::ShortcutMsg;
pub use spec::SettingId;

/// The page's scrollable (for revealing a row).
const SCROLL_ID: &str = "fogu-settings-scroll";
/// The search field.
pub const SEARCH_ID: &str = "fogu-settings-search";
/// How long a row found by the search stays highlighted.
const HIGHLIGHT: std::time::Duration = std::time::Duration::from_millis(1500);

/// A page of the settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Section {
    #[default]
    Appearance,
    Terminal,
    Profiles,
    Shortcuts,
    Keys,
    Sftp,
    Advanced,
}

impl Section {
    /// Every page, in nav order.
    pub const ALL: [Self; 7] = [
        Self::Appearance,
        Self::Terminal,
        Self::Profiles,
        Self::Shortcuts,
        Self::Keys,
        Self::Sftp,
        Self::Advanced,
    ];

    /// Its nav label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::Terminal => "Terminal",
            Self::Profiles => "Profiles",
            Self::Shortcuts => "Shortcuts",
            Self::Keys => "Keys",
            Self::Sftp => "SFTP",
            Self::Advanced => "Advanced",
        }
    }
}

/// A block of a page that is not a plain setting row (search target).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Anchor {
    UiStyle,
    Accent,
    Palette,
    Density,
}

impl Anchor {
    /// Every anchor.
    pub const ALL: [Self; 4] = [Self::UiStyle, Self::Accent, Self::Palette, Self::Density];

    /// Its page.
    pub fn section(self) -> Section {
        match self {
            Self::UiStyle | Self::Accent | Self::Palette | Self::Density => Section::Appearance,
        }
    }

    /// Its search title.
    pub fn title(self) -> &'static str {
        match self {
            Self::UiStyle => "UI style",
            Self::Accent => "Accent colour",
            Self::Palette => "Terminal palette",
            Self::Density => "Density",
        }
    }

    /// Extra words its search matches.
    pub fn keywords(self) -> &'static str {
        match self {
            Self::UiStyle => "theme look window structure tabs islands custom code",
            Self::Accent => "color highlight selection focus",
            Self::Palette => "color scheme colours ansi theme custom code",
            Self::Density => "rows compact comfortable spacing",
        }
    }

    /// The id of its block.
    pub fn id(self) -> Id {
        Id::new(match self {
            Self::UiStyle => "anchor:ui-style",
            Self::Accent => "anchor:accent",
            Self::Palette => "anchor:palette",
            Self::Density => "anchor:density",
        })
    }
}

/// Something the search jumps to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Target {
    /// A setting row.
    Setting(SettingId),
    /// A page block.
    Anchor(Anchor),
    /// An action's shortcuts (the Shortcuts page, filtered to it).
    Action(fogu_core::Action),
}

impl Target {
    /// The page it is on.
    pub fn section(self) -> Section {
        match self {
            Self::Setting(id) => id.spec().section,
            Self::Anchor(anchor) => anchor.section(),
            Self::Action(_) => Section::Shortcuts,
        }
    }

    fn id(self) -> Id {
        match self {
            Self::Setting(id) => id.widget_id(),
            Self::Anchor(anchor) => anchor.id(),
            Self::Action(_) => Id::new(shortcuts::ANCHOR),
        }
    }
}

/// An edit on a setting's control.
#[derive(Clone, Debug)]
pub enum Edit {
    /// A switch.
    Bool(bool),
    /// A slider moved (applied, saved on release).
    Slide(f64),
    /// The slider was released: save.
    SlideDone,
    /// An optional slider's "automatic" switch.
    Auto(bool),
    /// A text or number field changed (kept until Enter or a click elsewhere).
    Text(String),
    /// Enter in a text or number field.
    Submit,
    /// A choice.
    Choice(usize),
    /// A flag of a flag set was clicked (toggles it).
    Flag(usize),
    /// A font family picked from the list.
    Pick(String),
    /// A font family added to a list.
    Add(String),
    /// Item `.0` of a list removed.
    Remove(usize),
}

/// A question the page asks before going on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confirm {
    /// The code editor has unsaved changes and the page is being closed.
    DiscardCode,
    /// Every shortcut back to its default.
    ResetShortcuts,
}

/// A choice in the settings page.
#[derive(Clone, Debug)]
pub enum SettingsMsg {
    /// Show another page.
    Section(Section),
    /// The search text changed.
    Query(String),
    /// Enter in the search field: go to the highlighted result.
    SearchSubmit,
    /// Go to a search result.
    Jump(Target),
    /// Scroll the jumped-to row into view (once its page is laid out).
    Reveal(Target),
    /// The highlight of generation `.0` is over.
    Unhighlight(u64),
    /// A setting's control changed.
    Set(SettingId, Edit),
    /// A press on the dimmed window around the page.
    Outside,
    /// The answer to the page's question.
    Confirm(bool),
    /// A choice on the Shortcuts page.
    Shortcut(ShortcutMsg),
    /// Switch the UI style by key (applies immediately and is saved); the terminal palette
    /// follows when it was the default or another style's paired palette.
    Style(String),
    /// Set or clear the accent override.
    Accent(Option<RgbColor>),
    /// Switch the terminal colour scheme by exact name (applies and is saved).
    Scheme(String),
    /// Switch to the active style's paired palette.
    UseMatchingPalette,
    /// Re-read `styles/` and `themes/`, then apply the configured style and scheme against the
    /// new catalogs.
    Reload,
    /// Open the code editor on a new custom style or palette, based on the active one.
    NewCode(CodeKind),
    /// Open the code editor on the file of custom style (key) or palette (name).
    EditCode(CodeKind, String),
    /// An action in the code editor.
    Code(text_editor::Action),
    /// Write the edited file, reload the catalogs and apply it.
    SaveCode,
    /// Close the code editor without saving.
    CloseCode,
}

/// The open settings page.
#[derive(Debug, Default)]
pub struct SettingsPage {
    /// The page shown.
    pub section: Section,
    /// The search text.
    pub query: String,
    /// The highlighted search result.
    pub selected: usize,
    /// Text typed into a field, not saved yet (Enter or a click elsewhere saves it).
    pub pending: BTreeMap<SettingId, String>,
    /// Why a field's text cannot be saved.
    pub errors: BTreeMap<SettingId, String>,
    /// The row or block the search jumped to.
    pub highlight: Option<Target>,
    /// Bumped per jump, so an older highlight's timer does not end a newer one.
    pub highlight_gen: u64,
    /// The question asked over the page.
    pub confirm: Option<Confirm>,
    /// The custom style / palette code editor, when open.
    pub editor: Option<CodeEditor>,
    /// The editor's text when it opened (unsaved changes = it differs).
    pub editor_original: String,
    /// The font family pickers (installed monospace families), filled when the page opens.
    pub fonts: Option<FontPickers>,
    /// The Shortcuts page.
    pub shortcuts: shortcuts::ShortcutsState,
}

/// The searchable font lists of the Terminal page.
#[derive(Debug)]
pub struct FontPickers {
    /// The terminal font family.
    pub family: iced::widget::combo_box::State<String>,
    /// "Add a fallback font".
    pub fallback: iced::widget::combo_box::State<String>,
}

impl FontPickers {
    /// Pickers over `families`.
    pub fn new(families: Vec<String>) -> Self {
        Self {
            fallback: iced::widget::combo_box::State::new(families.clone()),
            family: iced::widget::combo_box::State::new(families),
        }
    }
}

impl SettingsPage {
    /// Whether the code editor holds unsaved changes.
    pub fn editor_dirty(&self) -> bool {
        self.editor
            .as_ref()
            .is_some_and(|e| e.content.text() != self.editor_original)
    }
}

/// The page body, placed inside a floating card by [`crate::view`].
pub fn view<'a>(
    page: &'a SettingsPage,
    look: Look,
    config: &Config,
    catalogs: (&SchemeCatalog, &StyleCatalog),
    env: Env,
) -> Element<'a, Message> {
    let header = header(page, look);
    let nav = Section::ALL
        .iter()
        .fold(column![].spacing(2), |nav, &section| {
            nav.push(nav_item(section, page.section == section, look))
        })
        .width(170);
    let content: Element<'a, Message> = if let Some(editor) = &page.editor {
        let (schemes, styles) = catalogs;
        let (current, _) = resolve_style(&config.appearance.ui_theme, styles);
        let scheme = schemes
            .by_name(&config.appearance.color_scheme)
            .unwrap_or_else(|| appearance::default_scheme());
        container(style_code::view(
            editor,
            look,
            (current, scheme),
            schemes,
            env,
        ))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    } else {
        let body = section_body(page, look, config, catalogs, env);
        scrollable(container(body).padding(iced::Padding::ZERO.right(14)))
            .id(Id::new(SCROLL_ID))
            .height(Length::Fill)
            .width(Length::Fill)
            .into()
    };
    let results = results(page, look);
    let content = stack![content, results];
    let body = row![nav, rule(look), content]
        .spacing(18)
        .height(Length::Fill);
    container(
        column![header, controls::hairline(look.border), body]
            .spacing(16)
            .padding(20),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

/// Title, subtitle, search field and close button.
fn header(page: &SettingsPage, look: Look) -> Element<'_, Message> {
    let close = iced::widget::button(icon(Icon::Close, 12.0, look.dim))
        .width(28)
        .height(28)
        .padding(0)
        .style(move |_theme: &iced::Theme, status| controls::flat_button(look, status, false))
        .on_press(Message::CloseSettings);
    let search = row![
        fixed(Icon::Search, 12.0, look.dim),
        iced::widget::text_input("Search settings", &page.query)
            .id(Id::new(SEARCH_ID))
            .on_input(|q| Message::Settings(SettingsMsg::Query(q)))
            .on_submit(Message::Settings(SettingsMsg::SearchSubmit))
            .size(13)
            .font(look.text.chrome)
            .padding([5, 8])
            .style(move |_theme: &iced::Theme, status| controls::field(look, status))
            .width(260),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    row![
        column![
            crate::modal::title("Settings", look),
            text("Changes apply live and are saved automatically.")
                .size(12)
                .font(look.text.chrome)
                .color(look.dim),
        ]
        .spacing(3)
        .width(Length::Fill),
        search,
        close,
    ]
    .spacing(14)
    .align_y(Alignment::Center)
    .into()
}

/// One nav entry.
fn nav_item<'a>(section: Section, active: bool, look: Look) -> Element<'a, Message> {
    let (bg, fg) = controls::row_colors(look, active, false);
    let item = container(
        text(section.label())
            .size(13)
            .font(look.text.chrome)
            .color(if active { fg } else { look.dim }),
    )
    .width(Length::Fill)
    .padding([8, 10])
    .style(move |_theme: &iced::Theme| container::Style {
        background: bg.map(Into::into),
        border: Border::default().rounded(look.shape.control),
        ..container::Style::default()
    });
    mouse_area(item)
        .interaction(iced::mouse::Interaction::Pointer)
        .on_press(Message::Settings(SettingsMsg::Section(section)))
        .into()
}

/// The page shown.
fn section_body<'a>(
    page: &'a SettingsPage,
    look: Look,
    config: &Config,
    catalogs: (&SchemeCatalog, &StyleCatalog),
    env: Env,
) -> Element<'a, Message> {
    let anchor = match page.highlight {
        Some(Target::Anchor(anchor)) => Some(anchor),
        _ => None,
    };
    match page.section {
        Section::Appearance => appearance::body(look, config, catalogs, env, anchor),
        Section::Shortcuts => shortcuts::body(&page.shortcuts, &config.hotkeys, look),
        Section::Terminal => render::rows(
            page,
            look,
            config,
            terminal::TermKey::ALL.iter().copied().map(SettingId::Term),
        ),
        section => placeholder(look, section),
    }
}

/// A page that is not built yet.
fn placeholder<'a>(look: Look, section: Section) -> Element<'a, Message> {
    render::small(look, &format!("{} settings are coming.", section.label()))
}

/// The search results, floating under the search field.
fn results<'a>(page: &SettingsPage, look: Look) -> Element<'a, Message> {
    let hits = index::search(&page.query);
    if page.query.trim().is_empty() {
        return space().into();
    }
    let mut list = column![].spacing(2);
    if hits.is_empty() {
        list = list.push(render::small(look, "No setting matches."));
    }
    for (i, hit) in hits.into_iter().enumerate() {
        let (bg, fg) = controls::row_colors(look, i == page.selected, false);
        let entry = container(
            column![
                text(hit.title).size(13).font(look.text.chrome).color(fg),
                text(hit.place)
                    .size(11)
                    .font(look.text.chrome)
                    .color(look.dim),
            ]
            .spacing(1),
        )
        .width(Length::Fill)
        .padding([6, 10])
        .style(move |_theme: &iced::Theme| container::Style {
            background: bg.map(Into::into),
            border: Border::default().rounded(look.shape.control),
            ..container::Style::default()
        });
        list = list.push(
            mouse_area(entry)
                .interaction(iced::mouse::Interaction::Pointer)
                .on_press(Message::Settings(SettingsMsg::Jump(hit.target))),
        );
    }
    let panel = container(list)
        .padding(6)
        .width(360)
        .style(move |_theme: &iced::Theme| controls::floating(look));
    container(iced::widget::opaque(panel))
        .width(Length::Fill)
        .align_x(Alignment::End)
        .into()
}

/// `content` in a container with `id` (so the search can scroll to it), outlined in the accent
/// while `highlighted`.
pub(crate) fn anchored(
    id: Id,
    highlighted: bool,
    content: Element<'_, Message>,
    look: Look,
) -> Element<'_, Message> {
    container(content)
        .id(id)
        .width(Length::Fill)
        .padding([6, 8])
        .style(move |_theme: &iced::Theme| container::Style {
            background: highlighted.then(|| {
                iced::Color {
                    a: 0.10,
                    ..look.accent
                }
                .into()
            }),
            border: Border {
                color: if highlighted {
                    look.accent
                } else {
                    iced::Color::TRANSPARENT
                },
                width: 1.0,
                radius: look.shape.control.into(),
            },
            ..container::Style::default()
        })
        .into()
}

fn rule(look: Look) -> Element<'static, Message> {
    container(space())
        .width(1)
        .height(Length::Fill)
        .style(move |_theme: &iced::Theme| container::background(look.border))
        .into()
}
