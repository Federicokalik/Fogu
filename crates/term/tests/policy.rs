//! Security regression corpus for the `EventProxy` policy: terminal output is attacker input, and
//! every rule here is deliberately independent of whatever else changed in `term.rs`.
//!
//! **Re-run this file whenever `alacritty_terminal` (or `vte`) is bumped** — a new release could
//! add a reply this policy never intended to allow.

use fogu_term::{CLIPBOARD_STORE_MAX_BYTES, OscScanner, TermEvent, TermHandle, WORD_SEPARATORS};

fn fed(bytes: &[u8]) -> TermHandle {
    let mut t = TermHandle::new(80, 24, 100, WORD_SEPARATORS);
    t.feed(bytes);
    t
}

fn pty_writes(t: &mut TermHandle) -> Vec<Vec<u8>> {
    t.drain_events()
        .into_iter()
        .filter_map(|e| match e {
            TermEvent::PtyWrite(bytes) => Some(bytes),
            _ => None,
        })
        .collect()
}

/// Minimal base64 encoder (no padding-tolerant decoder assumptions needed on our side: we only
/// need *valid* base64 so alacritty's OSC 52 handler actually decodes it).
fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk.first().copied().unwrap_or(0);
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        let n = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
        let sextet = |shift: u32| ALPHABET.get(((n >> shift) & 0x3F) as usize).copied();
        out.push(sextet(18).unwrap_or(b'A') as char);
        out.push(sextet(12).unwrap_or(b'A') as char);
        out.push(if chunk.len() > 1 {
            sextet(6).unwrap_or(b'A') as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            sextet(0).unwrap_or(b'A') as char
        } else {
            '='
        });
    }
    out
}

/// OSC 52 read (`c;?`) must never be answered: alacritty's `ClipboardLoad` is always dropped
/// without calling the formatter (see also `event_proxy::tests::clipboard_load_is_dropped...`).
#[test]
fn osc52_read_yields_no_event_and_no_reply() {
    let mut t = fed(b"\x1b]52;c;?\x07\x1b]52;p;?\x1b\\");
    assert_eq!(t.drain_events(), []);
}

#[test]
fn decrqss_is_never_echoed() {
    let mut t = fed(b"\x1bP$qm\x1b\\");
    let writes = pty_writes(&mut t);
    assert_eq!(writes, Vec::<Vec<u8>>::new());
}

#[test]
fn title_report_csi_21t_is_never_echoed() {
    let mut t = fed(b"\x1b]2;secret-token\x07\x1b[21t");
    let writes = pty_writes(&mut t);
    assert_eq!(writes, Vec::<Vec<u8>>::new());
    for reply in &writes {
        assert!(!contains_subslice(reply, b"secret-token"));
    }
}

#[test]
fn xtgettcap_is_never_echoed() {
    // "TN" (terminal-name) capability query, hex-encoded.
    let mut t = fed(b"\x1bP+q544e\x1b\\");
    let writes = pty_writes(&mut t);
    assert_eq!(writes, Vec::<Vec<u8>>::new());
}

/// A 2 MiB OSC 52 write (decoded payload, well past [`CLIPBOARD_STORE_MAX_BYTES`]) must be
/// dropped: no `ClipboardStore` event, and the raw payload bytes never show up in a `PtyWrite`.
#[test]
fn oversized_osc52_write_is_dropped() {
    let payload = vec![b'A'; 2 * 1024 * 1024];
    assert!(payload.len() > CLIPBOARD_STORE_MAX_BYTES);
    let encoded = base64_encode(&payload);
    let mut t = fed(format!("\x1b]52;c;{encoded}\x07").as_bytes());
    let events = t.drain_events();
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, TermEvent::ClipboardStore { .. })),
        "oversized clipboard store must be dropped, got {events:?}"
    );
    for event in &events {
        if let TermEvent::PtyWrite(bytes) = event {
            assert!(!contains_subslice(bytes, &payload));
        }
    }
}

/// A 1 MiB OSC 7 (`file://` cwd report) must be discarded by the side-channel scanner: it never
/// allocates past [`OscScanner`]'s 4 KiB bound and never produces an event.
#[test]
fn oversized_osc7_is_discarded_by_the_scanner() {
    let huge_path = "a".repeat(1024 * 1024);
    let mut scanner = OscScanner::new();
    let events = scanner.scan(format!("\x1b]7;file://host/{huge_path}\x07").as_bytes());
    assert_eq!(events, Vec::new());
}

/// Belt-and-suspenders: none of the fixed-format replies above ever contain the attacker-chosen
/// bytes we fed in, whatever reply was generated for unrelated reasons (DA/DSR etc. are exact
/// fixed bytes, asserted elsewhere in `term.rs`).
fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|w| w == needle)
}
