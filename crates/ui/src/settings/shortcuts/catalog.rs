//! What the Shortcuts page calls each action, where it groups it, and which chords would take a
//! key away from the shell.

use fogu_core::{Action, Dir, KeyChord, KeyName, NamedKey};

/// A group of the Shortcuts page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    Tabs,
    Panes,
    Clipboard,
    View,
    Scroll,
    Panels,
    Session,
}

impl Category {
    /// Its heading.
    pub fn label(self) -> &'static str {
        match self {
            Self::Tabs => "Tabs",
            Self::Panes => "Panes",
            Self::Clipboard => "Clipboard",
            Self::View => "View",
            Self::Scroll => "Scrolling",
            Self::Panels => "Panels and windows",
            Self::Session => "Sessions",
        }
    }
}

/// `action`'s group.
pub fn category(action: Action) -> Category {
    match action {
        Action::NewTab
        | Action::CloseTab
        | Action::NextTab
        | Action::PrevTab
        | Action::GoToTab(_)
        | Action::RenameTab
        | Action::NewTabHere
        | Action::DuplicateTab
        | Action::CloseOtherTabs
        | Action::CloseTabsRight => Category::Tabs,
        Action::SplitRight
        | Action::SplitDown
        | Action::ClosePane
        | Action::PaneNav(_)
        | Action::ResizePane(_)
        | Action::MaximizePane
        | Action::SwapPanes
        | Action::EqualizePanes => Category::Panes,
        Action::Copy
        | Action::Paste
        | Action::PastePrimary
        | Action::SelectAll
        | Action::CopyPath => Category::Clipboard,
        Action::Find
        | Action::Clear
        | Action::ZoomIn
        | Action::ZoomOut
        | Action::ZoomReset
        | Action::ToggleFullscreen => Category::View,
        Action::ScrollPageUp
        | Action::ScrollPageDown
        | Action::ScrollTop
        | Action::ScrollBottom => Category::Scroll,
        Action::OpenProfilePicker
        | Action::ToggleSftp
        | Action::ToggleProfiles
        | Action::OpenSettings => Category::Panels,
        Action::Reconnect | Action::RestartTab | Action::ClearSavedSessions => Category::Session,
    }
}

/// `action`'s name on the page.
pub fn label(action: Action) -> String {
    let dir = |d: Dir| match d {
        Dir::Left => "left",
        Dir::Right => "right",
        Dir::Up => "up",
        Dir::Down => "down",
    };
    match action {
        Action::NewTab => "New tab".to_owned(),
        Action::CloseTab => "Close tab".to_owned(),
        Action::NextTab => "Next tab".to_owned(),
        Action::PrevTab => "Previous tab".to_owned(),
        Action::GoToTab(n) => format!("Go to tab {n}"),
        Action::RenameTab => "Rename tab".to_owned(),
        Action::NewTabHere => "New tab in the same directory".to_owned(),
        Action::DuplicateTab => "Duplicate tab".to_owned(),
        Action::CloseOtherTabs => "Close other tabs".to_owned(),
        Action::CloseTabsRight => "Close tabs to the right".to_owned(),
        Action::SplitRight => "Split right".to_owned(),
        Action::SplitDown => "Split down".to_owned(),
        Action::ClosePane => "Close pane".to_owned(),
        Action::PaneNav(d) => format!("Focus pane {}", dir(d)),
        Action::ResizePane(d) => format!("Grow pane {}", dir(d)),
        Action::MaximizePane => "Maximize pane".to_owned(),
        Action::SwapPanes => "Swap panes".to_owned(),
        Action::EqualizePanes => "Equalize panes".to_owned(),
        Action::Copy => "Copy".to_owned(),
        Action::Paste => "Paste".to_owned(),
        Action::PastePrimary => "Paste the selection".to_owned(),
        Action::SelectAll => "Select all".to_owned(),
        Action::CopyPath => "Copy current directory".to_owned(),
        Action::Find => "Find".to_owned(),
        Action::Clear => "Clear screen and scrollback".to_owned(),
        Action::ZoomIn => "Zoom in".to_owned(),
        Action::ZoomOut => "Zoom out".to_owned(),
        Action::ZoomReset => "Reset zoom".to_owned(),
        Action::ToggleFullscreen => "Full screen".to_owned(),
        Action::ScrollPageUp => "Scroll a page up".to_owned(),
        Action::ScrollPageDown => "Scroll a page down".to_owned(),
        Action::ScrollTop => "Scroll to the top".to_owned(),
        Action::ScrollBottom => "Scroll to the bottom".to_owned(),
        Action::OpenProfilePicker => "Open a profile".to_owned(),
        Action::ToggleSftp => "SFTP panel".to_owned(),
        Action::ToggleProfiles => "Profiles sidebar".to_owned(),
        Action::OpenSettings => "Settings".to_owned(),
        Action::Reconnect => "Reconnect".to_owned(),
        Action::RestartTab => "Restart pane".to_owned(),
        Action::ClearSavedSessions => "Clear saved sessions".to_owned(),
    }
}

/// How much a chord gets in the way of typing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// It would stop a key from typing at all: refused.
    Forbidden,
    /// Shells and editors use it: allowed after a warning.
    Warn,
}

/// Whether `chord` would take a key away from typing (`Forbidden`) or from the shell (`Warn`),
/// and why. `alt_is_meta`: Alt+letter is readline's word motion then.
pub fn check(chord: KeyChord, alt_is_meta: bool) -> Option<(Severity, &'static str)> {
    let m = chord.mods;
    let bare = !m.ctrl && !m.alt && !m.meta;
    match chord.key {
        KeyName::Named(NamedKey::Escape) if bare => Some((
            Severity::Forbidden,
            "Escape closes dialogs and reaches programs",
        )),
        KeyName::Char(_)
        | KeyName::Named(NamedKey::Space | NamedKey::Enter | NamedKey::Tab | NamedKey::Backspace)
            if bare =>
        {
            Some((
                Severity::Forbidden,
                "a key without Ctrl, Alt or the system key would stop typing it",
            ))
        }
        KeyName::Char(c) if m.ctrl && !m.alt && !m.meta && !m.shift => {
            "CDZ\\LRAEUWKYSQ".contains(c).then_some((
                Severity::Warn,
                "shells and editors use this key (interrupt, end of input, line editing…)",
            ))
        }
        KeyName::Char('B' | 'F' | 'D') if alt_is_meta && m.alt && !m.ctrl && !m.meta => Some((
            Severity::Warn,
            "readline moves and deletes words with this key",
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::unwrap_used, reason = "tests")]

    use super::*;

    fn severity(chord: &str) -> Option<Severity> {
        check(KeyChord::parse(chord).unwrap(), true).map(|(s, _)| s)
    }

    #[test]
    fn plain_keys_are_refused_and_shell_keys_warned() {
        assert_eq!(severity("A"), Some(Severity::Forbidden));
        assert_eq!(severity("Shift-A"), Some(Severity::Forbidden));
        assert_eq!(severity("Escape"), Some(Severity::Forbidden));
        assert_eq!(severity("Ctrl-C"), Some(Severity::Warn));
        assert_eq!(severity("Alt-B"), Some(Severity::Warn));
        assert_eq!(severity("Ctrl-Shift-C"), None);
        assert_eq!(severity("F5"), None);
        assert_eq!(severity("Alt-1"), None);
    }
}
