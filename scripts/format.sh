#!/usr/bin/env bash
# Format everything. Pass --check to verify without writing (used by CI / `pnpm check`).
source "$(dirname "$0")/_common.sh"
require cargo

if [[ "${1:-}" == "--check" ]]; then
  step "rustfmt --check"
  cargo fmt --all -- --check
  step "prettier --check (apps/mobile)"
  pnpm --filter @family-beacon/mobile format:check
else
  step "rustfmt"
  cargo fmt --all
  step "prettier (apps/mobile)"
  pnpm --filter @family-beacon/mobile format
fi
