#!/usr/bin/env bash
# Run the store-and-forward walkthrough on the Rust mock transport.
source "$(dirname "$0")/_common.sh"
require cargo
cargo run --quiet -p beacon-sync-core --example family_sim
