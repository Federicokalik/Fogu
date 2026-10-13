//! Application messages.

use fogu_session::SessionOutput;
use fogu_term::input::KeyInput;

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
}
