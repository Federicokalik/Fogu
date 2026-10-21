//! The tab bar: a custom title bar with tabs, quick actions and window controls, shaped by the
//! UI style.
//!
//! Styles change its structure, not only its colours: the tabs are pills, folder tabs merging
//! into the terminal, underlined labels, inverse-video cells or glowing chips; they size to a
//! fixed width, share the strip or fit their titles; and they sit in the title bar or, for a
//! status-line style, under the panes with a thin strip above keeping the window controls.
//!
//! Tabs scroll horizontally (vertical wheel included), keep a per-tab colour marker, and can be
//! renamed inline. The bar itself moves the window; the resize edges are layered above.

use std::f32::consts::{FRAC_PI_2, PI};

use fogu_core::Action;
use fogu_core::style::{
    BarFill, BarPlacement, Geometry, IndicatorColor, TabClose, TabMarker, TabStyle,
};
use iced::gradient::Linear;
use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{
    Id, Row, Space, button, column, container, mouse_area, row, scrollable, space, stack, text,
    text_input, tooltip,
};
use iced::{Alignment, Background, Border, Color, Element, Length, Point, Radians, Size};

use crate::context_menu::to_iced;
use crate::controls;
use crate::drag::{DragMsg, Payload};
use crate::icons::{Icon, icon};
use crate::lifecycle::{warning_detail, warning_title};
use crate::message::{Message, WindowCmd};
use crate::palette::{Look, mix};
use crate::tab::{Tab, TabId};
use crate::tab_layout::{GAP, ISLAND_PAD};

/// macOS: room at the bar's left end for the native window buttons (the title bar is hidden).
pub const TRAFFIC_LIGHTS_W: f32 = 80.0;
/// Width of `+` and `⌄` after the headers.
pub(crate) const ADD_W: f32 = 28.0 + 24.0 + 4.0;
/// Width of one panel toggle (ports, SFTP, profiles, settings).
pub(crate) const TOGGLE_W: f32 = 32.0;
/// Width of the separator and the three window buttons (not on macOS).
pub(crate) const WINDOW_BUTTONS_W: f32 = 6.0 + 1.0 + 2.0 + 3.0 * 44.0;
/// Titles longer than this many characters end in `…`.
const MAX_TITLE_CHARS: usize = 20;
/// Width of the window's resize edges.
const EDGE: f32 = 5.0;
/// Warning badge colour (amber).
const WARN: Color = Color::from_rgb(0.96, 0.65, 0.14);
/// Id of the inline rename field.
pub const RENAME_INPUT: &str = "fogu-rename-input";
/// Id of the scrollable tab strip.
pub const TAB_STRIP: &str = "fogu-tab-strip";

/// An inline rename in progress.
#[derive(Clone, Debug)]
pub struct Rename {
    pub tab: TabId,
    pub value: String,
}

/// Everything the bar needs for one `view`.
#[derive(Clone, Debug)]
pub struct TabBar<'a> {
    pub tabs: &'a [Tab],
    pub active: TabId,
    pub hovered: Option<TabId>,
    pub renaming: Option<&'a Rename>,
    pub look: Look,
    /// The active tab has an SSH pane: show the SFTP button (`true` = panel open).
    pub sftp: Option<bool>,
    /// The active tab has an SSH pane: show the "Ports" button with this many listening
    /// forwards.
    pub forwards: Option<usize>,
    /// The profile sidebar is open (its toggle button is highlighted).
    pub profiles: bool,
    /// Empty room before the headers ([`TRAFFIC_LIGHTS_W`] on macOS).
    pub inset: f32,
    /// Each header's width (`App::tab_widths`).
    pub widths: Vec<f32>,
    /// The window size (where a bottom bar sits).
    pub window: Size,
    /// Gap around islands (0 when docked).
    pub margin: f32,
}

/// The bar's parts: the title bar on top and, for status-line styles, the tab strip under the
/// panes.
pub struct BarParts<'a> {
    /// Above the panes.
    pub top: Element<'a, Message>,
    /// Under the panes (bottom placement only).
    pub bottom: Option<Element<'a, Message>>,
}

/// `title` cut to [`MAX_TITLE_CHARS`] characters, ending in `…` when cut.
pub fn truncate(title: &str) -> String {
    if title.chars().count() <= MAX_TITLE_CHARS {
        return title.to_owned();
    }
    let mut out: String = title.chars().take(MAX_TITLE_CHARS - 1).collect();
    out.push('…');
    out
}

fn solid<'a>(color: Color, width: Length, height: impl Into<Length>) -> Element<'a, Message> {
    container(space())
        .width(width)
        .height(height)
        .style(move |_theme: &iced::Theme| container::background(color))
        .into()
}

/// How one header is drawn, from its state.
struct HeaderLook {
    /// Fill of the header body.
    surface: Option<Color>,
    /// Border of the header body.
    border: Color,
    /// Title colour.
    title: Color,
    /// Marker colour (dot or index).
    marker: Color,
    /// Gap between the header body and the bar edges: `[top, side, bottom]`.
    pad: [f32; 3],
    /// Shadow under the header body.
    shadow: iced::Shadow,
}

impl<'a> TabBar<'a> {
    /// The colour the bar is (roughly) filled with, for mixing header colours.
    fn bar_color(&self) -> Color {
        match self.look.bar.fill {
            BarFill::Base => self.look.base,
            BarFill::Solid | BarFill::Vertical | BarFill::Horizontal => self.look.raised,
        }
    }

    fn header_look(&self, active: bool, hovered: bool, color: Option<Color>) -> HeaderLook {
        let look = self.look;
        let spec = look.tabs;
        let bar = self.bar_color();
        let inactive_title = mix(look.fg, bar, 0.38);
        let marker = color.unwrap_or(if active {
            look.accent
        } else {
            mix(look.fg, bar, 0.7)
        });
        let none = iced::Shadow::default();
        let inset = spec.inset;
        match spec.style {
            TabStyle::Pill => HeaderLook {
                surface: if active {
                    Some(mix(bar, look.fg, 0.09))
                } else {
                    hovered.then(|| mix(bar, look.fg, 0.05))
                },
                border: if active {
                    mix(bar, look.accent, 0.55)
                } else {
                    Color::TRANSPARENT
                },
                title: if active { look.fg } else { inactive_title },
                marker,
                pad: [inset, inset / 2.0, inset],
                shadow: if active { look.shadow.tab } else { none },
            },
            TabStyle::Folder => HeaderLook {
                surface: if active {
                    Some(look.base)
                } else {
                    hovered.then(|| mix(bar, look.fg, 0.06))
                },
                border: Color::TRANSPARENT,
                title: if active { look.fg } else { inactive_title },
                marker,
                pad: [inset, 0.0, 0.0],
                shadow: if active { look.shadow.tab } else { none },
            },
            TabStyle::Underline => HeaderLook {
                surface: None,
                border: Color::TRANSPARENT,
                title: if active || hovered {
                    look.fg
                } else {
                    inactive_title
                },
                marker,
                pad: [0.0, 0.0, 0.0],
                shadow: none,
            },
            TabStyle::Inverse => HeaderLook {
                surface: if active {
                    Some(look.accent)
                } else {
                    hovered.then_some(Color {
                        a: 0.18,
                        ..look.accent
                    })
                },
                border: Color::TRANSPARENT,
                title: if active {
                    look.on_accent
                } else {
                    inactive_title
                },
                marker: if active && color.is_none() {
                    look.on_accent
                } else {
                    marker
                },
                pad: [inset, 0.0, inset],
                shadow: if active { look.shadow.tab } else { none },
            },
            TabStyle::Glow => HeaderLook {
                surface: if active {
                    Some(Color {
                        a: 0.12,
                        ..look.accent
                    })
                } else {
                    hovered.then_some(Color { a: 0.05, ..look.fg })
                },
                border: Color::TRANSPARENT,
                title: if active { look.fg } else { inactive_title },
                marker,
                pad: [inset, inset / 2.0, inset],
                shadow: none,
            },
        }
    }

    /// The active tab's indicator line, laid over the header body (top for folder tabs, bottom
    /// otherwise).
    fn indicator(&self, width: f32) -> Option<Element<'a, Message>> {
        let look = self.look;
        let spec = look.tabs;
        if spec.indicator <= 0.0 {
            return None;
        }
        let color = match spec.indicator_color {
            IndicatorColor::Accent => look.accent,
            IndicatorColor::Fg => look.fg,
        };
        let length = if spec.indicator_length > 0.0 {
            spec.indicator_length.min(width)
        } else {
            (width - 2.0 * look.layout.gutter).max(8.0)
        };
        let glow = spec.style == TabStyle::Glow;
        let shadow = if glow {
            look.shadow.tab
        } else {
            iced::Shadow::default()
        };
        let thickness = spec.indicator;
        let line = container(space().width(length).height(thickness)).style(move |_theme| {
            container::Style {
                background: Some(color.into()),
                border: Border::default().rounded(thickness / 2.0),
                shadow,
                ..container::Style::default()
            }
        });
        let top = spec.style == TabStyle::Folder;
        Some(
            container(line)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(Alignment::Center)
                .align_y(if top {
                    Alignment::Start
                } else {
                    Alignment::End
                })
                .padding(if top { [0, 0] } else { [2, 0] })
                .into(),
        )
    }

    fn marker(&self, index: usize, color: Color, explicit: bool) -> Option<Element<'a, Message>> {
        let look = self.look;
        match look.tabs.marker {
            TabMarker::Dot => Some(controls::dot(look, color, 6.0)),
            TabMarker::Index => Some(
                text(format!("{}", index + 1))
                    .size(look.tabs.title_size - 1.0)
                    .font(look.text.strong)
                    .color(color)
                    .into(),
            ),
            // No marker, except a colour the user gave the tab.
            TabMarker::None if explicit => Some(controls::dot(look, color, 6.0)),
            TabMarker::None => None,
        }
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one header: marker, title or rename field, close, indicator and its frame"
    )]
    fn header(&self, index: usize, tab: &'a Tab) -> Element<'a, Message> {
        let look = self.look;
        let spec = look.tabs;
        let width = self.widths.get(index).copied().unwrap_or(spec.width);
        let active = tab.id == self.active;
        let hovered = self.hovered == Some(tab.id);
        let explicit = tab.color.map(to_iced);
        let hl = self.header_look(active, hovered, explicit);
        let title: Element<'a, Message> = match self.renaming {
            Some(rename) if rename.tab == tab.id => text_input("Tab title", &rename.value)
                .id(Id::new(RENAME_INPUT))
                .on_input(Message::RenameInput)
                .on_submit(Message::CommitRename)
                .size(spec.title_size)
                .font(look.text.chrome)
                .padding([2, 4])
                .style(move |_theme, status| controls::field(look, status))
                .into(),
            _ => text(truncate(tab.title()))
                .size(spec.title_size)
                .font(if active {
                    look.text.active
                } else {
                    look.text.chrome
                })
                .color(hl.title)
                .wrapping(text::Wrapping::None)
                .width(Length::Fill)
                .into(),
        };
        let show_close = match spec.close {
            TabClose::Always => true,
            TabClose::Hover => active || hovered,
            TabClose::Never => false,
        };
        let close: Option<Element<'a, Message>> = if show_close {
            let glyph = if hovered { hl.title } else { look.dim };
            let glyph = if active && spec.style == TabStyle::Inverse {
                look.on_accent
            } else {
                glyph
            };
            Some(
                button(icon(Icon::Close, 9.0, glyph))
                    .width(20)
                    .height(20)
                    .padding(0)
                    .style(move |_theme: &iced::Theme, status| {
                        let mut style = controls::flat_button(look, status, false);
                        style.border = style.border.rounded(look.shape.control.min(10.0));
                        style
                    })
                    .on_press(Message::CloseTab(tab.id))
                    .into(),
            )
        } else if spec.close == TabClose::Hover {
            Some(space().width(20).into())
        } else {
            None
        };
        let mut line = row![].spacing(8).align_y(Alignment::Center);
        if let Some(marker) = self.marker(index, hl.marker, explicit.is_some()) {
            line = line.push(marker);
        }
        if let Some(badge) = warning_badge(tab, look) {
            line = line.push(badge);
        }
        line = line.push(title);
        if let Some(close) = close {
            line = line.push(close);
        }
        let surface = hl.surface;
        let border = hl.border;
        let shadow = hl.shadow;
        let content = container(line.clip(true))
            .padding([0.0, look.layout.gutter])
            .height(Length::Fill)
            .width(Length::Fill)
            .align_y(Alignment::Center);
        let body: Element<'a, Message> = match self.indicator(width).filter(|_| active) {
            Some(indicator) => stack![content, indicator].into(),
            None => content.into(),
        };
        let body = container(body)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_theme: &iced::Theme| container::Style {
                background: surface.map(Into::into),
                border: Border {
                    color: border,
                    width: if border.a > 0.0 { 1.0 } else { 0.0 },
                    radius: look.shape.tab,
                },
                shadow,
                ..container::Style::default()
            });
        let [top, side, bottom] = hl.pad;
        let body = container(body)
            .padding(iced::Padding {
                top,
                right: side,
                bottom,
                left: side,
            })
            .width(width)
            .height(look.bar.height);
        mouse_area(body)
            .on_press(Message::Drag(DragMsg::Start(Payload::Tab(tab.id))))
            .on_middle_press(Message::CloseTab(tab.id))
            .on_double_click(Message::StartRename(tab.id))
            .on_right_press(Message::OpenTabMenu(tab.id))
            .on_enter(Message::HoverTab(Some(tab.id)))
            .on_exit(Message::LeaveTab(tab.id))
            .into()
    }

    /// The 1 px between headers `index - 1` and `index`: a separator line between two inactive
    /// headers when the style draws them.
    fn gap(&self, index: usize) -> Element<'a, Message> {
        let look = self.look;
        let next_to_active = [index.wrapping_sub(1), index]
            .iter()
            .any(|i| self.tabs.get(*i).is_some_and(|t| t.id == self.active));
        if look.tabs.separators && !next_to_active {
            container(solid(
                look.border,
                Length::Fixed(GAP),
                look.bar.height * 0.42,
            ))
            .height(look.bar.height)
            .align_y(Alignment::Center)
            .into()
        } else {
            space().width(GAP).into()
        }
    }

    /// The scrollable headers with `+` / `⌄` after them; their empty rest moves the window.
    fn strip(&self, inset: f32, origin: Point) -> Element<'a, Message> {
        let look = self.look;
        let plus = icon_button(Icon::Plus, 28.0, look, Message::Action(Action::NewTab));
        // Right-click on `+` or the chevron next to it: the profile picker.
        let plus = mouse_area(plus).on_right_press(Message::Action(Action::OpenProfilePicker));
        let picker = icon_button(
            Icon::ChevronDown,
            24.0,
            look,
            Message::Action(Action::OpenProfilePicker),
        );
        let mut headers = Row::new().align_y(Alignment::Center);
        for (i, tab) in self.tabs.iter().enumerate() {
            if i > 0 {
                headers = headers.push(self.gap(i));
            }
            headers = headers.push(self.header(i, tab));
        }
        let scroller = Scrollbar::new().width(2).scroller_width(2);
        let strip = scrollable(headers)
            .id(Id::new(TAB_STRIP))
            .direction(Direction::Horizontal(scroller))
            .on_scroll(Message::TabStripViewport)
            .width(Length::Fill);
        // As wide as the headers when they fit (so `+` follows the last one), else whatever the
        // bar leaves after `+` and the buttons; the tiny spacer portion takes the rest.
        let total: f32 = self.widths.iter().map(|w| w + GAP).sum();
        let strip = container(strip)
            .width(Length::FillPortion(1000))
            .max_width(total.max(1.0));
        let add =
            container(row![plus, picker].spacing(0).align_y(Alignment::Center)).padding([6, 2]);
        let tabs = row![
            space().width(inset),
            strip,
            add,
            space().width(Length::FillPortion(1))
        ]
        .align_y(Alignment::Center)
        .width(Length::Fill);
        window_area(tabs.into(), origin)
            .on_scroll(Message::TabBarWheel)
            .into()
    }

    /// Panel toggles, then the window buttons (not on macOS, where the native ones show).
    fn controls(&self) -> Row<'a, Message> {
        let look = self.look;
        let toggle = |glyph: Icon, on: bool, tip: String, message| {
            let color = if on { look.accent } else { look.fg };
            let button = button(icon(glyph, 16.0, color))
                .width(TOGGLE_W)
                .height(28)
                .padding(0)
                .style(move |_theme: &iced::Theme, status| {
                    let mut style = controls::flat_button(look, status, false);
                    if on {
                        style.background = Some(look.accent_soft.into());
                    }
                    style
                })
                .on_press(message);
            controls::tip(button, tip, tooltip::Position::Bottom, look)
        };
        let mut bar_row = row![].align_y(Alignment::Center);
        if let Some(count) = self.forwards {
            bar_row = bar_row.push(toggle(
                Icon::Ports,
                count > 0,
                format!("Port forwarding ({count} listening)"),
                Message::Forwards(crate::forwards_dialog::ForwardsMsg::Open),
            ));
        }
        if let Some(open) = self.sftp {
            bar_row = bar_row.push(toggle(
                Icon::Folder,
                open,
                "SFTP panel (toggle-sftp)".to_owned(),
                Message::Action(Action::ToggleSftp),
            ));
        }
        bar_row = bar_row
            .push(toggle(
                Icon::Sidebar,
                self.profiles,
                "Profiles (toggle-profiles)".to_owned(),
                Message::Action(Action::ToggleProfiles),
            ))
            .push(toggle(
                Icon::Settings,
                false,
                "Settings".to_owned(),
                Message::OpenSettings,
            ));
        if !cfg!(target_os = "macos") {
            bar_row = bar_row
                .push(space().width(6))
                .push(solid(look.border, Length::Fixed(1.0), 20.0))
                .push(space().width(2))
                .push(window_button(
                    Icon::Minimize,
                    look,
                    false,
                    Message::Window(WindowCmd::Minimize),
                ))
                .push(window_button(
                    Icon::Maximize,
                    look,
                    false,
                    Message::Window(WindowCmd::ToggleMaximize),
                ))
                .push(window_button(
                    Icon::Close,
                    look,
                    true,
                    Message::CloseRequested,
                ));
        }
        bar_row
    }

    /// The bar's background: solid, the terminal colour, or an accent gradient.
    fn fill(&self) -> Background {
        let look = self.look;
        let raised = look.raised;
        match look.bar.fill {
            BarFill::Solid => raised.into(),
            BarFill::Base => look.base.into(),
            BarFill::Vertical => Linear::new(Radians(PI))
                .add_stop(0.0, mix(raised, look.accent, look.bar.gradient))
                .add_stop(1.0, raised)
                .into(),
            BarFill::Horizontal => Linear::new(Radians(FRAC_PI_2))
                .add_stop(0.0, mix(raised, look.accent, look.bar.gradient))
                .add_stop(0.5, raised)
                .add_stop(1.0, mix(raised, look.secondary, look.bar.gradient))
                .into(),
        }
    }

    /// `content` on the bar's surface, `height` px tall: flush with the window when docked, a
    /// rounded island with a light top edge otherwise. Folder tabs get the bar's bottom line
    /// under the headers, so the active one covers it and merges into the terminal.
    fn surface(
        &self,
        content: Element<'a, Message>,
        height: f32,
        merge: bool,
    ) -> Element<'a, Message> {
        let look = self.look;
        let fill = self.fill();
        let island = look.layout.geometry == Geometry::Islands;
        let content: Element<'a, Message> = if merge {
            let rule = column![
                Space::new().height(Length::Fill),
                controls::hairline(look.border)
            ];
            stack![rule, content].into()
        } else {
            content
        };
        let content: Element<'a, Message> = if island {
            // A 1 px sheen just inside the top edge: light catching the glass.
            let sheen = Color {
                a: if look.dark { 0.10 } else { 0.55 },
                ..Color::WHITE
            };
            let edge = container(controls::hairline(sheen))
                .padding([0.0, look.shape.card])
                .width(Length::Fill);
            stack![
                container(content).padding([0.0, ISLAND_PAD]),
                column![Space::new().height(1), edge]
            ]
            .into()
        } else {
            content
        };
        container(content)
            .width(Length::Fill)
            .height(height)
            .clip(true)
            .style(move |_theme: &iced::Theme| container::Style {
                background: Some(fill),
                border: if island {
                    Border {
                        color: look.border,
                        width: 1.0,
                        radius: look.shape.card.into(),
                    }
                } else {
                    Border::default()
                },
                shadow: if island {
                    look.shadow.panel
                } else {
                    iced::Shadow::default()
                },
                ..container::Style::default()
            })
            .into()
    }

    /// Top placement: one bar with the headers, the toggles and the window buttons. Bottom
    /// placement: a thin strip with the active title and the controls on top, the headers as a
    /// status line under the panes.
    pub fn view(self) -> BarParts<'a> {
        let look = self.look;
        let m = self.margin;
        let rule = look.bar.rule && look.layout.geometry == Geometry::Docked;
        let merge = look.tabs.style == TabStyle::Folder;
        match look.bar.placement {
            BarPlacement::Top => {
                let origin = Point::new(m, m);
                let content = row![self.strip(self.inset, origin), self.controls()]
                    .align_y(Alignment::Center);
                let bar = self.surface(content.into(), look.bar.height, merge && rule);
                let top = if rule && !merge {
                    column![bar, solid(look.border, Length::Fill, 1.0)].into()
                } else {
                    bar
                };
                BarParts { top, bottom: None }
            }
            BarPlacement::Bottom => {
                let origin = Point::new(m, m);
                let title = self
                    .tabs
                    .iter()
                    .find(|t| t.id == self.active)
                    .map(|t| t.title().to_owned())
                    .unwrap_or_default();
                let title = container(
                    text(title)
                        .size(11)
                        .font(look.text.chrome)
                        .color(look.dim)
                        .wrapping(text::Wrapping::None),
                )
                .padding([0.0, look.layout.gutter])
                .height(Length::Fill)
                .align_y(Alignment::Center)
                .clip(true);
                let drag = window_area(
                    row![
                        space().width(self.inset),
                        title,
                        space().width(Length::Fill)
                    ]
                    .height(Length::Fill)
                    .into(),
                    origin,
                );
                let strip = row![drag, self.controls()]
                    .align_y(Alignment::Center)
                    .height(Length::Fill);
                let strip = self.surface(strip.into(), look.bar.strip, false);
                let top = if rule {
                    column![strip, solid(look.border, Length::Fill, 1.0)].into()
                } else {
                    strip
                };
                let bottom_origin = Point::new(m, self.window.height - m - look.bar.height);
                let tabs = self.surface(self.strip(0.0, bottom_origin), look.bar.height, false);
                let bottom = if rule {
                    column![solid(look.border, Length::Fill, 1.0), tabs].into()
                } else {
                    tabs
                };
                BarParts {
                    top,
                    bottom: Some(bottom),
                }
            }
        }
    }
}

/// `content` as part of the bar's empty area: presses move the window, a double click
/// maximizes it, a right click opens the bar menu; pointer moves report window coordinates.
fn window_area(
    content: Element<'_, Message>,
    origin: Point,
) -> iced::widget::MouseArea<'_, Message> {
    mouse_area(content)
        .on_press(Message::Window(WindowCmd::Press))
        .on_release(Message::Window(WindowCmd::Release))
        .on_double_click(Message::Window(WindowCmd::ToggleMaximize))
        .on_right_press(Message::TabBarMenu)
        .on_move(move |p| Message::TabBarCursor(Point::new(p.x + origin.x, p.y + origin.y)))
}

/// A flat bar button `width` px wide with a 16 px icon.
fn icon_button<'a>(glyph: Icon, width: f32, look: Look, message: Message) -> Element<'a, Message> {
    button(icon(glyph, 16.0, look.fg))
        .width(width)
        .height(28)
        .padding(0)
        .style(move |_theme: &iced::Theme, status| controls::flat_button(look, status, false))
        .on_press(message)
        .into()
}

/// Minimize / maximize / close at the bar's right end (close turns white on red).
fn window_button<'a>(
    glyph: Icon,
    look: Look,
    danger: bool,
    message: Message,
) -> Element<'a, Message> {
    button(icon(glyph, 15.0, look.fg))
        .width(44)
        .height(Length::Fill)
        .padding(0)
        .style(move |_theme: &iced::Theme, status| {
            let mut style = controls::flat_button(look, status, danger);
            style.border = Border::default().rounded(0);
            style
        })
        .on_press(message)
        .into()
}

/// Invisible strips along the window's edges and corners that start a resize (there are no OS
/// decorations to do it); layered over everything else.
pub fn resize_edges<'a>() -> Element<'a, Message> {
    use iced::mouse::Interaction;
    use iced::window::Direction as D;
    let edge = |w: Length, h: Length, dir: D, cursor: Interaction| -> Element<'a, Message> {
        mouse_area(space().width(w).height(h))
            .interaction(cursor)
            .on_press(Message::Window(WindowCmd::Resize(dir)))
            .into()
    };
    let (e, fill) = (Length::Fixed(EDGE), Length::Fill);
    let (h, v) = (
        Interaction::ResizingHorizontally,
        Interaction::ResizingVertically,
    );
    let (up, down) = (
        Interaction::ResizingDiagonallyUp,
        Interaction::ResizingDiagonallyDown,
    );
    column![
        row![
            edge(e, e, D::NorthWest, down),
            edge(fill, e, D::North, v),
            edge(e, e, D::NorthEast, up)
        ],
        row![
            edge(e, fill, D::West, h),
            space().width(fill),
            edge(e, fill, D::East, h)
        ]
        .height(fill),
        row![
            edge(e, e, D::SouthWest, up),
            edge(fill, e, D::South, v),
            edge(e, e, D::SouthEast, down)
        ],
    ]
    .into()
}

/// A persistent `!` badge when a pane of `tab` reported a warning (e.g. host key not
/// verified); the tooltip says what.
fn warning_badge(tab: &Tab, look: Look) -> Option<Element<'_, Message>> {
    let lines: Vec<String> = tab
        .warnings()
        .map(|w| format!("{}: {}", warning_title(w), warning_detail(w)))
        .collect();
    if lines.is_empty() {
        return None;
    }
    let radius = look.shape.control.min(6.0);
    let badge = container(
        text("!")
            .size(11)
            .font(look.text.strong)
            .color(Color::BLACK),
    )
    .padding([0, 5])
    .style(move |_theme: &iced::Theme| container::Style {
        background: Some(WARN.into()),
        border: Border::default().rounded(radius),
        ..container::Style::default()
    });
    Some(controls::tip(
        badge,
        lines.join("\n"),
        tooltip::Position::Bottom,
        look,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_long_titles_with_ellipsis() {
        assert_eq!(truncate("zsh"), "zsh");
        let long = "a".repeat(40);
        let cut = truncate(&long);
        assert_eq!(cut.chars().count(), MAX_TITLE_CHARS);
        assert!(cut.ends_with('…'));
        assert_eq!(
            truncate(&"Ǹ".repeat(MAX_TITLE_CHARS)).chars().count(),
            MAX_TITLE_CHARS
        );
    }
}
