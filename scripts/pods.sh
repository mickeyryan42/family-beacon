#!/usr/bin/env bash
# Install iOS CocoaPods for apps/mobile. Prefers the versions pinned in the
# Gemfile (via Bundler); falls back to a globally installed `pod` (e.g. Homebrew).
source "$(dirname "$0")/_common.sh"
cd "$ROOT/apps/mobile/ios"

if has bundle && bundle install; then
  bundle exec pod install "$@"
elif has pod; then
  warn "Bundler unavailable; using $(command -v pod) ($(pod --version))"
  pod install "$@"
else
  die "CocoaPods not found. Install it with: brew install cocoapods"
fi
