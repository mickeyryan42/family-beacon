#!/usr/bin/env bash
# One-shot local setup: JS deps, Rust toolchain/deps, and (on macOS) CocoaPods.
# Safe to re-run.
source "$(dirname "$0")/_common.sh"

step "Checking prerequisites"
require node
NODE_MAJOR="$(node -p 'process.versions.node.split(".")[0]')"
(( NODE_MAJOR >= 20 )) || die "Node $NODE_MAJOR found; Node 20+ is required (see .nvmrc)."
ok "node $(node --version)"

if ! has pnpm; then
  warn "pnpm not found; enabling it through corepack (ships with Node)"
  corepack enable
  corepack prepare pnpm@9.15.4 --activate
fi
ok "pnpm $(pnpm --version)"

require cargo
ok "$(cargo --version)"

if [[ ! -f .env ]]; then
  cp .env.example .env
  ok "created .env from .env.example"
fi

step "Installing JavaScript dependencies"
pnpm install

step "Fetching Rust dependencies"
cargo fetch

if [[ "$(uname)" == "Darwin" ]]; then
  step "iOS: installing CocoaPods dependencies"
  if ! xcode-select -p >/dev/null 2>&1 || ! has xcodebuild; then
    warn "Xcode not found. Skipping pods; Android and Rust work without it."
  elif ! has bundle && ! has pod; then
    warn "CocoaPods not found. Install it with: brew install cocoapods"
  else
    pnpm pods
  fi
fi

step "Done"
echo "  Run the Rust tests:    pnpm test:rust"
echo "  Start Metro:           pnpm start"
echo "  Launch Android / iOS:  pnpm android   |   pnpm ios"
