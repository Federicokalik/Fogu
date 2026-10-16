//! Keyboard and pointer focus of the SFTP panel: after a click inside it, keys no widget took
//! are its hotkeys and navigation (Up/Down, Enter, Backspace, type-to-filter).

use iced::Task;
use iced::keyboard::{Key, Modifiers, key::Named};
use iced::widget::operation;

use super::PanelMsg;
use super::panel::{FILTER_INPUT, LIST_ID, ROW_H, rows_px};
use crate::app::App;
use crate::message::Message;

impl App {
    /// Whether keys nobody else took go to the active tab's panel.
    pub(crate) fn panel_has_keys(&self) -> bool {
        let Some(tab) = self.active_tab() else {
            return false;
        };
        let panel_focused = tab
            .sftp
            .as_ref()
            .is_some_and(|p| p.visible && p.focused && p.conflicts.current().is_none());
        panel_focused
            && self.picker.is_none()
            && self.menu.is_none()
            && self.renaming.is_none()
            && self.pending_close.is_none()
            && self.pending_paste.is_none()
            && tab.panes.values().all(|p| p.prompt.is_none())
    }

    /// A mouse press anywhere: the panel takes the keyboard when the pointer is over it.
    pub(crate) fn on_mouse_pressed(&mut self) {
        let active = self.active;
        if let Some(panel) = self
            .tabs
            .iter_mut()
            .find(|t| t.id == active)
            .and_then(|t| t.sftp.as_mut())
        {
            panel.focused = panel.visible && panel.hovered;
        }
    }

    /// A key no widget took: the panel's hotkeys/navigation when it has the keyboard, else
    /// dialog defaults and picker navigation.
    pub(crate) fn on_key_ignored(
        &mut self,
        key: &Key,
        modified: &Key,
        mods: Modifiers,
    ) -> Task<Message> {
        if self.panel_has_keys() {
            if let Some(action) = crate::keys::hotkey(&self.config.hotkeys, key, modified, mods) {
                return self.on_action(action);
            }
            return self.on_panel_key(modified, mods);
        }
        match key {
            Key::Named(Named::Enter) => self.on_enter(),
            Key::Named(Named::ArrowUp) => {
                self.on_picker(crate::profile_picker::PickerMsg::Move(-1))
            }
            Key::Named(Named::ArrowDown) => {
                self.on_picker(crate::profile_picker::PickerMsg::Move(1))
            }
            _ => Task::none(),
        }
    }

    /// ↑/↓ move, Enter opens, Backspace goes up, printable text starts filtering.
    fn on_panel_key(&mut self, key: &Key, mods: Modifiers) -> Task<Message> {
        let tab = self.active;
        let Some((panel, _)) = self.panel_of(tab) else {
            return Task::none();
        };
        match key {
            Key::Named(Named::ArrowUp | Named::ArrowDown) => {
                let delta = if *key == Key::Named(Named::ArrowUp) {
                    -1
                } else {
                    1
                };
                let Some(row) = panel.listing.move_cursor(delta) else {
                    return Task::none();
                };
                let top = rows_px(row);
                let view = panel.view_h.max(ROW_H);
                let target = if top < panel.scroll_y {
                    top
                } else if top + ROW_H > panel.scroll_y + view {
                    top + ROW_H - view
                } else {
                    return Task::none();
                };
                operation::scroll_to(
                    LIST_ID,
                    iced::widget::scrollable::AbsoluteOffset {
                        x: None,
                        y: Some(target),
                    },
                )
            }
            Key::Named(Named::Enter) => match panel.listing.cursor.clone() {
                Some(name) => self.on_panel(tab, PanelMsg::Open(name)),
                None => Task::none(),
            },
            Key::Named(Named::Backspace) => self.on_panel(tab, PanelMsg::Up),
            Key::Character(c) if !mods.control() && !mods.alt() && !mods.logo() => {
                panel.listing.filter.push_str(c);
                Task::batch([
                    operation::focus(FILTER_INPUT),
                    operation::move_cursor_to_end(FILTER_INPUT),
                ])
            }
            _ => Task::none(),
        }
    }
}
