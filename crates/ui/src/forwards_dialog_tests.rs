//! Reducer tests for the port forwarding dialog on a headless `App` with stub sessions.
#![expect(clippy::unwrap_used, clippy::panic, reason = "tests")]

use fogu_core::{ForwardKind, PortForward, ProfileKind};
use fogu_session::{
    ForwardCmd, ForwardId, ForwardState, ForwardStatus, SessionInput, SessionOutput,
};

use super::{ForwardsMsg, FwdField, listening};
use crate::app::App;
use crate::forms::{Field, FieldValue};
use crate::message::Message;
use crate::profiles::tests::{app_in, id, tmp};

fn send(app: &mut App, message: ForwardsMsg) {
    let _task = app.update(Message::Forwards(message));
}

fn set(app: &mut App, field: FwdField, value: FieldValue) {
    send(app, ForwardsMsg::Field(Field(field, value)));
}

fn text(app: &mut App, field: FwdField, value: &str) {
    set(app, field, FieldValue::Text(value.to_owned()));
}

/// The app with the stored SSH profile "box" open in the active tab and the dialog open.
fn box_app(dir: &std::path::Path) -> App {
    let mut app = app_in(dir);
    let profile = app
        .config
        .profiles
        .iter()
        .find(|p| p.id == id("box"))
        .cloned()
        .unwrap();
    app.open_tab(&profile, None);
    send(&mut app, ForwardsMsg::Open);
    assert!(app.forwards.is_some());
    app
}

/// The last control message the active (SSH) pane's stub session received.
fn last_input(app: &mut App) -> Option<SessionInput> {
    let rx = app.stub_inputs.last_mut().unwrap();
    let mut last = None;
    while let Ok(input) = rx.try_recv() {
        last = Some(input);
    }
    last
}

#[test]
fn add_validates_sends_and_saves_to_the_profile() {
    let t = tmp();
    let mut app = box_app(&t.0);
    text(&mut app, FwdField::Port, "80a");
    send(&mut app, ForwardsMsg::Add);
    assert!(app.forwards.as_ref().unwrap().error.is_some());
    assert!(last_input(&mut app).is_none());
    text(&mut app, FwdField::Port, "8080");
    text(&mut app, FwdField::Target, "127.0.0.1");
    text(&mut app, FwdField::TargetPort, "80");
    set(&mut app, FwdField::Save, FieldValue::Bool(true));
    send(&mut app, ForwardsMsg::Add);
    let want = PortForward {
        kind: ForwardKind::Local,
        port: 8080,
        target_address: "127.0.0.1".into(),
        target_port: 80,
        ..PortForward::default()
    };
    match last_input(&mut app) {
        Some(SessionInput::Forward(ForwardCmd::Add(sent))) => assert_eq!(sent, want),
        other => panic!("expected Add, got {other:?}"),
    }
    let dialog = app.forwards.as_ref().unwrap();
    assert!(dialog.error.is_none() && dialog.port.is_empty());
    let file = std::fs::read_to_string(t.0.join("config.toml")).unwrap();
    assert!(file.contains("forwarded_ports"), "{file}");
    assert!(file.contains("# the box"), "comments kept: {file}");
    let stored = app
        .config
        .profiles
        .iter()
        .find(|p| p.id == id("box"))
        .unwrap();
    let ProfileKind::Ssh(ssh) = &stored.kind else {
        panic!("ssh")
    };
    assert_eq!(ssh.forwarded_ports, Some(vec![want]));
}

#[test]
fn dynamic_needs_no_target_and_without_save_nothing_is_written() {
    let t = tmp();
    let mut app = box_app(&t.0);
    let before = std::fs::read_to_string(t.0.join("config.toml")).unwrap();
    set(&mut app, FwdField::Kind, FieldValue::Choice(2));
    text(&mut app, FwdField::Port, "1080");
    send(&mut app, ForwardsMsg::Add);
    match last_input(&mut app) {
        Some(SessionInput::Forward(ForwardCmd::Add(sent))) => {
            assert_eq!((sent.kind, sent.port), (ForwardKind::Dynamic, 1080));
        }
        other => panic!("expected Add, got {other:?}"),
    }
    assert_eq!(
        std::fs::read_to_string(t.0.join("config.toml")).unwrap(),
        before
    );
}

#[test]
fn statuses_fill_the_list_the_badge_and_remove() {
    let t = tmp();
    let mut app = box_app(&t.0);
    let pane = app.active_pane().unwrap().id;
    let status = |state, active_connections| ForwardStatus {
        id: ForwardId(3),
        forward: PortForward {
            port: 8080,
            ..PortForward::default()
        },
        state,
        active_connections,
    };
    for s in [
        status(ForwardState::Listening, 0),
        status(ForwardState::Listening, 1),
    ] {
        let _task = app.update(Message::Session(pane, SessionOutput::ForwardStatus(s)));
    }
    let tab = app.active_tab().unwrap();
    assert_eq!(listening(tab.panes.values()), 1);
    assert_eq!(
        app.pane(pane).unwrap().forwards,
        [status(ForwardState::Listening, 1)]
    );
    send(&mut app, ForwardsMsg::Remove(ForwardId(3)));
    assert!(matches!(
        last_input(&mut app),
        Some(SessionInput::Forward(ForwardCmd::Remove(ForwardId(3))))
    ));
    let stopped = status(ForwardState::Stopped, 0);
    let _task = app.update(Message::Session(
        pane,
        SessionOutput::ForwardStatus(stopped),
    ));
    assert_eq!(app.pane(pane).unwrap().forwards, Vec::new());
    send(&mut app, ForwardsMsg::Close);
    assert!(app.forwards.is_none());
}
