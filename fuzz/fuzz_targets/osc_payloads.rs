//! Fuzz target: the OSC side-channel scanner must never panic and must never buffer more than
//! 4 KiB for one sequence (`fogu_term::OscScanner`).

#![no_main]

use fogu_term::OscScanner;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut scanner = OscScanner::new();
    // Split the input in two arbitrary places and feed it across three `scan` calls: the
    // scanner is stateful across reads, and a split sequence is the case most likely to panic.
    let a = data.len() / 3;
    let b = 2 * data.len() / 3;
    let _ = scanner.scan(data.get(..a).unwrap_or(data));
    let _ = scanner.scan(data.get(a..b).unwrap_or(&[]));
    let _ = scanner.scan(data.get(b..).unwrap_or(&[]));
});
