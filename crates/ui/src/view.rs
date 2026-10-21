//! `App::view` (tab bar, the active tab's split container with the SFTP side panel, toasts,
//! modals and the context menu) and `App::subscription` (session/SFTP events, window and
//! keyboard events).

use iced::keyboard::{self, key::Named};
use iced::widget::{column, container};
use iced::{Element, Subscription, event, window};

use crate::app::App;
use crate::context_menu;
use crate::drag::{DragMsg, Payload};
use crate::message::Message;
use crate::modal;
use crate::mouse::MouseConfig;
use crate::palette::to_iced;
use crate::profile_picker;
use crate::profiles;
use crate::prompt;
use crate::sftp;
use crate::split_container::SplitContainer;
use crate::subscription::{session_events, sftp_events};
use crate::tab_bar::TabBar;
use crate::terminal_view::TerminalView;

impl App {
    /// Tab bar above the active tab's split container, plus modal / menu overlays.
    #[expect(
        clippy::too_many_lines,
        reason = "terminal layout and its overlays share one visual tree"
    )]
    pub fn view(&self) -> Element<'_, Message> {
        let Some(tab) = self.active_tab() else {
            return column![].into();
        };
        if let Some(editor) = &tab.editor {
            let look = self.look;
            let page = crate::profile_editor::page(tab.id, editor, &self.config, look);
            return self.chrome(page, None, None);
        }
        let search = self.search_in(tab);
        let typing_elsewhere = self.settings.is_some()
            || self.renaming.is_some()
            || search.is_some()
            || self.menu.is_some()
            || self.picker.is_some()
            || self.pending_close.is_some()
            || self.forwards.is_some()
            || self.profiles.pending_delete.is_some()
            || (self.profiles.open && self.profiles.focused);
        let sftp_panel = tab.sftp.as_ref().filter(|p| p.visible);
        let panel_has_keys = sftp_panel.is_some_and(|p| p.focused);
        let mouse = MouseConfig {
            wheel_lines: self.config.terminal.wheel_lines,
            paste_on_middle_click: self.config.terminal.paste_on_middle_click,
            right_click: self.config.terminal.right_click,
        };
        let (backdrop, terminal_alpha) = self.backdrop(tab);
        let layout = tab.solo.as_ref().unwrap_or(&tab.layout);
        let grips = layout.leaves().len() > 1;
        let panes = layout
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
                .flash(pane.flash)
                .default_bg_alpha(terminal_alpha)
                .into();
                let view = view.map(move |message| message.for_pane(id));
                let view = if grips {
                    column![self.pane_grip(id), view].into()
                } else {
                    view
                };
                // A prompt is modal over the pane that asked, not over the whole window.
                match &pane.prompt {
                    Some(state) => (
                        id,
                        modal::over(view, prompt::view(id, state, self.look), self.look),
                    ),
                    None => (id, view),
                }
            })
            .collect();
        let divider = self.look.divider;
        let mut split = SplitContainer::new(
            tab.id,
            layout,
            panes,
            to_iced(self.colors.background()),
            divider,
        )
        .backdrop(backdrop);
        if let Some(drag) = self.drag.as_ref().filter(|d| d.started) {
            split = split.dropping(drag.target, self.look.accent);
        }
        // Focused pane border: the tab colour, else the accent once there is more than one pane.
        let border = tab
            .color
            .map(context_menu::to_iced)
            .or_else(|| (tab.panes.len() > 1).then(|| to_iced(self.colors.cursor())));
        if let Some(color) = border {
            split = split.border(tab.focused, color);
        }
        // SFTP opens as a large modal over the workspace; it can become a dock later.
        let look = self.look;
        let sftp_modal = sftp_panel.map(|panel| {
            let width = (self.window_size.width * 0.88).clamp(640.0, 1180.0);
            let height = (self.window_size.height * 0.84).clamp(420.0, 860.0);
            let card = container(sftp::panel::view(tab.id, panel, look))
                .width(width)
                .height(height)
                .style(move |_theme: &iced::Theme| crate::controls::floating(look));
            (tab.id, card.into())
        });
        let dialog = sftp_panel.and_then(|p| sftp::dialog(tab.id, p, look));
        self.chrome(
            crate::search::over(split.into(), search, self.look),
            sftp_modal,
            dialog,
        )
    }

    /// Room before the tab headers: the native window buttons on macOS (hidden in full screen).
    pub(crate) fn bar_inset(&self) -> f32 {
        if cfg!(target_os = "macos") && !self.fullscreen {
            crate::tab_bar::TRAFFIC_LIGHTS_W
        } else {
            0.0
        }
    }

    /// The tab bar above `body` (the panes or an editor page), toasts, the profile sidebar and
    /// the overlays.
    fn chrome<'a>(
        &'a self,
        body: Element<'a, Message>,
        sftp_modal: Option<(crate::tab::TabId, Element<'a, Message>)>,
        dialog: Option<Element<'a, Message>>,
    ) -> Element<'a, Message> {
        let (sftp, forwards) = self.bar_flags();
        let widths = self.tab_widths();
        let margin = self.island_margin();
        let bar = TabBar {
            tabs: &self.tabs,
            active: self.active,
            hovered: self.hovered_tab,
            renaming: self.renaming.as_ref(),
            look: self.look,
            sftp,
            forwards,
            profiles: self.profiles.open,
            inset: self.bar_inset(),
            widths,
            window: self.window_size,
            margin,
        }
        .view();
        // Toasts are layered over the panes only, never over the tab bar. The stack is always
        // there (an empty spacer without toasts): wrapping the panes only while a toast shows
        // would rebuild their widget state, and a prompt's text field would lose its focus when
        // the toast goes away.
        // With the settings page open the toasts are drawn over it instead (see `overlays`).
        let toasts = self
            .settings
            .is_none()
            .then(|| self.toasts.view(self.metrics.cell_h, self.look))
            .flatten()
            .unwrap_or_else(|| iced::widget::space().into());
        let panes: Element<'a, Message> = iced::widget::stack![body, toasts].into();
        let look = self.look;
        let islands = margin > 0.0;
        // Islands: the terminal is a rounded card, inset enough that its square cells never
        // cross the rounded corners; it never gets a shadow (it repaints on every keystroke).
        let panes = if islands {
            let inner = (look.shape.card * 0.3).ceil();
            container(panes)
                .padding(inner)
                .width(iced::Length::Fill)
                .height(iced::Length::Fill)
                .style(move |_theme: &iced::Theme| container::Style {
                    background: Some(look.base.into()),
                    border: iced::Border {
                        color: look.border,
                        width: 1.0,
                        radius: look.shape.card.into(),
                    },
                    ..container::Style::default()
                })
                .into()
        } else {
            panes
        };
        let panes = profiles::view::with_sidebar(panes, &self.profiles, &self.config, look);
        let gap = if islands { margin } else { 0.0 };
        let mut main = column![bar.top, panes].spacing(gap);
        if let Some(bottom) = bar.bottom {
            main = main.push(bottom);
        }
        let main: Element<'a, Message> = if islands {
            container(main)
                .padding(margin)
                .width(iced::Length::Fill)
                .height(iced::Length::Fill)
                .style(move |_theme: &iced::Theme| container::background(look.canvas))
                .into()
        } else {
            main.into()
        };
        self.overlays(main, sftp_modal, dialog)
    }

    /// Every layer over `base`, bottom to top: the SFTP panel, the settings page (and the
    /// toasts while it is open), a confirmation, a dialog, the profile picker, the context menu, the drag label and the
    /// window's resize edges. The stack always has the same children (an empty space for a
    /// layer that is not shown), so opening one never rebuilds the state of those below it: the
    /// settings page keeps its scroll and a field its focus while a confirmation comes and goes.
    fn overlays<'a>(
        &'a self,
        base: Element<'a, Message>,
        sftp_modal: Option<(crate::tab::TabId, Element<'a, Message>)>,
        sftp_dialog: Option<Element<'a, Message>>,
    ) -> Element<'a, Message> {
        let look = self.look;
        let sftp = sftp_modal.map(|(tab, panel)| {
            let close = Message::Sftp(sftp::SftpMsg::Panel(tab, sftp::PanelMsg::Close));
            modal::centered(panel, look, Some(close))
        });
        let settings = self.settings.as_ref().map(|page| {
            let width = (self.window_size.width * 0.9).clamp(860.0, 1200.0);
            let height = (self.window_size.height * 0.86).clamp(600.0, 900.0);
            let entries = if page.section == crate::settings::Section::Profiles {
                self.all_entries()
            } else {
                Vec::new()
            };
            let page = crate::settings::view(
                page,
                look,
                &self.config,
                (&self.schemes, &self.styles),
                (self.env, &entries),
            );
            let card = container(page)
                .width(width)
                .height(height)
                .style(move |_theme: &iced::Theme| crate::controls::floating(look));
            let outside = Message::Settings(crate::settings::SettingsMsg::Outside);
            modal::centered(card.into(), look, Some(outside))
        });
        // A save that failed must be seen while the settings page is open.
        let toasts = self
            .settings
            .as_ref()
            .and_then(|_| self.toasts.view(36.0, look));
        let confirm = self
            .paste_modal()
            .or_else(|| self.close_modal())
            .or_else(|| self.delete_modal())
            .or_else(|| self.clear_sessions_modal())
            .or_else(|| self.discard_modal())
            .or_else(|| self.settings_modal())
            .map(|m| modal::modal_layer(m, look));
        let dialog = self
            .secret_dialog
            .as_ref()
            .map(|dialog| crate::secret_dialog::view(dialog, look))
            .or(sftp_dialog)
            .or_else(|| self.forwards_view())
            .map(|d| modal::dialog_layer(d, look));
        let picker = self.picker.as_ref().map(|picker| {
            let rows = profile_picker::listing(self).1;
            profile_picker::layer(picker, &rows, look)
        });
        let menu = self
            .menu
            .as_ref()
            .map(|menu| context_menu::layer(menu, self.window_size, look));
        let ghost = self.drag_label().map(|(at, label)| {
            let ghost = container(iced::widget::text(label).size(13))
                .padding([4, 10])
                .style(move |_theme: &iced::Theme| context_menu::panel_style(look));
            container(ghost)
                .padding(iced::Padding::ZERO.left(at.x + 14.0).top(at.y + 14.0))
                .width(iced::Length::Fill)
                .height(iced::Length::Fill)
                .into()
        });
        // macOS: winit cannot start a resize from the app; the native frame resizes instead.
        let edges =
            (!self.fullscreen && !cfg!(target_os = "macos")).then(crate::tab_bar::resize_edges);
        let layers = [
            Some(base),
            sftp,
            settings,
            toasts,
            confirm,
            dialog,
            picker,
            menu,
            ghost,
            edges,
        ]
        .into_iter()
        .map(|layer| layer.unwrap_or_else(|| iced::widget::space().into()));
        iced::widget::Stack::with_children(layers).into()
    }

    /// The strip above a pane (tabs with several panes) that drags it to another place.
    fn pane_grip(&self, pane: crate::tab::PaneId) -> Element<'_, Message> {
        let look = self.look;
        let strip = container(iced::widget::space())
            .width(iced::Length::Fill)
            .height(look.layout.grip)
            .style(move |_theme: &iced::Theme| container::background(look.raised));
        iced::widget::mouse_area(strip)
            .interaction(iced::mouse::Interaction::Grab)
            .on_press(Message::Drag(DragMsg::Start(Payload::Pane(pane))))
            .into()
    }

    /// The port forwarding dialog, while its pane exists.
    fn forwards_view(&self) -> Option<Element<'_, Message>> {
        let dialog = self.forwards.as_ref()?;
        let pane = self.pane(dialog.pane)?;
        let stored = self.config.profiles.iter().any(|p| p.id == pane.profile.id);
        let look = self.look;
        Some(crate::forwards_dialog::view(dialog, pane, stored, look))
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
            iced::Event::Window(window::Event::CloseRequested) => Some(Message::CloseRequested),
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
            // Always listened to: a quick click releases before a drag-only subscription starts.
            iced::Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left)) => {
                Some(Message::Drag(DragMsg::Release))
            }
            // X11 and macOS (winit 0.30 has no Wayland drops); one event per file.
            iced::Event::Window(window::Event::FileDropped(path)) => {
                Some(Message::Desktop(sftp::DesktopMsg::Dropped(path)))
            }
            _ => None,
        });
        let frame = self
            .exit_after_first_frame
            .then(|| window::frames().map(|_| Message::Frame));
        // Pointer moves only reach `update` while a drag may be under way (releases always do).
        let drag = self.drag.is_some().then(|| {
            event::listen_with(|event, _status, _window| match event {
                iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }) => {
                    Some(Message::Drag(DragMsg::Move(position)))
                }
                _ => None,
            })
        });
        Subscription::batch(sessions.chain([window_events]).chain(frame).chain(drag))
    }
}
