#!/usr/bin/env bash
# Lint Rust (clippy, warnings are errors) and the mobile app (ESLint).
source "$(dirname "$0")/_common.sh"
require cargo
step "cargo clippy"
cargo clippy --workspace --all-targets -- -D warnings

step "eslint (apps/mobile)"
pnpm --filter @family-beacon/mobile lint
