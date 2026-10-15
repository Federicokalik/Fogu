#!/usr/bin/env bash
# Same checks as the CI `check` job. Run from anywhere; cds into the repository root.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo check --manifest-path fuzz/Cargo.toml --locked
cargo nextest run --workspace --locked --no-tests=pass
cargo test --doc --workspace --locked
cargo deny check
