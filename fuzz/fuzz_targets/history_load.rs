//! Fuzz target: a tab-recovery history file (JSON) goes through parse → `validate` → restore
//! without panicking (`fogu_term::history`).

#![no_main]

use fogu_term::history::{MAX_BYTES, SavedHistory, validate};
use fogu_term::{TermHandle, WORD_SEPARATORS};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(saved) = serde_json::from_slice::<SavedHistory>(data) else {
        return;
    };
    if validate(&saved, 10_000, MAX_BYTES).is_err() {
        return;
    }
    let mut term = TermHandle::new(80, 24, 1000, WORD_SEPARATORS);
    term.restore_history(&saved);
    let _ = term.capture_history(10_000, 0);
});
