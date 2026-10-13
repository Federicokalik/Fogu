//! Fuzz target: `TermHandle::feed` on arbitrary bytes, then a snapshot and a resize to a
//! data-derived size, must never panic.

#![no_main]

use fogu_term::TermHandle;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut term = TermHandle::new(80, 24, 200);
    term.feed(data);
    let _ = term.snapshot();
    let _ = term.drain_events();

    // Derive a resize target from the data itself so the fuzzer explores odd grid sizes
    // (including the 1x1 clamp floor) without needing a structured input format.
    let cols = u16::from(data.first().copied().unwrap_or(1));
    let rows = u16::from(data.get(1).copied().unwrap_or(1));
    term.resize(cols, rows);
    let _ = term.snapshot();
});
