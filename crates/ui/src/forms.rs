//! Generic form helpers: one labelled row per field, built once and reused by the profile editor
//! and, later, the settings pages. Every widget publishes a [`Field`]: the caller's field id plus a
//! [`FieldValue`]; the caller's reducer maps the id to the struct field it edits (one `match`).
//!
//! Stock iced widgets on the scheme colours (like the SFTP panel and the profile sidebar); the
//! text fields keep iced's default look (graphics polish is postponed).

use std::path::PathBuf;

use fogu_core::RgbColor;
use iced::widget::{
    Column, Row, button, checkbox, column, container, pick_list, row, space, text, text_input,
    tooltip,
};
use iced::{Alignment, Color, Element, Length};

use crate::color_picker::{color_picker, preview};
use crate::context_menu::{PRESET_COLORS, to_iced};
use crate::sftp::panel::Look;

/// Width of the label column, logical px.
pub const LABEL_W: f32 = 220.0;
/// Width of a number field.
const NUMBER_W: f32 = 140.0;
const LABEL_SIZE: f32 = 13.0;
const INPUT_SIZE: f32 = 13.0;
const NOTE_SIZE: f32 = 11.0;
/// "Not supported yet" notes and warnings (amber, like the tab warning badge).
pub const WARN: Color = Color::from_rgb(0.96, 0.65, 0.14);
/// Validation errors.
pub const ERROR: Color = Color::from_rgb(0.90, 0.28, 0.30);

/// What a widget reports; the caller's reducer interprets it for the field.
#[derive(Clone, Debug, PartialEq)]
pub enum FieldValue {
    /// A text or number field changed (numbers are parsed by the reducer).
    Text(String),
    /// A checkbox.
    Bool(bool),
    /// The index of the option picked in a [`choice`].
    Choice(usize),
    /// A colour swatch or "None".
    Color(Option<RgbColor>),
    /// "Browse…": open a file picker for this field.
    Browse,
    /// The picker returned this file.
    Picked(PathBuf),
    /// "+ Add" on a list.
    Add,
    /// "×" on list row `.0`.
    Remove(usize),
    /// "↑" on list row `.0` (moves it one up).
    Up(usize),
    /// List row `.0`'s text changed (a key-value list: its key).
    Item(usize, String),
    /// Key-value row `.0`'s value changed.
    Value(usize, String),
    /// A multi-select entry was ticked or unticked.
    Toggle(String),
    /// "Set…" on a secret.
    SetSecret,
    /// "Clear" on a secret.
    ClearSecret,
    /// Right-click on a text field at `.0` (window coordinates); `.1` is the text it shows.
    Menu(iced::Point, String),
}

/// One widget event: which field, what happened.
#[derive(Clone, Debug, PartialEq)]
pub struct Field<Id>(pub Id, pub FieldValue);

/// `label` on the left, `control` on the right.
pub(crate) fn labeled<'a, M: 'a>(
    look: Look,
    label: &str,
    control: Element<'a, M>,
) -> Element<'a, M> {
    row![
        text(label.to_owned())
            .size(LABEL_SIZE)
            .color(look.fg)
            .width(LABEL_W),
        control,
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .into()
}

pub(crate) fn input<'a, Id: Copy + 'a>(
    id: Id,
    value: &str,
    placeholder: &str,
) -> iced::widget::TextInput<'a, Field<Id>> {
    text_input(placeholder, value)
        .on_input(move |s| Field(id, FieldValue::Text(s)))
        .size(INPUT_SIZE)
        .padding(4)
}

/// `field` (showing `value`) with a right-click menu: reports [`FieldValue::Menu`].
fn with_menu<'a, Id: Copy + 'a>(
    id: Id,
    value: &str,
    field: impl Into<Element<'a, Field<Id>>>,
) -> Element<'a, Field<Id>> {
    let value = value.to_owned();
    crate::right_click::right_click(field, move |at| {
        Field(id, FieldValue::Menu(at, value.clone()))
    })
    .into()
}

fn small_button<'a, M: Clone + 'a>(label: &str, on_press: Option<M>) -> Element<'a, M> {
    button(text(label.to_owned()).size(LABEL_SIZE))
        .padding([2, 8])
        .style(button::secondary)
        .on_press_maybe(on_press)
        .into()
}

/// A one-line text field; `placeholder` shows the inherited value.
pub fn text_field<'a, Id: Copy + 'a>(
    look: Look,
    id: Id,
    label: &str,
    value: &str,
    placeholder: &str,
) -> Element<'a, Field<Id>> {
    labeled(
        look,
        label,
        with_menu(id, value, input(id, value, placeholder)),
    )
}

/// A number field (the reducer parses it and checks the range).
pub fn number<'a, Id: Copy + 'a>(
    look: Look,
    id: Id,
    label: &str,
    value: &str,
    placeholder: &str,
) -> Element<'a, Field<Id>> {
    labeled(
        look,
        label,
        with_menu(id, value, input(id, value, placeholder).width(NUMBER_W)),
    )
}

/// A checkbox.
pub fn toggle<'a, Id: Copy + 'a>(
    look: Look,
    id: Id,
    label: &str,
    value: bool,
) -> Element<'a, Field<Id>> {
    let check = checkbox(value).on_toggle(move |b| Field(id, FieldValue::Bool(b)));
    labeled(look, label, check.into())
}

/// A drop-down of `options`; `selected` is an index into them.
pub fn choice<'a, Id: Copy + 'a>(
    look: Look,
    id: Id,
    label: &str,
    options: Vec<String>,
    selected: usize,
) -> Element<'a, Field<Id>> {
    let current = options.get(selected).cloned();
    let lookup = options.clone();
    let list = pick_list(options, current, move |picked: String| {
        let index = lookup.iter().position(|o| *o == picked).unwrap_or(0);
        Field(id, FieldValue::Choice(index))
    })
    .text_size(INPUT_SIZE)
    .padding(4);
    labeled(look, label, list.into())
}

/// A colour: the preset swatches, "None" and a `#rrggbb` field (`hex`).
pub fn color<'a, Id: Copy + 'a>(
    look: Look,
    id: Id,
    label: &str,
    value: Option<RgbColor>,
    hex: &str,
) -> Element<'a, Field<Id>> {
    let mut swatches = Row::new().spacing(4).align_y(Alignment::Center);
    for (name, rgb) in PRESET_COLORS {
        let rgb = RgbColor::from_hex(rgb);
        let fill = to_iced(rgb);
        let ring = if value == Some(rgb) { look.fg } else { fill };
        let swatch = button(space().width(16).height(16))
            .padding(0)
            .style(move |_theme: &iced::Theme, _status| button::Style {
                background: Some(fill.into()),
                border: iced::Border {
                    color: ring,
                    width: 2.0,
                    radius: 3.0.into(),
                },
                ..button::Style::default()
            })
            .on_press(Field(id, FieldValue::Color(Some(rgb))));
        swatches = swatches.push(tooltip(
            swatch,
            container(text(name).size(NOTE_SIZE)).padding(4),
            tooltip::Position::Top,
        ));
    }
    let none = small_button("None", Some(Field(id, FieldValue::Color(None))));
    let field = with_menu(id, hex, input(id, hex, "#rrggbb").width(100));
    let picker = color_picker(value, 260.0, move |c| Field(id, FieldValue::Color(Some(c))));
    labeled(
        look,
        label,
        column![
            row![swatches, none, field, preview(value, 24.0, look.border)]
                .spacing(8)
                .align_y(Alignment::Center),
            Element::from(picker),
        ]
        .spacing(6)
        .into(),
    )
}

/// A file path with a "Browse…" button (disabled with `why` when no picker is available).
pub fn path<'a, Id: Copy + 'a>(
    look: Look,
    id: Id,
    label: &str,
    value: &str,
    placeholder: &str,
    picker: Result<(), &'static str>,
) -> Element<'a, Field<Id>> {
    let field = with_menu(id, value, input(id, value, placeholder).width(Length::Fill));
    let browse = browse_button(id, picker);
    labeled(
        look,
        label,
        row![field, browse]
            .spacing(6)
            .align_y(Alignment::Center)
            .into(),
    )
}

fn browse_button<'a, Id: Copy + 'a>(
    id: Id,
    picker: Result<(), &'static str>,
) -> Element<'a, Field<Id>> {
    let button = small_button(
        "Browse…",
        picker.is_ok().then_some(Field(id, FieldValue::Browse)),
    );
    match picker {
        Ok(()) => button,
        Err(why) => tooltip(
            button,
            container(text(why).size(NOTE_SIZE)).padding(4),
            tooltip::Position::Top,
        )
        .into(),
    }
}

/// An ordered list of strings: each row editable, movable up and removable; "+ Add" appends an
/// empty row, `browse` (path lists) adds "Browse…".
pub fn list<'a, Id: Copy + 'a>(
    look: Look,
    id: Id,
    label: &str,
    items: &[String],
    placeholder: &str,
    browse: Option<Result<(), &'static str>>,
) -> Element<'a, Field<Id>> {
    let mut rows = Column::new().spacing(4);
    for (i, item) in items.iter().enumerate() {
        let field = text_input(placeholder, item)
            .on_input(move |s| Field(id, FieldValue::Item(i, s)))
            .size(INPUT_SIZE)
            .padding(4);
        rows = rows.push(list_row(id, i, field.into()));
    }
    let mut add = row![small_button("+ Add", Some(Field(id, FieldValue::Add)))].spacing(6);
    if let Some(picker) = browse {
        add = add.push(browse_button(id, picker));
    }
    labeled(look, label, rows.push(add).into())
}

/// `content` followed by "↑" and "×" for list row `i`.
pub fn list_row<'a, Id: Copy + 'a>(
    id: Id,
    i: usize,
    content: Element<'a, Field<Id>>,
) -> Element<'a, Field<Id>> {
    let up = small_button("↑", (i > 0).then_some(Field(id, FieldValue::Up(i))));
    let remove = small_button("×", Some(Field(id, FieldValue::Remove(i))));
    row![container(content).width(Length::Fill), up, remove]
        .spacing(4)
        .align_y(Alignment::Center)
        .into()
}

/// Key = value rows (environment variables).
pub fn kv_list<'a, Id: Copy + 'a>(
    look: Look,
    id: Id,
    label: &str,
    items: &[(String, String)],
) -> Element<'a, Field<Id>> {
    let mut rows = Column::new().spacing(4);
    for (i, (key, value)) in items.iter().enumerate() {
        let key = text_input("NAME", key)
            .on_input(move |s| Field(id, FieldValue::Item(i, s)))
            .size(INPUT_SIZE)
            .padding(4)
            .width(Length::FillPortion(1));
        let value = text_input("value", value)
            .on_input(move |s| Field(id, FieldValue::Value(i, s)))
            .size(INPUT_SIZE)
            .padding(4)
            .width(Length::FillPortion(2));
        let pair = row![key, text("=").color(look.dim), value]
            .spacing(4)
            .align_y(Alignment::Center);
        rows = rows.push(list_row(id, i, pair.into()));
    }
    let add = small_button("+ Add", Some(Field(id, FieldValue::Add)));
    labeled(look, label, rows.push(add).into())
}

/// A multi-select: one checkbox per `(value, label, ticked)`; a click reports
/// [`FieldValue::Toggle`] with the value.
pub fn multi<'a, Id: Copy + 'a>(
    look: Look,
    id: Id,
    label: &str,
    options: &[(String, String, bool)],
) -> Element<'a, Field<Id>> {
    let mut boxes = Column::new().spacing(3);
    for (value, name, ticked) in options {
        let toggled = value.clone();
        let check = checkbox(*ticked)
            .on_toggle(move |_| Field(id, FieldValue::Toggle(toggled.clone())))
            .size(14);
        boxes = boxes.push(
            row![check, text(name.clone()).size(LABEL_SIZE).color(look.fg)]
                .spacing(6)
                .align_y(Alignment::Center),
        );
    }
    labeled(look, label, boxes.into())
}

/// A secret that is never shown: its `status` ("stored" / "not stored") with Set… / Clear.
/// `disabled` = why the buttons are off (shown as their tooltip).
pub fn secret_hint<'a, Id: Copy + 'a>(
    look: Look,
    id: Id,
    label: &str,
    status: &str,
    disabled: Option<&'static str>,
) -> Element<'a, Field<Id>> {
    let on = |value| disabled.is_none().then_some(Field(id, value));
    let buttons = row![
        small_button("Set…", on(FieldValue::SetSecret)),
        small_button("Clear", on(FieldValue::ClearSecret)),
    ]
    .spacing(6);
    let buttons: Element<'a, Field<Id>> = match disabled {
        Some(why) => tooltip(
            buttons,
            container(text(why).size(NOTE_SIZE)).padding(4),
            tooltip::Position::Top,
        )
        .into(),
        None => buttons.into(),
    };
    let row = row![
        text(status.to_owned()).size(LABEL_SIZE).color(look.dim),
        buttons
    ]
    .spacing(12)
    .align_y(Alignment::Center);
    labeled(look, label, row.into())
}

/// `field` with a small line under it (aligned with the controls), in `color`.
pub fn note<'a, M: 'a>(field: Element<'a, M>, line: &str, color: Color) -> Element<'a, M> {
    note_maybe(field, Some((line, color)))
}

/// [`note`] when `line` is set, else `field` alone. The widget tree is the same either way
/// (the line is just 0 px high), so a text field keeps the keyboard when a note comes or goes
/// while the user types.
pub fn note_maybe<'a, M: 'a>(field: Element<'a, M>, line: Option<(&str, Color)>) -> Element<'a, M> {
    let below: Element<'a, M> = match line {
        Some((words, color)) => row![
            space().width(LABEL_W + 8.0),
            text(words.to_owned())
                .size(NOTE_SIZE)
                .color(color)
                .width(Length::Fill)
        ]
        .into(),
        None => space().height(0).into(),
    };
    column![field, below]
        .spacing(if line.is_some() { 2 } else { 0 })
        .into()
}

/// A titled group of rows.
pub fn section<'a, M: 'a>(look: Look, title: &str, rows: Vec<Element<'a, M>>) -> Element<'a, M> {
    let rule = container(space())
        .width(Length::Fill)
        .height(1)
        .style(move |_theme: &iced::Theme| container::background(look.dim));
    let mut body = column![text(title.to_owned()).size(16).color(look.fg), rule].spacing(8);
    for r in rows {
        body = body.push(r);
    }
    body.padding([8, 0]).into()
}
