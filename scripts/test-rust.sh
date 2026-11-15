#!/usr/bin/env bash
# Run every Rust test (unit + integration + examples compile).
source "$(dirname "$0")/_common.sh"
require cargo
step "cargo test"
cargo test --workspace --all-targets "$@"
