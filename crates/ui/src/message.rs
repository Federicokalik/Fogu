//! Application messages.

use fogu_session::SessionOutput;
use fogu_term::input::KeyInput;
use fogu_term::{Scroll, SelectionType};

use crate::metrics::CellPos;

/// A step of a mouse selection gesture, in viewport cells.
#[derive(Clone, Copy, Debug)]
pub enum SelectEvent {
    /// Start a new selection of `kind` anchored at `at`.
    Start {
        /// Simple / semantic (word) / lines / block.
        kind: SelectionType,
        /// Anchor cell.
        at: CellPos,
    },
    /// Move the free end of the selection.
    Update(CellPos),
    /// The button was released: copy-on-select.
    Finish,
    /// A plain click: drop the selection.
    Clear,
}

/// Which clipboard a paste reads from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasteSource {
    /// Ctrl+Shift+V / Shift+Insert.
    Clipboard,
    /// Middle-click.
    Primary,
}

/// Everything `update` reacts to.
#[derive(Clone, Debug)]
pub enum Message {
    /// An event from the pane's session.
    Session(SessionOutput),
    /// The terminal widget now fits a different grid.
    GridResized {
        /// Columns that fit.
        cols: u16,
        /// Rows that fit.
        rows: u16,
    },
    /// The 50 ms resize debounce for generation `.0` expired.
    ResizeTimer(u64),
    /// The window's scale factor (device px per logical px).
    ScaleFactor(f32),
    /// Pane focus gained (`true`: window focused or pane clicked) or lost (window unfocused).
    Focused(bool),
    /// A key press for the focused pane.
    Key(KeyInput),
    /// Text committed by the input method for the focused pane.
    Text(String),
    /// Mouse selection gesture.
    Select(SelectEvent),
    /// Scroll the view through history.
    Scroll(Scroll),
    /// Copy the selection to CLIPBOARD (Ctrl+Shift+C).
    Copy,
    /// Read `source`'s current text and maybe paste it (Ctrl+Shift+V, Shift+Insert, middle-click).
    Paste(PasteSource),
    /// The clipboard read for a [`Message::Paste`] completed.
    PasteReady(Option<String>),
    /// The user confirmed the pending multi-line/large paste in the modal.
    ConfirmPaste,
    /// The user cancelled it.
    CancelPaste,
    /// Grow the font by 1 pt (Ctrl+=).
    ZoomIn,
    /// Shrink the font by 1 pt (Ctrl+-).
    ZoomOut,
    /// Reset the font to its default size (Ctrl+0).
    ZoomReset,
    /// Clear the screen and scrollback (Ctrl+Shift+L).
    Clear,
}
