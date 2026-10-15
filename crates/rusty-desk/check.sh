#!/usr/bin/env bash
# The desk's gates before a commit: formatted, lint-clean, tests green.
# Exits non-zero on the first that fails, so `bash check.sh && git commit`
# commits only what passed.
set -euo pipefail
cd "$(dirname "$0")/../.."
cargo fmt -p rusty-desk -- --check
cargo clippy -p rusty-desk --release --all-targets -- -D warnings
cargo test -p rusty-desk --release --quiet
