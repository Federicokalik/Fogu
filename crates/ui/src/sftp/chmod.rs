//! The "Permissions…" dialog's model: nine rwx checkboxes and an octal field kept in sync.
//! The octal field also carries setuid/setgid/sticky (a fourth digit), which have no checkbox.

/// Rows of the checkbox grid: label and the read/write/execute bits of that class.
pub const CLASSES: [(&str, [u32; 3]); 3] = [
    ("Owner", [0o400, 0o200, 0o100]),
    ("Group", [0o040, 0o020, 0o010]),
    ("Others", [0o004, 0o002, 0o001]),
];

/// Highest mode the dialog sends (permission bits plus setuid/setgid/sticky).
const MODE_MAX: u32 = 0o7777;

/// The open dialog for one remote entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChmodDialog {
    /// Remote path (raw).
    pub path: String,
    /// Entry name (raw; shown through `display_name`).
    pub name: String,
    /// The mode the checkboxes show (always valid).
    pub mode: u32,
    /// The octal field as typed.
    pub octal: String,
}

impl ChmodDialog {
    /// A dialog starting from `mode`.
    pub fn new(path: String, name: String, mode: u32) -> Self {
        let mode = mode & MODE_MAX;
        Self {
            path,
            name,
            mode,
            octal: octal(mode),
        }
    }

    /// A checkbox changed: flips `bit` and rewrites the octal field.
    pub fn toggle(&mut self, bit: u32) {
        self.mode ^= bit & 0o777;
        self.octal = octal(self.mode);
    }

    /// The octal field changed: a valid value moves the checkboxes, an invalid one leaves them
    /// (and disables OK).
    pub fn type_octal(&mut self, value: String) {
        if let Some(mode) = parse_octal(&value) {
            self.mode = mode;
        }
        self.octal = value;
    }

    /// The mode to send, if the octal field is valid.
    pub fn result(&self) -> Option<u32> {
        parse_octal(&self.octal)
    }
}

/// `mode` as 3 octal digits, or 4 when setuid/setgid/sticky is set.
pub fn octal(mode: u32) -> String {
    if mode & 0o7000 == 0 {
        format!("{:03o}", mode & 0o777)
    } else {
        format!("{:04o}", mode & MODE_MAX)
    }
}

/// 1–4 octal digits (surrounding spaces ignored) as a mode.
pub fn parse_octal(text: &str) -> Option<u32> {
    let text = text.trim();
    if text.is_empty() || text.len() > 4 || !text.bytes().all(|b| (b'0'..=b'7').contains(&b)) {
        return None;
    }
    u32::from_str_radix(text, 8).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rwx_and_octal_stay_in_sync() {
        let mut d = ChmodDialog::new("/x".into(), "x".into(), 0o100_644);
        assert_eq!((d.mode, d.octal.as_str()), (0o644, "644"));
        d.toggle(0o020); // g+w
        assert_eq!((d.mode, d.octal.as_str()), (0o664, "664"));
        d.toggle(0o004); // o-r
        assert_eq!(d.octal, "660");
        d.type_octal("4755".into());
        assert_eq!((d.mode, d.result()), (0o4755, Some(0o4755)));
        d.toggle(0o002);
        assert_eq!(d.octal, "4757", "special bits survive a checkbox click");
        d.type_octal("79".into());
        assert_eq!((d.mode, d.result()), (0o4757, None));
    }

    #[test]
    fn octal_parsing_table() {
        let rows = [
            ("644", Some(0o644)),
            (" 755 ", Some(0o755)),
            ("7", Some(0o7)),
            ("0644", Some(0o644)),
            ("7777", Some(0o7777)),
            ("17777", None),
            ("8", None),
            ("", None),
            ("-1", None),
            ("u+x", None),
        ];
        for (text, want) in rows {
            assert_eq!(parse_octal(text), want, "{text:?}");
        }
        assert_eq!(octal(0o2750), "2750");
        assert_eq!(octal(0o7), "007");
    }
}
