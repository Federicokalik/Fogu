//! The "Port forwarding" dialog of an SSH pane: its forwards with state and connection count,
//! Remove, and an "Add" form built with the generic form helpers (with an explicit "Save to
//! profile" toggle that appends the forward to the stored profile's `forwarded_ports`). Opened from
//! the tab menu and the tab bar's "Ports" button; the session does the work and reports
//! `ForwardStatus`, kept per pane in `Pane::forwards`.

use fogu_core::config::ProfileEdit;
use fogu_core::{ForwardKind, PortForward, ProfileKind};
use fogu_session::{
    ForwardCmd, ForwardId, ForwardState, ForwardStatus, SessionInput, sanitize_remote,
};
use iced::widget::{Column, button, column, container, row, scrollable, text};
use iced::{Alignment, Element, Length, Task};

use crate::app::App;
use crate::forms::{self, ERROR, Field, FieldValue, WARN};
use crate::message::Message;
use crate::notifications::{Level, Notification};
use crate::sftp::panel::Look;
use crate::tab::{Pane, PaneId};

const KINDS: [(&str, ForwardKind); 3] = [
    ("Local (-L)", ForwardKind::Local),
    ("Remote (-R)", ForwardKind::Remote),
    ("Dynamic SOCKS5 (-D)", ForwardKind::Dynamic),
];
/// Longest forward label / error shown, in characters.
const SHOWN_MAX_CHARS: usize = 120;

/// The open dialog: which pane, and the "Add" form.
#[derive(Debug)]
pub struct ForwardsDialog {
    pub pane: PaneId,
    pub draft: PortForward,
    /// Port fields as typed (parsed on Add).
    pub port: String,
    pub target_port: String,
    pub save: bool,
    /// Why the last Add was refused.
    pub error: Option<String>,
}

/// The form's fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FwdField {
    Kind,
    Host,
    Port,
    Target,
    TargetPort,
    Description,
    Save,
}

/// Dialog input.
#[derive(Clone, Debug)]
pub enum ForwardsMsg {
    /// Open for the active tab's SSH pane.
    Open,
    Close,
    Field(Field<FwdField>),
    Add,
    Remove(ForwardId),
}

/// Applies a status report to a pane's list: `Stopped` removes the forward.
pub fn apply_status(list: &mut Vec<ForwardStatus>, status: ForwardStatus) {
    let at = list.iter().position(|s| s.id == status.id);
    match (at, status.state == ForwardState::Stopped) {
        (Some(at), true) => {
            list.remove(at);
        }
        (Some(at), false) => {
            if let Some(slot) = list.get_mut(at) {
                *slot = status;
            }
        }
        (None, false) => list.push(status),
        (None, true) => {}
    }
}

/// Forwards of the given SSH panes that are listening (the tab bar badge, the close
/// confirmation), each connection's forwards counted once (panes can share one).
pub fn listening<'a>(panes: impl Iterator<Item = &'a Pane>) -> usize {
    let mut seen = Vec::new();
    for pane in panes {
        for s in pane
            .forwards
            .iter()
            .filter(|s| s.state == ForwardState::Listening)
        {
            let key = (pane.connection.ok_or(pane.id), s.id);
            if !seen.contains(&key) {
                seen.push(key);
            }
        }
    }
    seen.len()
}

/// The form's draft as a forward, or why it isn't one.
fn parse(d: &ForwardsDialog) -> Result<PortForward, String> {
    let port = |text: &str, what: &str| {
        text.trim()
            .parse::<u16>()
            .map_err(|_| format!("the {what} must be a number 1–65535"))
    };
    let mut forward = d.draft.clone();
    forward.port = port(&d.port, "listen port")?;
    forward.target_port = match forward.kind {
        ForwardKind::Dynamic => 0,
        _ => port(&d.target_port, "target port")?,
    };
    if forward.kind == ForwardKind::Dynamic {
        forward.target_address.clear();
    }
    match forward.problem() {
        Some(problem) => Err(problem.to_owned()),
        None => Ok(forward),
    }
}

impl App {
    pub(crate) fn on_forwards(&mut self, message: ForwardsMsg) -> Task<Message> {
        match message {
            ForwardsMsg::Open => {
                self.menu = None;
                let tab = self.active_tab();
                let pane = tab
                    .and_then(|t| t.focused_pane())
                    .filter(|p| p.is_ssh())
                    .or_else(|| tab.and_then(|t| t.panes.values().find(|p| p.is_ssh())));
                self.forwards = pane.map(|p| ForwardsDialog {
                    pane: p.id,
                    draft: PortForward::default(),
                    port: String::new(),
                    target_port: String::new(),
                    save: false,
                    error: None,
                });
            }
            ForwardsMsg::Close => self.forwards = None,
            ForwardsMsg::Field(Field(field, value)) => {
                if let Some(d) = self.forwards.as_mut() {
                    edit(d, field, value);
                }
            }
            ForwardsMsg::Add => return self.add_forward(),
            ForwardsMsg::Remove(id) => {
                let pane = self.forwards.as_ref().map(|d| d.pane);
                if let Some(pane) = pane.and_then(|p| self.pane(p)) {
                    pane.control(SessionInput::Forward(ForwardCmd::Remove(id)));
                }
            }
        }
        Task::none()
    }

    fn add_forward(&mut self) -> Task<Message> {
        let Some(d) = self.forwards.as_mut() else {
            return Task::none();
        };
        let forward = match parse(d) {
            Ok(forward) => forward,
            Err(why) => {
                d.error = Some(why);
                return Task::none();
            }
        };
        let (pane_id, save) = (d.pane, d.save);
        let sent = self.pane(pane_id).is_some_and(|p| {
            p.state.is_running()
                && p.control(SessionInput::Forward(ForwardCmd::Add(forward.clone())))
        });
        let Some(d) = self.forwards.as_mut() else {
            return Task::none();
        };
        if !sent {
            d.error = Some("the session is not running".to_owned());
            return Task::none();
        }
        d.error = None;
        d.port.clear();
        d.target_port.clear();
        d.draft = PortForward {
            kind: d.draft.kind,
            ..PortForward::default()
        };
        if save {
            return self.save_forward(pane_id, forward);
        }
        Task::none()
    }

    /// Appends `forward` to the pane's stored profile (`forwarded_ports`, in place).
    fn save_forward(&mut self, pane: PaneId, forward: PortForward) -> Task<Message> {
        let Some(id) = self.pane(pane).map(|p| p.profile.id.clone()) else {
            return Task::none();
        };
        let Some(old) = self.config.profiles.iter().find(|p| p.id == id).cloned() else {
            return Task::none();
        };
        let mut new = old.clone();
        if let ProfileKind::Ssh(ssh) = &mut new.kind {
            let inherited = &self.config.profile_defaults.ssh.forwarded_ports;
            let list = ssh
                .forwarded_ports
                .get_or_insert_with(|| inherited.clone().unwrap_or_default());
            list.push(forward);
        }
        let name = new.name.clone();
        let edit = ProfileEdit::Update {
            old: Box::new(old),
            new: Box::new(new.clone()),
        };
        if let Err(task) = self.write_edits(&[edit]) {
            return task;
        }
        // Panes of this profile restart/reconnect with the saved list (like the editor's Save).
        for pane in self.tabs.iter_mut().flat_map(|t| t.panes.values_mut()) {
            if pane.profile.id == id {
                pane.profile = new.clone();
            }
        }
        self.notify(Notification::new(Level::Info, "Forward saved to the profile").detail(name))
    }
}

fn edit(d: &mut ForwardsDialog, field: FwdField, value: FieldValue) {
    d.error = None;
    match (field, value) {
        (FwdField::Kind, FieldValue::Choice(i)) => {
            d.draft.kind = KINDS.get(i).map_or(ForwardKind::Local, |k| k.1);
        }
        (FwdField::Host, FieldValue::Text(t)) => d.draft.host = t,
        (FwdField::Port, FieldValue::Text(t)) => d.port = t,
        (FwdField::Target, FieldValue::Text(t)) => d.draft.target_address = t,
        (FwdField::TargetPort, FieldValue::Text(t)) => d.target_port = t,
        (FwdField::Description, FieldValue::Text(t)) => d.draft.description = t,
        (FwdField::Save, FieldValue::Bool(b)) => d.save = b,
        _ => {}
    }
}

/// The dialog for `pane`, if it is open and the pane still exists.
pub fn view<'a>(
    d: &'a ForwardsDialog,
    pane: &'a Pane,
    stored: bool,
    look: Look,
) -> Element<'a, Message> {
    let mut list = Column::new().spacing(6);
    if pane.forwards.is_empty() {
        list = list.push(text("No forwards.").size(13).color(look.dim));
    }
    for status in &pane.forwards {
        list = list.push(forward_row(status, look));
    }
    let content = column![
        text("Port forwarding").size(18).color(look.fg),
        scrollable(list).height(Length::Shrink),
        text("Add a forward").size(15).color(look.fg),
        form(d, stored, look),
        row![
            button(text("Add")).on_press(Message::Forwards(ForwardsMsg::Add)),
            button(text("Close"))
                .style(button::secondary)
                .on_press(Message::Forwards(ForwardsMsg::Close)),
        ]
        .spacing(8),
    ]
    .spacing(12);
    container(content)
        .padding(16)
        .max_width(640)
        .style(move |_theme: &iced::Theme| {
            container::background(look.bg).border(iced::Border {
                color: look.dim,
                width: 1.0,
                radius: 6.0.into(),
            })
        })
        .into()
}

fn forward_row(status: &ForwardStatus, look: Look) -> Element<'_, Message> {
    let mut label = sanitize_remote(&status.forward.label(), SHOWN_MAX_CHARS, false);
    if !status.forward.description.is_empty() {
        label.push_str(" — ");
        label.push_str(&sanitize_remote(
            &status.forward.description,
            SHOWN_MAX_CHARS,
            false,
        ));
    }
    let (state, color) = match &status.state {
        ForwardState::Listening => {
            let n = status.active_connections;
            let s = if n == 1 { "" } else { "s" };
            (format!("Listening · {n} connection{s}"), look.accent)
        }
        ForwardState::Error(why) => (sanitize_remote(why, SHOWN_MAX_CHARS, false), ERROR),
        ForwardState::Stopped => ("Stopped".to_owned(), look.dim),
    };
    let mut details = column![
        text(label).size(13).color(look.fg),
        text(state).size(12).color(color)
    ];
    if status.forward.is_exposed() {
        details = details.push(
            text("Reachable from other machines (not a loopback address)")
                .size(12)
                .color(WARN),
        );
    }
    row![
        details.width(Length::Fill),
        button(text("Remove").size(13))
            .style(button::secondary)
            .on_press(Message::Forwards(ForwardsMsg::Remove(status.id))),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .into()
}

fn form<'a>(d: &'a ForwardsDialog, stored: bool, look: Look) -> Element<'a, Message> {
    let kind = KINDS.iter().position(|k| k.1 == d.draft.kind).unwrap_or(0);
    let labels = KINDS.iter().map(|k| k.0.to_owned()).collect();
    let listen = if d.draft.kind == ForwardKind::Remote {
        "Listen address (server)"
    } else {
        "Listen address"
    };
    let mut rows: Vec<Element<'a, Field<FwdField>>> = vec![
        forms::choice(look, FwdField::Kind, "Type", labels, kind),
        forms::text_field(look, FwdField::Host, listen, &d.draft.host, "127.0.0.1"),
        forms::number(look, FwdField::Port, "Listen port", &d.port, "8080"),
    ];
    if d.draft.kind != ForwardKind::Dynamic {
        let (target, hint) = if d.draft.kind == ForwardKind::Local {
            ("Target (from the server)", "127.0.0.1")
        } else {
            ("Target (from this computer)", "127.0.0.1")
        };
        rows.push(forms::text_field(
            look,
            FwdField::Target,
            target,
            &d.draft.target_address,
            hint,
        ));
        rows.push(forms::number(
            look,
            FwdField::TargetPort,
            "Target port",
            &d.target_port,
            "80",
        ));
    }
    rows.push(forms::text_field(
        look,
        FwdField::Description,
        "Description",
        &d.draft.description,
        "",
    ));
    let save = forms::toggle(look, FwdField::Save, "Save to profile", d.save && stored);
    let save_note = (!stored).then_some((
        "Only stored profiles can be changed (save this connection as a profile first).",
        look.dim,
    ));
    rows.push(forms::note_maybe(save, save_note));
    let exposed = d.draft.is_exposed().then_some((
        "Not a loopback address: other machines will be able to connect.",
        WARN,
    ));
    let error = d.error.as_deref().map(|e| (e, ERROR));
    let mut fields = Column::with_children(rows).spacing(6);
    for (line, color) in exposed.into_iter().chain(error) {
        fields = fields.push(text(line.to_owned()).size(12).color(color));
    }
    Element::from(fields).map(|f| Message::Forwards(ForwardsMsg::Field(f)))
}

#[cfg(test)]
#[path = "forwards_dialog_tests.rs"]
mod tests;
