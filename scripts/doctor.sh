#!/usr/bin/env bash
# Report which prerequisites are present. Changes nothing.
source "$(dirname "$0")/_common.sh"
set +e
problems=0

check() { # name, command-to-test, fix hint
  if eval "$2" >/dev/null 2>&1; then ok "$1"; else fail "$1  ->  $3"; problems=$((problems + 1)); fi
}
optional() {
  if eval "$2" >/dev/null 2>&1; then ok "$1"; else warn "$1 (optional)  ->  $3"; fi
}

step "Core (Rust + JS)"
check "Node 20+"  'node -e "process.exit(+process.versions.node.split(\".\")[0] >= 20 ? 0 : 1)"' \
  "install Node 22 LTS (nvm install 22, or https://nodejs.org)"
check "pnpm"      'pnpm --version'  "corepack enable && corepack prepare pnpm@9.15.4 --activate"
check "cargo"     'cargo --version' "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
check "rustfmt"   'cargo fmt --version'    "rustup component add rustfmt"
check "clippy"    'cargo clippy --version' "rustup component add clippy"
check "node_modules installed" '[ -d node_modules/react-native ]' "pnpm install"

step "Android"
optional "JDK 17"        'java -version 2>&1 | grep -q "\"17\."' "install Zulu/Temurin JDK 17 and set JAVA_HOME"
optional "ANDROID_HOME"  '[ -n "${ANDROID_HOME:-}" ] && [ -d "$ANDROID_HOME" ]' \
  "install Android Studio, then export ANDROID_HOME=\$HOME/Library/Android/sdk (macOS)"
optional "adb"           'adb version' "add \$ANDROID_HOME/platform-tools to PATH"

if [[ "$(uname)" == "Darwin" ]]; then
  step "iOS (macOS only)"
  optional "Xcode"       'xcodebuild -version' "install Xcode from the App Store, then: sudo xcode-select -s /Applications/Xcode.app"
  optional "Ruby bundler" 'bundle --version'   "gem install bundler  (optional if pod is installed)"
  optional "CocoaPods (pod)" 'pod --version' "brew install cocoapods  (or: gem install cocoapods)"
  optional "iOS pods installed for this project" '[ -d apps/mobile/ios/Pods ]' "pnpm pods  (needs node_modules + Xcode first)"
  optional "Watchman"    'watchman --version'  "brew install watchman (recommended for Metro)"
fi

echo
if (( problems > 0 )); then
  fail "$problems required item(s) missing"
  exit 1
fi
ok "Core tooling looks good"
