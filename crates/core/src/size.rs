//! Terminal size shared by the UI, the terminal core and session transports.

/// Grid size in cells plus the text area in pixels (for `TIOCSWINSZ` / `CSI 14 t`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Size {
    /// Columns (cells).
    pub cols: u16,
    /// Rows (cells).
    pub rows: u16,
    /// Text-area width in pixels (0 when unknown).
    pub px_w: u16,
    /// Text-area height in pixels (0 when unknown).
    pub px_h: u16,
}
