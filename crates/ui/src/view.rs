//! `App::view` (tab bar, the active tab's split container with the SFTP side panel, toasts,
//! modals and the context menu) and `App::subscription` (session/SFTP events, window and
//! keyboard events).

use iced::keyboard::{self, key::Named};
use iced::widget::column;
use iced::{Element, Subscription, event, window};

use crate::app::App;
use crate::context_menu;
use crate::message::Message;
use crate::modal::{self, overlay};
use crate::mouse::MouseConfig;
use crate::palette::to_iced;
use crate::profile_picker;
use crate::prompt;
use crate::sftp;
use crate::split_container::SplitContainer;
use crate::subscription::{session_events, sftp_events};
use crate::tab_bar::TabBar;
use crate::terminal_view::TerminalView;

impl App {
    /// Tab bar above the active tab's split container, plus modal / menu overlays.
    pub fn view(&self) -> Element<'_, Message> {
        let Some(tab) = self.active_tab() else {
            return column![].into();
        };
        let typing_elsewhere = self.renaming.is_some()
            || self.menu.is_some()
            || self.picker.is_some()
            || self.pending_close.is_some();
        let sftp_panel = tab.sftp.as_ref().filter(|p| p.visible);
        let panel_has_keys = sftp_panel.is_some_and(|p| p.focused);
        let mouse = MouseConfig {
            wheel_lines: self.config.terminal.wheel_lines,
            paste_on_middle_click: self.config.terminal.paste_on_middle_click,
        };
        let (backdrop, terminal_alpha) = self.backdrop(tab);
        let panes = tab
            .layout
            .leaves()
            .into_iter()
            .filter_map(|id| tab.panes.get(id))
            .map(|pane| {
                let id = pane.id;
                let focused = id == tab.focused
                    && self.focused
                    && !typing_elsewhere
                    && !panel_has_keys
                    && pane.prompt.is_none();
                let view: Element<'_, Message> = TerminalView::new(
                    id,
                    &pane.snapshot,
                    &self.colors,
                    self.metrics,
                    self.resolved.font,
                    focused,
                    mouse,
                )
                .status(pane.status.as_deref())
                .hotkeys(&self.config.hotkeys)
                .cursor_on(self.blink.on)
                .default_bg_alpha(terminal_alpha)
                .into();
                let view = view.map(move |message| message.for_pane(id));
                // A prompt is modal over the pane that asked, not over the whole window.
                match &pane.prompt {
                    Some(state) => (id, modal::over(view, prompt::view(id, state))),
                    None => (id, view),
                }
            })
            .collect();
        let mut divider = to_iced(self.colors.foreground());
        divider.a = 0.3;
        let mut split = SplitContainer::new(
            tab.id,
            &tab.layout,
            panes,
            to_iced(self.colors.background()),
            divider,
        )
        .backdrop(backdrop);
        // Focused pane border: the tab colour, else the accent once there is more than one pane.
        let border = tab
            .color
            .map(context_menu::to_iced)
            .or_else(|| (tab.panes.len() > 1).then(|| to_iced(self.colors.cursor())));
        if let Some(color) = border {
            split = split.border(tab.focused, color);
        }
        if let Some(panel) = sftp_panel {
            split = split.side(sftp::panel::view(tab.id, panel, &self.colors), panel.width);
        }
        let bar = TabBar {
            tabs: &self.tabs,
            active: self.active,
            hovered: self.hovered_tab,
            renaming: self.renaming.as_ref(),
            colors: &self.colors,
            sftp: tab
                .panes
                .values()
                .any(|p| p.sftp.is_some())
                .then_some(sftp_panel.is_some()),
        }
        .view();
        // Toasts are layered over the panes only, never over the tab bar.
        let mut panes: Element<'_, Message> = split.into();
        if let Some(toasts) = self.toasts.view(self.metrics.cell_h) {
            panes = iced::widget::stack![panes, toasts].into();
        }
        let modal = self.paste_modal().or_else(|| self.close_modal());
        let base = overlay(column![bar, panes].into(), modal);
        let base = match sftp_panel.and_then(|p| sftp::dialog(tab.id, p)) {
            Some(dialog) => modal::over(base, dialog),
            None => base,
        };
        let base = profile_picker::overlay(base, self.picker.as_ref(), &self.config);
        context_menu::overlay(base, self.menu.as_ref(), self.window_size)
    }

    /// Every pane's session events plus window focus/scale/size changes and Escape.
    pub fn subscription(&self) -> Subscription<Message> {
        let sessions = self
            .tabs
            .iter()
            .flat_map(|tab| tab.panes.values())
            .flat_map(|pane| {
                let sftp = pane.sftp.as_ref().map(|link| sftp_events(&link.events));
                std::iter::once(session_events(&pane.events)).chain(sftp)
            });
        let window_events = event::listen_with(|event, status, _window| match event {
            iced::Event::Window(window::Event::Focused) => Some(Message::Focused(true)),
            iced::Event::Window(window::Event::Unfocused) => Some(Message::Focused(false)),
            iced::Event::Window(window::Event::Rescaled(scale)) => {
                Some(Message::ScaleFactor(scale))
            }
            iced::Event::Window(window::Event::Resized(size)) => Some(Message::WindowResized(size)),
            iced::Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(Named::Escape),
                ..
            }) => Some(Message::Escape),
            // Keys no widget took: SFTP panel keys, dialog defaults and picker navigation.
            iced::Event::Keyboard(keyboard::Event::KeyPressed {
                key,
                modified_key,
                modifiers,
                ..
            }) if status == event::Status::Ignored => {
                Some(Message::KeyIgnored(key, modified_key, modifiers))
            }
            iced::Event::Keyboard(keyboard::Event::ModifiersChanged(mods)) => {
                Some(Message::Modifiers(mods))
            }
            iced::Event::Mouse(iced::mouse::Event::ButtonPressed(_)) => Some(Message::MousePressed),
            // X11 only (winit 0.30); one event per file.
            iced::Event::Window(window::Event::FileDropped(path)) => {
                Some(Message::Desktop(sftp::DesktopMsg::Dropped(path)))
            }
            _ => None,
        });
        let frame = self
            .exit_after_first_frame
            .then(|| window::frames().map(|_| Message::Frame));
        Subscription::batch(sessions.chain([window_events]).chain(frame))
    }
}
