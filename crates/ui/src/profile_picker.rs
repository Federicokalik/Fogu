//! Profile picker: every profile (the built-in default shell first) in a filterable
//! list; Enter / click opens it in a new tab, "Split" next to the focused pane. Substring
//! filter only (no fuzzy search yet).

use fogu_core::{Config, EffectiveProfile, Profile, Side, resolve};
use fogu_term::sanitize_title;
use iced::widget::{
    Id, button, column, container, mouse_area, opaque, operation, row, scrollable, space, stack,
    text, text_input,
};
use iced::{Alignment, Color, Element, Length, Task};

use crate::app::App;
use crate::context_menu::to_iced;
use crate::message::Message;

/// Id of the filter field.
pub const PICKER_INPUT: &str = "fogu-picker-input";
const PANEL_W: f32 = 440.0;

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

/// One pickable profile and its second line.
pub(crate) struct Entry {
    pub profile: Profile,
    pub subtitle: String,
}

/// The built-in default shell, then every configured profile, in config order.
pub(crate) fn entries(config: &Config) -> Vec<Entry> {
    std::iter::once(Profile::default_shell())
        .chain(config.profiles.iter().cloned())
        .map(|profile| {
            let subtitle = match resolve(&profile, &config.profile_defaults) {
                EffectiveProfile::Ssh(ssh) if ssh.port == 22 => {
                    format!("{}@{}", ssh.user, ssh.host)
                }
                EffectiveProfile::Ssh(ssh) => format!("{}@{}:{}", ssh.user, ssh.host, ssh.port),
                EffectiveProfile::Local(local) => {
                    local.program.unwrap_or_else(|| "local shell".into())
                }
            };
            let subtitle = sanitize_title(&subtitle);
            Entry { profile, subtitle }
        })
        .collect()
}

/// Indices of the entries whose name or subtitle contains `filter`, ignoring case.
pub(crate) fn matching(entries: &[Entry], filter: &str) -> Vec<usize> {
    let filter = filter.trim().to_lowercase();
    (0..entries.len())
        .filter(|&i| {
            entries.get(i).is_some_and(|e| {
                e.profile.name.to_lowercase().contains(&filter)
                    || e.subtitle.to_lowercase().contains(&filter)
            })
        })
        .collect()
}

impl App {
    /// `OpenProfilePicker` / right-click or `▾` on the tab bar's `+`.
    pub(crate) fn open_picker(&mut self) -> Task<Message> {
        self.menu = None;
        self.picker = Some(ProfilePicker::default());
        operation::focus(Id::new(PICKER_INPUT))
    }

    pub(crate) fn on_picker(&mut self, message: PickerMsg) -> Task<Message> {
        let entries = entries(&self.config);
        let Some(picker) = self.picker.as_mut() else {
            return Task::none();
        };
        let shown = matching(&entries, &picker.filter);
        let open = match message {
            PickerMsg::Filter(filter) => {
                picker.filter = filter;
                picker.selected = 0;
                None
            }
            PickerMsg::Move(delta) => {
                let last = shown.len().saturating_sub(1);
                picker.selected = picker
                    .selected
                    .saturating_add_signed(delta.into())
                    .min(last);
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
        match self.active_pane().map(|p| p.id) {
            Some(target) if split => self.split_now(target, Side::Right, None, Some(profile)),
            _ => {
                self.open_tab(&profile, None);
            }
        }
        Task::none()
    }
}

/// One list row: colour chip, name, subtitle, "Split".
fn entry_row(index: usize, entry: &Entry, selected: bool) -> Element<'static, Message> {
    let chip = entry.profile.color.map_or(Color::TRANSPARENT, to_iced);
    let chip = container(space())
        .width(4)
        .height(30)
        .style(move |_theme: &iced::Theme| container::background(chip));
    let label = column![
        text(entry.profile.name.clone()),
        text(entry.subtitle.clone()).size(12)
    ];
    let open = |split| Message::Picker(PickerMsg::Open { index, split });
    let main = button(row![chip, label].spacing(8).align_y(Alignment::Center))
        .width(Length::Fill)
        .style(if selected {
            button::primary
        } else {
            button::text
        })
        .on_press(open(false));
    let split = button(text("Split").size(12))
        .style(button::secondary)
        .on_press(open(true));
    row![main, split]
        .spacing(6)
        .align_y(Alignment::Center)
        .into()
}

/// The picker panel over `base`; a click outside closes it.
pub(crate) fn overlay<'a>(
    base: Element<'a, Message>,
    picker: Option<&'a ProfilePicker>,
    config: &Config,
) -> Element<'a, Message> {
    let Some(picker) = picker else { return base };
    let entries = entries(config);
    let shown = matching(&entries, &picker.filter);
    let mut list = column![].spacing(2);
    for (pos, &index) in shown.iter().enumerate() {
        if let Some(entry) = entries.get(index) {
            list = list.push(entry_row(index, entry, pos == picker.selected));
        }
    }
    let filter = text_input("Filter profiles", &picker.filter)
        .id(Id::new(PICKER_INPUT))
        .on_input(|f| Message::Picker(PickerMsg::Filter(f)))
        .on_submit(Message::Picker(PickerMsg::Submit))
        .padding(6);
    let panel = container(column![filter, scrollable(list).height(Length::Shrink)].spacing(8))
        .padding(8)
        .width(PANEL_W)
        .max_height(420)
        .style(container::bordered_box);
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
        .padding([40, 0])
        .center_x(Length::Fill);
    stack![base, opaque(click_away), placed].into()
}

#[cfg(test)]
mod tests {
    #![expect(clippy::unwrap_used, reason = "tests")]

    use fogu_core::{ProfileId, ProfileKind, SshOpts};

    use super::*;

    fn ssh(name: &str, host: &str, port: Option<u16>) -> Profile {
        Profile {
            id: ProfileId(name.to_owned()),
            name: name.to_owned(),
            kind: ProfileKind::Ssh(SshOpts {
                host: Some(host.to_owned()),
                port,
                user: Some("alice".to_owned()),
                ..SshOpts::default()
            }),
            ..Profile::default_shell()
        }
    }

    #[test]
    fn lists_default_shell_first_with_user_at_host_subtitles() {
        let config = Config {
            profiles: vec![
                ssh("Work", "work.lan", None),
                ssh("Pi", "10.0.0.7", Some(2222)),
            ],
            ..Config::default()
        };
        let entries = entries(&config);
        let subtitles: Vec<&str> = entries.iter().map(|e| e.subtitle.as_str()).collect();
        assert_eq!(
            subtitles,
            ["local shell", "alice@work.lan", "alice@10.0.0.7:2222"]
        );
        assert_eq!(entries.first().unwrap().profile.name, "Default shell");
    }

    #[test]
    fn filter_is_a_case_insensitive_substring_of_name_or_subtitle() {
        let config = Config {
            profiles: vec![
                ssh("Work", "work.lan", None),
                ssh("Pi", "10.0.0.7", Some(2222)),
            ],
            ..Config::default()
        };
        let entries = entries(&config);
        // (filter, expected entry indices)
        let rows: [(&str, &[usize]); 6] = [
            ("", &[0, 1, 2]),
            ("WORK", &[1]),
            ("10.0", &[2]),
            ("alice@", &[1, 2]),
            ("shell", &[0]),
            ("nope", &[]),
        ];
        for (filter, want) in rows {
            assert_eq!(matching(&entries, filter), want, "{filter:?}");
        }
    }
}
