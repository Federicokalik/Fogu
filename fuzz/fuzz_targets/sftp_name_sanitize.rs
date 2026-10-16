//! Fuzz target: the SFTP name sanitiser (`fogu_session::{validate_name, display_name}`). Remote
//! names are attacker input: neither function may panic, an accepted name must be a single plain
//! path component, and a displayed name must carry no control or bidi-override characters and
//! stay short.

#![no_main]

use fogu_session::sftp::{DISPLAY_NAME_MAX_CHARS, NAME_MAX};
use fogu_session::{display_name, validate_name};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Names arrive as `String`s (russh-sftp decodes lossily), so fuzz the UTF-8 view too.
    let text = String::from_utf8_lossy(data);
    if let Ok(name) = validate_name(&text) {
        assert!(!name.is_empty() && name != "." && name != "..");
        assert!(name.len() <= NAME_MAX);
        assert!(!name.contains(['/', '\0']));
        assert!(!name.chars().any(char::is_control));
        assert!(
            !name
                .chars()
                .any(|c| matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
        );
    }
    let shown = display_name(&text);
    assert!(!shown.chars().any(char::is_control));
    assert!(
        !shown
            .chars()
            .any(|c| matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
    );
    assert!(shown.chars().count() <= DISPLAY_NAME_MAX_CHARS + 1);
    // Escaping is idempotent on a short name without backslashes (nothing left to escape).
    if !shown.contains('\\') && text.chars().count() <= DISPLAY_NAME_MAX_CHARS {
        assert_eq!(display_name(&shown), shown);
    }
});
