//! Profile picker: every profile of the profile sidebar (built-ins first) in a fuzzy
//! filtered list (`fuzzy_list`); Enter / click opens it in a new tab, "Split" next to the
//! focused pane. Opening records it in Recent.

use crate::app::App;
use crate::fuzzy_list::{self, FuzzyRow, RowEvents, RowLook};
use crate::message::Message;
use crate::palette::Look;
use crate::profiles::{self, Entry, Launch, now_secs};
use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{
    Id, column, container, mouse_area, opaque, operation, row, scrollable, space, stack, text_input,
};
use iced::{Alignment, Color, Element, Length, Task};

/// Id of the filter field.
pub const PICKER_INPUT: &str = "fogu-picker-input";
const PANEL_W: f32 = 480.0;

/// The open picker.
#[derive(Clone, Debug, Default)]
pub(crate) struct ProfilePicker {
    pub filter: String,
    /// Position in the filtered list.
    pub selected: usize,
}

/// Picker messages.
#[derive(Clone, Debug)]
pub enum PickerMsg {
    /// The filter changed.
    Filter(String),
    /// Arrow up (`-1`) / down (`1`).
    Move(i8),
    /// Enter in the filter: open the selected entry in a tab.
    Submit,
    /// Open entry `index` (of [`entries`]) in a tab, or in a split.
    Open { index: usize, split: bool },
    /// Click outside the panel.
    Close,
}

/// What the picker lists: every profile of the sidebar, as entries and fuzzy rows.
pub(crate) fn listing(app: &App) -> (Vec<Entry>, Vec<FuzzyRow>) {
    let entries = app.all_entries();
    let mut rows = profiles::rows(&entries, &app.config, &app.profiles.recent, now_secs());
    // A flat list: say where a `~/.ssh/config` host comes from (a stored copy looks the same).
    for (row, entry) in rows.iter_mut().zip(&entries) {
        if entry.origin == profiles::Origin::SshConfig {
            row.subtitle.push_str(" · ssh config");
        }
    }
    (entries, rows)
}

impl App {
    /// `OpenProfilePicker` / right-click or `▾` on the tab bar's `+`.
    pub(crate) fn open_picker(&mut self) -> Task<Message> {
        self.menu = None;
        self.picker = Some(ProfilePicker::default());
        Task::batch([
            self.refresh_ssh_hosts(),
            operation::focus(Id::new(PICKER_INPUT)),
        ])
    }

    pub(crate) fn on_picker(&mut self, message: PickerMsg) -> Task<Message> {
        let (entries, rows) = listing(self);
        let Some(picker) = self.picker.as_mut() else {
            return Task::none();
        };
        let shown = fuzzy_list::rank(&picker.filter, &rows);
        let open = match message {
            PickerMsg::Filter(filter) => {
                picker.filter = filter;
                picker.selected = 0;
                None
            }
            PickerMsg::Move(delta) => {
                picker.selected = fuzzy_list::step(picker.selected, delta, shown.len());
                None
            }
            PickerMsg::Submit => shown.get(picker.selected).map(|&i| (i, false)),
            PickerMsg::Open { index, split } => Some((index, split)),
            PickerMsg::Close => {
                self.picker = None;
                None
            }
        };
        let Some((index, split)) = open else {
            return Task::none();
        };
        self.picker = None;
        let Some(Entry { profile, .. }) = entries.into_iter().nth(index) else {
            return Task::none();
        };
        self.launch(&profile, if split { Launch::Split } else { Launch::Tab })
    }
}

/// One list row (the sidebar's look) and its "Split" button.
fn entry_row(
    index: usize,
    entry: &FuzzyRow,
    selected: bool,
    look: Look,
) -> Element<'static, Message> {
    let open = |split| Message::Picker(PickerMsg::Open { index, split });
    let row_look = RowLook {
        fg: look.fg,
        dim: look.dim,
        selected: Color {
            a: 0.35,
            ..look.accent
        },
    };
    let events = RowEvents {
        press: Some(open(false)),
        double: None,
        middle: Some(open(true)),
        right: None,
    };
    row![
        fuzzy_list::row_view(entry, selected, row_look, events),
        crate::sftp::panel::flat("Split", look, Some(open(true))),
    ]
    .spacing(6)
    .align_y(Alignment::Center)
    .into()
}

/// The picker panel over `base`; a click outside closes it.
pub(crate) fn overlay<'a>(
    base: Element<'a, Message>,
    picker: Option<&'a ProfilePicker>,
    rows: &[FuzzyRow],
    look: Look,
) -> Element<'a, Message> {
    let Some(picker) = picker else { return base };
    let shown = fuzzy_list::rank(&picker.filter, rows);
    let mut list = column![].spacing(2);
    for (pos, &index) in shown.iter().enumerate() {
        if let Some(entry) = rows.get(index) {
            list = list.push(entry_row(index, entry, pos == picker.selected, look));
        }
    }
    let filter = text_input("Filter profiles", &picker.filter)
        .id(Id::new(PICKER_INPUT))
        .on_input(|f| Message::Picker(PickerMsg::Filter(f)))
        .on_submit(Message::Picker(PickerMsg::Submit))
        .size(14)
        .padding(8);
    let scroller = Scrollbar::new().width(6).scroller_width(6);
    // Room for the scrollbar right of the "Split" buttons.
    let list = scrollable(list.padding(iced::Padding::ZERO.right(10)))
        .direction(Direction::Vertical(scroller))
        .height(Length::Shrink);
    let panel = container(column![filter, list].spacing(8))
        .padding(8)
        .width(PANEL_W)
        .max_height(480)
        .style(move |_theme: &iced::Theme| crate::context_menu::panel_style(look));
    let dim = Color {
        a: 0.35,
        ..Color::BLACK
    };
    let click_away = mouse_area(
        container(space())
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_theme: &iced::Theme| container::background(dim)),
    )
    .on_press(Message::Picker(PickerMsg::Close));
    let placed = container(opaque(panel))
        .padding([48, 0])
        .center_x(Length::Fill);
    stack![base, opaque(click_away), placed].into()
}
