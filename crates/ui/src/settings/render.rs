//! Settings rows: label, help and badge on the left, the control on the right, drawn from a
//! [`Spec`]; plus the styled controls the pages share.

use iced::widget::{column, container, pick_list, row, slider, space, text, text_input, toggler};
use iced::{Alignment, Background, Border, Color, Element, Length};

use super::spec::{Kind, SettingId, Spec, Value};
use super::{Edit, SettingsMsg, SettingsPage};
use crate::controls;
use crate::message::Message;
use crate::palette::{Look, mix};

/// Width of the control column.
pub const CONTROL_W: f32 = 300.0;
/// Validation errors.
pub const ERROR: Color = Color::from_rgb(0.90, 0.28, 0.30);

/// The rows of `ids` under their group headings (a heading starts wherever the group changes).
pub fn rows<'a>(
    page: &'a SettingsPage,
    look: Look,
    config: &fogu_core::Config,
    ids: impl IntoIterator<Item = SettingId>,
) -> Element<'a, Message> {
    let mut body = column![].spacing(6);
    let mut group = "";
    for id in ids {
        let spec = id.spec();
        if spec.group != group {
            if !group.is_empty() {
                body = body.push(space().height(14));
            }
            group = spec.group;
            body = body.push(controls::section_label(look, group));
        }
        body = body.push(setting_row(page, look, id, &spec, &id.get(config)));
    }
    body.into()
}

/// One setting's row.
fn setting_row<'a>(
    page: &'a SettingsPage,
    look: Look,
    id: SettingId,
    spec: &Spec,
    value: &Value,
) -> Element<'a, Message> {
    let control = control(page, look, id, spec, value);
    let mut title = row![
        text(spec.label)
            .size(13)
            .font(look.text.chrome)
            .color(look.fg)
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    if let Some(badge) = spec.applies.badge() {
        title = title.push(badge_chip(look, badge));
    }
    // The error line is always there (empty when fine) so a text field keeps the keyboard
    // while an error comes and goes.
    let error = page.errors.get(&id).cloned().unwrap_or_default();
    let error_h = if error.is_empty() {
        Length::Fixed(0.0)
    } else {
        Length::Shrink
    };
    let left = column![
        title,
        text(spec.help)
            .size(11)
            .font(look.text.chrome)
            .color(look.dim),
        text(error)
            .size(11)
            .font(look.text.chrome)
            .color(ERROR)
            .height(error_h),
    ]
    .spacing(3)
    .width(Length::Fill);
    let line = row![left, container(control).width(CONTROL_W)]
        .spacing(16)
        .align_y(Alignment::Center);
    let highlighted = page.highlight == Some(super::Target::Setting(id));
    super::anchored(id.widget_id(), highlighted, line.into(), look)
}

/// Setting `id`'s control, right-aligned in its column.
fn control<'a>(
    page: &'a SettingsPage,
    look: Look,
    id: SettingId,
    spec: &Spec,
    value: &Value,
) -> Element<'a, Message> {
    let msg = move |edit: Edit| Message::Settings(SettingsMsg::Set(id, edit));
    let end = |content: Element<'a, Message>| -> Element<'a, Message> {
        container(content)
            .align_x(Alignment::End)
            .width(Length::Fill)
            .into()
    };
    match spec.kind {
        Kind::Toggle => end(switch(look, value.bool(), move |b| msg(Edit::Bool(b)))),
        Kind::Slider {
            min,
            max,
            step,
            unit,
            auto,
        } => slider_control(look, id, (min, max, step), unit, auto, value),
        Kind::Number { .. } | Kind::Text { .. } => {
            let shown = page
                .pending
                .get(&id)
                .cloned()
                .unwrap_or_else(|| match value {
                    Value::Num(n) => format!("{n}"),
                    other => other.text().to_owned(),
                });
            let (placeholder, width) = match spec.kind {
                Kind::Text { placeholder } => (placeholder, Length::Fill),
                _ => ("", Length::Fixed(140.0)),
            };
            end(field(look, placeholder, &shown)
                .on_input(move |t| msg(Edit::Text(t)))
                .on_submit(msg(Edit::Submit))
                .width(width)
                .into())
        }
        Kind::Choice(options) => end(choice(look, options, value.choice(), move |i| {
            msg(Edit::Choice(i))
        })),
        Kind::Font => match &page.fonts {
            Some(fonts) => font_picker(look, id, &fonts.family, value.text()),
            None => end(small(look, value.text())),
        },
        Kind::Fonts => font_list(look, id, page.fonts.as_ref(), value.list()),
        Kind::Flags(labels) => labels
            .iter()
            .enumerate()
            .fold(row![].spacing(6), |line, (i, label)| {
                line.push(flag(look, label, value.flag(i), msg(Edit::Flag(i))))
            })
            .wrap()
            .into(),
    }
}

/// A slider with its value (and an "automatic" switch for optional values).
fn slider_control<'a>(
    look: Look,
    id: SettingId,
    (min, max, step): (f32, f32, f32),
    unit: super::spec::Unit,
    auto: Option<&'static str>,
    value: &Value,
) -> Element<'a, Message> {
    let msg = move |edit: Edit| Message::Settings(SettingsMsg::Set(id, edit));
    let auto_on = matches!(value, Value::Auto);
    let shown = if auto_on {
        auto.unwrap_or_default().to_owned()
    } else {
        unit.show(value.num())
    };
    let bar = range(
        look,
        (f64::from(min), f64::from(max), f64::from(step)),
        if auto_on { f64::from(min) } else { value.num() },
        move |v| msg(Edit::Slide(v)),
        msg(Edit::SlideDone),
    );
    let mut line = row![
        bar,
        text(shown)
            .size(12)
            .font(look.text.chrome)
            .color(look.dim)
            .width(64)
            .align_x(Alignment::End),
    ]
    .spacing(10)
    .align_y(Alignment::Center);
    if let Some(label) = auto {
        line = line.push(
            row![
                switch(look, auto_on, move |b| msg(Edit::Auto(b))),
                small(look, label),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        );
    }
    line.into()
}

/// The font family field: the installed families to pick from, or a name typed (saved on
/// Enter or a click elsewhere).
fn font_picker<'a>(
    look: Look,
    id: SettingId,
    state: &'a iced::widget::combo_box::State<String>,
    current: &str,
) -> Element<'a, Message> {
    let msg = move |edit: Edit| Message::Settings(SettingsMsg::Set(id, edit));
    let current = current.to_owned();
    iced::widget::combo_box(state, &current, Some(&current), move |f| msg(Edit::Pick(f)))
        .on_input(move |t| msg(Edit::Text(t)))
        .on_close(msg(Edit::Submit))
        .size(13.0)
        .font(look.text.chrome)
        .padding([5, 8])
        .width(Length::Fill)
        .input_style(move |_theme: &iced::Theme, status| controls::field(look, status))
        .menu_style(move |_theme: &iced::Theme| menu_style(look))
        .into()
}

/// A list of font families: removable chips, then a picker that adds one.
fn font_list<'a>(
    look: Look,
    id: SettingId,
    fonts: Option<&'a super::FontPickers>,
    items: &[String],
) -> Element<'a, Message> {
    let msg = move |edit: Edit| Message::Settings(SettingsMsg::Set(id, edit));
    let chips = items
        .iter()
        .enumerate()
        .fold(row![].spacing(6), |line, (i, name)| {
            line.push(chip(look, name, msg(Edit::Remove(i))))
        });
    let add: Element<'a, Message> = match fonts {
        Some(fonts) => {
            iced::widget::combo_box(&fonts.fallback, "Add a fallback font…", None, move |f| {
                msg(Edit::Add(f))
            })
            .size(13.0)
            .font(look.text.chrome)
            .padding([5, 8])
            .width(Length::Fill)
            .input_style(move |_theme: &iced::Theme, status| controls::field(look, status))
            .menu_style(move |_theme: &iced::Theme| menu_style(look))
            .into()
        }
        None => space().into(),
    };
    column![chips.wrap(), add].spacing(6).into()
}

/// A removable name: `label ×`.
fn chip<'a>(look: Look, label: &str, on_remove: Message) -> Element<'a, Message> {
    iced::widget::button(
        row![
            text(label.to_owned())
                .size(12)
                .font(look.text.chrome)
                .color(look.fg),
            text("×").size(12).font(look.text.chrome).color(look.dim),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    )
    .padding([3, 8])
    .style(move |_theme: &iced::Theme, status| {
        let mut style = controls::flat_button(look, status, false);
        style.border = Border {
            color: look.border,
            width: 1.0,
            radius: look.shape.control.into(),
        };
        style
    })
    .on_press(on_remove)
    .into()
}

/// One flag of a flag set: a pill that is filled while on.
fn flag<'a>(look: Look, label: &str, on: bool, on_press: Message) -> Element<'a, Message> {
    let (bg, fg) = controls::row_colors(look, on, false);
    iced::widget::button(
        text(label.to_owned())
            .size(12)
            .font(look.text.chrome)
            .color(if on { fg } else { look.dim }),
    )
    .padding([4, 10])
    .style(move |_theme: &iced::Theme, status| {
        let mut style = controls::flat_button(look, status, false);
        if on {
            style.background = bg.map(Into::into);
        }
        style.border = Border {
            color: if on { look.accent } else { look.border },
            width: 1.0,
            radius: look.shape.control.into(),
        };
        style
    })
    .on_press(on_press)
    .into()
}

/// A small rounded label ("New panes").
pub fn badge_chip<'a>(look: Look, label: &str) -> Element<'a, Message> {
    container(
        text(label.to_owned())
            .size(10)
            .font(look.text.chrome)
            .color(look.dim),
    )
    .padding([1, 6])
    .style(move |_theme: &iced::Theme| container::Style {
        background: Some(mix(look.panel, look.fg, 0.06).into()),
        border: Border {
            color: look.border,
            width: 1.0,
            radius: look.shape.control.into(),
        },
        ..container::Style::default()
    })
    .into()
}

/// Dim small text.
pub fn small<'a>(look: Look, label: &str) -> Element<'a, Message> {
    text(label.to_owned())
        .size(11)
        .font(look.text.chrome)
        .color(look.dim)
        .into()
}

/// An on/off switch in the style's accent.
pub fn switch<'a>(
    look: Look,
    on: bool,
    on_toggle: impl Fn(bool) -> Message + 'a,
) -> Element<'a, Message> {
    toggler(on)
        .on_toggle(on_toggle)
        .size(18)
        .style(move |_theme: &iced::Theme, status| {
            let (toggled, hovered) = match status {
                toggler::Status::Active { is_toggled }
                | toggler::Status::Disabled { is_toggled } => (is_toggled, false),
                toggler::Status::Hovered { is_toggled } => (is_toggled, true),
            };
            let off = mix(look.panel, look.fg, if hovered { 0.22 } else { 0.16 });
            toggler::Style {
                background: Background::Color(if toggled { look.accent } else { off }),
                background_border_width: 0.0,
                background_border_color: Color::TRANSPARENT,
                foreground: Background::Color(if toggled { look.on_accent } else { look.fg }),
                foreground_border_width: 0.0,
                foreground_border_color: Color::TRANSPARENT,
                text_color: Some(look.fg),
                border_radius: None,
                padding_ratio: 0.12,
            }
        })
        .into()
}

/// A slider over `(min, max, step)` that reports every move and the release.
pub fn range<'a>(
    look: Look,
    (min, max, step): (f64, f64, f64),
    value: f64,
    on_change: impl Fn(f64) -> Message + 'a,
    on_release: Message,
) -> Element<'a, Message> {
    slider(min..=max, value.clamp(min, max), on_change)
        .step(step)
        .on_release(on_release)
        .width(Length::Fill)
        .style(move |_theme: &iced::Theme, status| {
            let hot = !matches!(status, slider::Status::Active);
            slider::Style {
                rail: slider::Rail {
                    backgrounds: (
                        Background::Color(look.accent),
                        Background::Color(mix(look.panel, look.fg, 0.16)),
                    ),
                    width: 4.0,
                    border: Border::default().rounded(2.0),
                },
                handle: slider::Handle {
                    shape: slider::HandleShape::Circle {
                        radius: if hot { 8.0 } else { 7.0 },
                    },
                    background: Background::Color(look.bg),
                    border_width: 2.0,
                    border_color: look.accent,
                },
            }
        })
        .into()
}

/// A text field in the settings style.
pub fn field<'a>(look: Look, placeholder: &str, value: &str) -> text_input::TextInput<'a, Message> {
    text_input(placeholder, value)
        .size(13)
        .font(look.text.chrome)
        .padding([5, 8])
        .style(move |_theme: &iced::Theme, status| controls::field(look, status))
}

/// A drop-down of `options` with `selected` picked.
pub fn choice<'a>(
    look: Look,
    options: &'static [&'static str],
    selected: usize,
    on_pick: impl Fn(usize) -> Message + 'a,
) -> Element<'a, Message> {
    let current = options.get(selected).copied();
    pick_list(options, current, move |picked: &'static str| {
        on_pick(options.iter().position(|o| *o == picked).unwrap_or(0))
    })
    .text_size(13)
    .font(look.text.chrome)
    .padding([5, 8])
    .style(move |_theme: &iced::Theme, status| pick_style(look, status))
    .menu_style(move |_theme: &iced::Theme| menu_style(look))
    .into()
}

fn pick_style(look: Look, status: pick_list::Status) -> pick_list::Style {
    let open = matches!(
        status,
        pick_list::Status::Opened { .. } | pick_list::Status::Hovered
    );
    pick_list::Style {
        text_color: look.fg,
        placeholder_color: look.dim,
        handle_color: look.dim,
        background: Background::Color(mix(look.panel, look.base, 0.55)),
        border: Border {
            color: if open { look.focus } else { look.border },
            width: look.shape.border.max(1.0),
            radius: look.shape.control.into(),
        },
    }
}

fn menu_style(look: Look) -> iced::overlay::menu::Style {
    iced::overlay::menu::Style {
        background: Background::Color(look.bg),
        border: Border {
            color: look.border,
            width: 1.0,
            radius: look.shape.control.into(),
        },
        text_color: look.fg,
        selected_text_color: look.fg,
        selected_background: Background::Color(Color {
            a: 0.22,
            ..look.accent
        }),
        shadow: look.shadow.floating,
    }
}
