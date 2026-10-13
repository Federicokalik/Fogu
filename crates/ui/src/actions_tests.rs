//! Reducer tests on a headless `App` whose panes use stub sessions (no process, no window).
#![expect(clippy::unwrap_used, clippy::indexing_slicing, reason = "tests")]

use std::sync::Arc;

use fogu_core::{Action, Config, RgbColor};
use fogu_session::{ExitInfo, SessionInput, SessionOutput, SessionRuntime};

use crate::app::App;
use crate::message::Message;
use crate::metrics::{FontUnits, ResolvedFont};
use crate::tab::{PaneId, TabId};

fn headless() -> App {
    let runtime = Arc::new(SessionRuntime::new().unwrap());
    let resolved = ResolvedFont {
        font: iced::Font::MONOSPACE,
        units: FontUnits::FALLBACK,
    };
    let mut app = App::new(runtime, Arc::new(Config::default()), resolved);
    let profile = app.default_profile();
    app.open_tab(profile, None);
    app
}

fn act(app: &mut App, action: Action) {
    let _task = app.update(Message::Action(action));
}

fn send(app: &mut App, message: Message) {
    let _task = app.update(message);
}

fn ids(app: &App) -> Vec<TabId> {
    app.tabs.iter().map(|t| t.id).collect()
}

fn pane_of(app: &App, index: usize) -> PaneId {
    app.tabs[index].pane.id
}

/// What the UI sent to stub session `n` (in opening order).
fn sent(app: &mut App, n: usize) -> Vec<SessionInput> {
    let rx = &mut app.stub_inputs[n];
    std::iter::from_fn(|| rx.try_recv().ok()).collect()
}

const EXITED: SessionOutput = SessionOutput::Exited(ExitInfo {
    code: Some(0),
    signal: None,
});

#[test]
fn new_tab_appends_and_activates() {
    let mut app = headless();
    act(&mut app, Action::NewTab);
    act(&mut app, Action::NewTab);
    let tabs = ids(&app);
    assert_eq!(tabs.len(), 3);
    assert_eq!(app.active, tabs[2]);
    assert!(
        tabs[0].0 < tabs[1].0 && tabs[1].0 < tabs[2].0,
        "monotonic ids"
    );
    assert_eq!(app.title(), "Default shell");
}

#[test]
fn next_prev_and_go_to_tab() {
    let mut app = headless();
    act(&mut app, Action::NewTab);
    act(&mut app, Action::NewTab);
    let tabs = ids(&app);
    act(&mut app, Action::NextTab);
    assert_eq!(app.active, tabs[0], "next wraps around");
    act(&mut app, Action::PrevTab);
    assert_eq!(app.active, tabs[2], "prev wraps around");
    act(&mut app, Action::GoToTab(2));
    assert_eq!(app.active, tabs[1]);
    act(&mut app, Action::GoToTab(9));
    assert_eq!(app.active, tabs[1], "out of range is ignored");
    send(&mut app, Message::ActivateTab(tabs[0]));
    assert_eq!(app.active, tabs[0]);
}

#[test]
fn close_tab_waits_for_exited_then_activates_a_neighbour() {
    let mut app = headless();
    act(&mut app, Action::NewTab);
    act(&mut app, Action::NewTab);
    let tabs = ids(&app);
    act(&mut app, Action::GoToTab(2));
    act(&mut app, Action::CloseTab);
    assert!(matches!(
        sent(&mut app, 1).as_slice(),
        [SessionInput::Close]
    ));
    assert_eq!(ids(&app), tabs, "still there until the session exits");
    let pane = pane_of(&app, 1);
    send(&mut app, Message::Session(pane, EXITED));
    assert_eq!(ids(&app), [tabs[0], tabs[2]]);
    assert_eq!(app.active, tabs[2]);
}

#[test]
fn close_tab_times_out_and_closed_sessions_go_at_once() {
    let mut app = headless();
    act(&mut app, Action::NewTab);
    let tabs = ids(&app);
    send(&mut app, Message::CloseTab(tabs[0]));
    let pane = pane_of(&app, 0);
    send(&mut app, Message::CloseTimeout(pane));
    assert_eq!(ids(&app), [tabs[1]]);

    // Session already gone (input channel closed): removed without waiting. Last tab → empty
    // (the app exits).
    app.stub_inputs.clear();
    act(&mut app, Action::CloseTab);
    assert!(app.tabs.is_empty());
}

#[test]
fn rename_overrides_osc_title_and_empty_clears() {
    let mut app = headless();
    let pane = pane_of(&app, 0);
    send(
        &mut app,
        Message::Session(pane, SessionOutput::Title("vim".into())),
    );
    assert_eq!(app.title(), "vim");
    act(&mut app, Action::RenameTab);
    send(&mut app, Message::RenameInput("work\u{7}".into()));
    send(&mut app, Message::CommitRename);
    assert_eq!(app.title(), "work");
    send(
        &mut app,
        Message::Session(pane, SessionOutput::Title("zsh".into())),
    );
    assert_eq!(
        app.title(),
        "work",
        "the override wins over later OSC titles"
    );
    let tab = app.active;
    send(&mut app, Message::StartRename(tab));
    send(&mut app, Message::RenameInput("  ".into()));
    send(&mut app, Message::CommitRename);
    assert_eq!(app.title(), "zsh");
    send(&mut app, Message::StartRename(tab));
    send(&mut app, Message::Escape);
    assert!(app.renaming.is_none());
}

#[test]
fn tab_menu_sets_and_clears_colour() {
    let mut app = headless();
    let tab = app.active;
    send(&mut app, Message::OpenTabMenu(tab));
    send(&mut app, Message::MenuColors);
    send(&mut app, Message::MenuInput("nope".into()));
    send(&mut app, Message::MenuSubmit);
    assert!(app.menu.as_ref().unwrap().input.as_ref().unwrap().invalid);
    send(&mut app, Message::MenuInput("#ff0000".into()));
    send(&mut app, Message::MenuSubmit);
    assert!(app.menu.is_none());
    assert_eq!(app.tabs[0].color, Some(RgbColor { r: 255, g: 0, b: 0 }));
    send(&mut app, Message::OpenTabMenu(tab));
    send(&mut app, Message::Escape);
    assert!(app.menu.is_none());
    send(&mut app, Message::SetTabColor(None));
    assert_eq!(app.tabs[0].color, None);
}

#[test]
fn new_tab_here_uses_only_a_local_cwd_reply() {
    let mut app = headless();
    let pane = pane_of(&app, 0);
    act(&mut app, Action::NewTabHere);
    assert!(matches!(
        sent(&mut app, 0).as_slice(),
        [SessionInput::QueryCwd]
    ));
    let cwd = |remote| SessionOutput::Cwd {
        path: "/tmp".into(),
        remote,
    };
    send(&mut app, Message::Session(pane, cwd(true)));
    assert_eq!(app.tabs.len(), 1, "remote paths are informational");
    send(&mut app, Message::Session(pane, cwd(false)));
    assert_eq!(app.tabs.len(), 2);
    send(&mut app, Message::Session(pane, cwd(false)));
    assert_eq!(app.tabs.len(), 2, "only one tab per query");
}

#[test]
fn events_from_unknown_panes_are_ignored() {
    let mut app = headless();
    send(&mut app, Message::Session(PaneId(999), EXITED));
    assert_eq!(app.tabs.len(), 1);
    let pane = pane_of(&app, 0);
    act(&mut app, Action::RestartTab);
    assert_ne!(pane_of(&app, 0), pane, "restart opens a new pane");
    send(
        &mut app,
        Message::Session(pane, SessionOutput::Title("old".into())),
    );
    assert_eq!(app.title(), "Default shell");
}
