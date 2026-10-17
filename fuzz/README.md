# Fuzzing

Needs the nightly toolchain and `cargo-fuzz` (`cargo install cargo-fuzz --locked`), neither of which
the rest of the workspace requires. Run from the repository root:

```sh
cargo +nightly fuzz run history_load -- -max_total_time=60
cargo +nightly fuzz run login_script_window -- -max_total_time=60
cargo +nightly fuzz run osc_payloads -- -max_total_time=60
cargo +nightly fuzz run sftp_name_sanitize -- -max_total_time=60
cargo +nightly fuzz run vt_feed -- -max_total_time=60
```

Corpora live in `corpus/<target>/`. The nightly CI job (`.github/workflows/ci.yml`, `fuzz-smoke`)
runs every target for 60s on every scheduled run; do longer runs locally when hunting a specific bug.
