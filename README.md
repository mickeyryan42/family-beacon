# Family Beacon

**Family check-ins and meet-up plans that keep working when the phone network doesn't.**

Family Beacon is an offline-first mobile app for households. With one tap you can tell your
family you're safe, on your way, or need help. Everyone agrees ahead of time where to meet if
you can't reach each other. You can also share short local updates ("power's out on our
street"). All of this works with no cell service, no internet, and no cloud account: updates go
directly from phone to phone when family members are near each other.

---

## Contents

1. [What problem it solves](#what-problem-it-solves)
2. [Repository layout](#repository-layout)
3. [Architecture overview](#architecture-overview)
4. [Prerequisites](#prerequisites)
5. [Install dependencies](#install-dependencies)
6. [Rust: tests, formatting, linting](#rust-tests-formatting-linting)
7. [Run the app on Android](#run-the-app-on-android)
8. [Run the app on iOS](#run-the-app-on-ios)
9. [Mock offline transport](#mock-offline-transport)
10. [What's implemented, and what's next](#whats-implemented-and-whats-next)
11. [Troubleshooting](#troubleshooting)
12. [Offline-first architecture and local-only data](#offline-first-architecture-and-local-only-data)
13. [Command reference](#command-reference)

---

## What problem it solves

During a storm, a power cut, a wildfire evacuation, or just a dead cell tower, the first
question in every household is *"is everyone okay, and where do we meet?"* Today's family apps
can't answer that when it matters most, because they depend on a server and a signal.

Family Beacon is built for that moment, and for everyday households rather than preppers:

- **One-tap check-ins:** *I'm safe · On my way · At the meeting point · I need help.*
  A parent can check in on behalf of a child or grandparent without a phone.
- **Meet-up plans:** a primary meeting point and ranked backups, written in plain language
  ("back gate of the school, by the bike racks").
- **Neighborhoods:** invite-only networks of nearby households for outage alerts, offers and
  needs, plus private "Are you OK?" check-ins between neighbors. Family check-ins never leave
  the family.
- **No signal needed:** updates are stored on the phone and carried phone to phone
  (store-and-forward), sealed so only their audience can read them. Any Family Beacon phone
  helps carry them, so a neighbor walking from the school to your street brings the kids'
  check-ins home without being able to read them.
- **Private by design:** a trusted circle added in person. No accounts, no cloud, no tracking.
- **Gentle on battery:** low-power, infrequent sync that backs off when nobody is around.

## Repository layout

```
family-beacon/
├── apps/
│   └── mobile/                 React Native app (bare, TypeScript)
│       ├── src/
│       │   ├── screens/        Welcome, Circle, Check-ins, Meet-up plans, Nearby, Settings
│       │   ├── components/     UI building blocks, offline banner, error boundary
│       │   ├── navigation/     Stack + bottom tabs
│       │   ├── state/          BeaconStore (single source of UI state) + React bindings
│       │   ├── data/           Repository layer (local storage boundary), sample data
│       │   ├── sync/           Protocol mirror, conflict fold, transports (mock + stubs)
│       │   ├── theme/          Dark-by-default theme, spacing, tap targets
│       │   └── lib/            Logger, formatting
│       ├── __tests__/          Jest tests
│       ├── android/  ios/      Native projects
├── crates/
│   ├── crypto/                 Keys, signatures, sealing (group and one-to-one), route tags, padding
│   ├── protocol/               Wire types: scopes, envelopes (readable + sealed), messages, codec, validation
│   └── sync-core/              Store-and-forward engine: queue, store, conflict rules, sessions,
│       ├── src/transport/      schedule, identity hooks, Transport trait + mock/BLE/Wi-Fi Direct
│       ├── tests/              End-to-end multi-phone scenarios on the mock transport
│       ├── examples/           `family_sim`: runnable store-and-forward walkthrough
│       └── migrations/         SQLite schema for the on-device store
├── fixtures/
│   └── sample-household.json   Shared dev dataset (validated by Rust, loaded by the app)
├── docs/                       architecture, sync protocol, transports, security, neighborhood design
├── scripts/                    setup, doctor, test, lint, format, simulate
├── Cargo.toml                  Rust workspace root
├── package.json                JS workspace root (pnpm)
└── pnpm-workspace.yaml
```

## Architecture overview

```
 ┌──────────────── one phone ────────────────┐
 │  Screens  ──▶  BeaconStore  ──▶  Repository │      (local storage only)
 │                    │                        │
 │                    ▼                        │
 │        Sync engine (Rust: crates/sync-core) │
 │        ├─ OutboundQueue (priority, bounded) │
 │        ├─ HouseholdState (conflict rules)   │
 │        ├─ SyncSchedule (battery backoff)    │
 │        └─ Transport ── mock │ BLE │ Wi-Fi D │ ◀──radio──▶ other family phones
 └─────────────────────────────────────────────┘
```

- Every change becomes an immutable, uniquely identified **envelope**. Envelopes are copied
  phone to phone until they expire or run out of hops. Duplicates are harmless.
- **Conflict rules** are deterministic. Every phone holding the same envelopes shows the same
  thing, whatever order they arrived in.
- The **Rust core** has no I/O, async runtime, or background threads. The OS wakes the app and
  the app calls `sync_round()`. That keeps the radio on the OS's power budget.
- The Rust core is **not yet bridged** into the app. Until then, the app runs a small
  TypeScript mirror of the protocol and conflict rules (`apps/mobile/src/sync`). A shared
  fixture plus a Rust contract test keep the two in step. The bridging plan is in
  [docs/architecture.md](docs/architecture.md).

More detail: [architecture](docs/architecture.md) · [sync protocol](docs/sync-protocol.md) ·
[transports](docs/transports.md) · [privacy & security](docs/security.md) ·
[neighborhood networks (design)](docs/neighborhood-network.md)

## Prerequisites

| Tool | Version | Needed for |
| --- | --- | --- |
| Node.js | 20+ (22 LTS recommended, see `.nvmrc`) | Mobile app, scripts |
| pnpm | 9.x (pinned in `package.json`) | JS workspace |
| Rust | stable, with `rustfmt` and `clippy` (see `rust-toolchain.toml`) | Sync core |
| JDK | 17 | Android builds |
| Android Studio | latest, with SDK 35 + an emulator | Android |
| Xcode | 15.1+ (macOS only) | iOS |
| Ruby + Bundler | system Ruby is fine | CocoaPods for iOS |
| Watchman | optional, recommended on macOS | Faster Metro file watching |

You can work on the Rust core with only Rust installed. You can run the mobile app on Android
alone, without Xcode.

### macOS

```bash
# Xcode command line tools (needed by Homebrew, git, and Rust's linker)
xcode-select --install

# Homebrew (if you don't have it): https://brew.sh
brew install node@22 watchman
corepack enable                      # provides pnpm at the version pinned in package.json

# Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

# Android: JDK 17 + Android Studio
brew install --cask zulu@17 android-studio
```

For iOS, also install **Xcode** from the App Store. Open it once to accept the license and
install the iOS simulator, then point the command-line tools at it:

```bash
sudo xcode-select --switch /Applications/Xcode.app
```

### Linux / Windows (Android only)

Install Node 22, Rust (via [rustup](https://rustup.rs)), JDK 17, and Android Studio from their
official sites, then run `corepack enable`. iOS builds need macOS. On Windows, use WSL2 or Git
Bash for the shell scripts.

### Android SDK environment

In Android Studio, open **Settings → Languages & Frameworks → Android SDK** and install:

- **SDK Platforms:** Android 15 (API 35)
- **SDK Tools:** Android SDK Build-Tools 35, NDK (Side by side) **26.1.10909125**, Android SDK
  Command-line Tools, Android Emulator, Android SDK Platform-Tools

Then add this to your shell profile (`~/.zshrc` or `~/.bashrc`):

```bash
export ANDROID_HOME="$HOME/Library/Android/sdk"   # Linux: $HOME/Android/Sdk
export PATH="$PATH:$ANDROID_HOME/emulator:$ANDROID_HOME/platform-tools"
export JAVA_HOME="$(/usr/libexec/java_home -v 17)" # macOS; on Linux, point at your JDK 17
```

Create an emulator under **Device Manager** (any recent Pixel image, API 34 or 35).

### Check your setup

```bash
./scripts/doctor.sh
```

This reports what's present and how to fix what isn't. It changes nothing.

## Install dependencies

From the repo root:

```bash
git clone <repo-url> family-beacon && cd family-beacon
./scripts/setup.sh
```

The setup script does the following, and is safe to re-run:

1. Checks Node, pnpm (enabling it through corepack if needed), and cargo.
2. Copies `.env.example` to `.env`. This is optional, and there are no secrets.
3. Runs `pnpm install`. Dependencies are hoisted to the root `node_modules`; see `.npmrc`.
4. Runs `cargo fetch`.
5. On macOS with Xcode, installs CocoaPods (`pnpm pods`).

To do it by hand:

```bash
pnpm install
cargo fetch
pnpm pods         # macOS + iOS only
```

## Rust: tests, formatting, linting

```bash
pnpm test:rust          # cargo test --workspace --all-targets
pnpm simulate           # runs the store-and-forward walkthrough (see below)

cargo fmt --all         # format Rust
cargo clippy --workspace --all-targets -- -D warnings

pnpm format             # rustfmt + prettier, writes changes
pnpm format:check       # verify only (CI)
pnpm lint               # clippy + eslint
pnpm check              # format:check + lint + typecheck + all tests
```

Run a single crate or test:

```bash
cargo test -p beacon-protocol
cargo test -p beacon-sync-core --test store_and_forward strangers_phone
```

What's covered:

| Area | Where |
| --- | --- |
| Signatures, sealing, route tags, padding | `crypto/src/lib.rs`, `sync-core/src/sealing.rs` |
| Message creation and validation (all scopes) | `protocol/src/validation.rs`, `protocol/tests/sample_fixture.rs` |
| Envelope serialization / deserialization | `protocol/src/codec.rs`, `protocol/tests/sample_fixture.rs` |
| `Hello` summaries (exact and Bloom) | `protocol/src/summary.rs` |
| Queueing (tiers, budgets, priority, eviction) | `sync-core/src/queue.rs` |
| Conflict handling | `sync-core/src/conflict.rs`, `sync-core/src/feed.rs` |
| Neighborhood rosters, stewards, removal, invite limits | `sync-core/src/neighborhood.rs` |
| Sessions, frame packing, backoff, transports | `session.rs`, `schedule.rs`, `transport/mock.rs` |
| Sealed relay by strangers, invite chains, direct messages, key rotation, restarts | `sync-core/tests/store_and_forward.rs` |

Mobile tests: `pnpm test:mobile` (Jest) and `pnpm typecheck` (tsc).

## Run the app on Android

1. Start an emulator from Android Studio's Device Manager, or plug in a phone with USB
   debugging enabled. Check it shows up:
   ```bash
   adb devices
   ```
2. In one terminal, start Metro (the JS bundler):
   ```bash
   pnpm start
   ```
3. In a second terminal, build and install:
   ```bash
   pnpm android
   ```

The first build downloads Gradle and the Android dependencies and takes several minutes. Later
builds are much faster. Press `r` in the Metro terminal to reload, or `d` for the dev menu.

On a physical device, if the app can't reach Metro: `adb reverse tcp:8081 tcp:8081`.

## Run the app on iOS

macOS with Xcode only.

1. Install pods (once, and again whenever native dependencies change):
   ```bash
   pnpm pods      # cd apps/mobile/ios && bundle install && bundle exec pod install
   ```
2. Start Metro in one terminal:
   ```bash
   pnpm start
   ```
3. Build and launch the simulator in another:
   ```bash
   pnpm ios
   # or pick a device:
   pnpm ios --simulator "iPhone 16"
   ```

You can also open `apps/mobile/ios/FamilyBeacon.xcworkspace` (the **workspace**, not the
`.xcodeproj`) in Xcode and press Run. To run on a physical iPhone, select your team under
**Signing & Capabilities** first.

## Mock offline transport

There's no BLE or Wi-Fi Direct code yet, so development runs on a **mock transport** that
simulates the phones around you. No radios, no network, and no second device needed.

**In the app** (debug builds start on the mock automatically):

1. On the Welcome screen, tap **Explore with a sample family**. This loads
   `fixtures/sample-household.json`: the Lee household, three meeting points, and a few
   check-ins.
2. Every **20 seconds**, a simulated phone "comes into range" and swaps envelopes with you:
   - **family phones** head to the meeting point, check in and leave notes
   - **neighbor phones** (once you've joined a neighborhood) post alerts, offers and needs,
     ask "Are you OK?", and answer your messages
   - **a passer-by** from another household hands over sealed traffic your phone can only
     carry, never read

   Each exchange is listed under **Nearby → Recent exchanges**.
3. **Nearby → Family / Neighbor / Passer-by** brings one of each into range immediately.
4. **Neighbors → Simulate joining Elm & Oak** stands in for scanning a neighbor's invite. On
   the **People** screen, **Simulate inviting a neighbor** stands in for inviting someone in
   person. Real QR invites need camera support.
5. **Settings → Developer**:
   - switch transport (Mock / Bluetooth LE / Wi-Fi Direct). The radio options honestly report
     "not available yet".
   - **Simulate no signal** shows the offline banner as users will see it.
   - **Show recent logs** shows the dev log on the device.

To change the interval or other defaults, edit `apps/mobile/src/config.ts`.

**In Rust**, the mock is a shared in-memory "room" where phones move in and out of range, with
real encryption and signatures:

```bash
pnpm simulate
```

```
== Jordan walks past Dana's house. Dana is not family.
   Dana's phone is carrying 2 sealed envelopes it cannot read
== Dana walks on and passes Maya at the school gate
   Maya now has Jordan's check-in, carried by a neighbor who couldn't read it.
== Sam starts an 'Elm & Oak' neighborhood and invites Dana in person
   Sam sees a post from Dana: Alert { category: Power, text: "Transformer down at Elm & 5th", ... }
   Dana still can't read the Lee family's check-ins: true
```

`crates/sync-core/tests/store_and_forward.rs` uses the same mock for 17 automated multi-phone
scenarios.

## What's implemented, and what's next

### Implemented

- **Three scopes** ([design](docs/neighborhood-network.md)):
  - **Household:** check-ins, meeting points and notes, for family only.
  - **Neighborhood:** invite-only networks of nearby households, for alerts, offers, needs and
    an opt-in "everyone safe".
  - **Direct:** private messages and one-tap "Are you OK?" between any two people who share a
    circle.
- **Sealed relay:** every Family Beacon phone carries everyone's encrypted traffic, so reach
  grows with every install. Only the audience can read it. Relaying can be turned off in
  Settings.
- **Rust crypto crate:** Ed25519 signatures, XChaCha20-Poly1305 group sealing, X25519 + HKDF
  one-to-one sealing, HMAC route tags, and size padding.
- **Rust protocol crate (v2):** scopes and audiences, every payload type, readable and sealed
  envelopes, membership certificates, an exact-or-Bloom `Hello` summary, a codec with base64
  bytes, and two-level validation.
- **Rust sync core:** a `SyncEngine` that authors, seals, recognizes, opens, verifies and
  relays envelopes. It also covers:
  - neighborhoods: create, invite in person, accept, stewards, removal with optional cascade,
    and immediate key rotation
  - per-person rate limits and local blocking
  - a tiered `OutboundQueue` with per-scope budgets, plus deterministic conflict rules
  - sessions, frame packing, battery-aware scheduling and peer cooldowns
- **Transports:** a `Transport` trait, a working `MockTransport`, and `BleTransport` /
  `WifiDirectTransport` stubs.
- **Tests:** 93 Rust tests (unit, 18 end-to-end multi-phone scenarios, a store contract run
  against both backends, fixture contract) and 32 mobile tests (folds, store flows, restarts,
  freshness, storage, and full UI flows).
- **SQLite store in Rust:** `SqliteStore` (`sync-core/src/sqlite_store.rs`) implements
  `EnvelopeStore`, applies `migrations/` in order (one transaction each), refuses a newer
  schema, and passes the same contract tests as `MemoryStore`. A `SyncEngine` restarts from it
  with its state and outbound queue intact.
- **App data survives restarts:** `StorageRepository` keeps the household, envelopes, carried
  relay traffic and local-only state (joined neighborhoods, blocks, relay setting, today's
  invites, the Lamport clock floor) in on-device storage (AsyncStorage) until the Rust store is
  bridged in. Open screens re-derive every 30 s and on returning to the foreground, so expired
  check-ins disappear and "x min ago" stays true.
- **Mobile app:** bare React Native 0.76 + TypeScript. Screens: Welcome, trusted circle,
  check-ins (including on behalf of others), meet-up plans, **Neighbors** (feed, composer,
  people, steward tools, private messages), Nearby, and Settings (with the relay toggle and
  blocked people). Every compose area says who will see what you write. Dark theme by default,
  an always-visible offline banner, empty states, large tap targets, minimal motion, a dev error
  boundary and an on-device log viewer.
- **Mock offline transport** in the app, with simulated family, neighbor and passer-by phones.
- **Developer experience:** setup/doctor/test/lint/format/simulate scripts, a shared sample
  dataset, and CI.

### Future work (roughly in order)

1. **Bridge Rust into the app** (UniFFI + Turbo Module) and retire the TypeScript mirror. The
   app follows the same rules today but doesn't encrypt.
2. **Keys in the OS keystore and in-person QR pairing** for households and neighborhood
   invites (needs camera support).
3. **Retire the interim app storage:** once the bridge lands, move the app's repository onto the
   Rust `SqliteStore` (already built and tested) and migrate any `StorageRepository` data on
   first launch.
4. **BLE transport:** native iOS/Android modules, rotating identifiers, background modes, and
   permissions.
5. **Encrypted database at rest.**
6. **Wi-Fi Direct / Multipeer** for bulk catch-up.
7. **Scale work:** per-peer sync watermarks, range-based set reconciliation, compact binary
   encoding for BLE, and map pins for meeting points.
8. *Optional, and only if needed:* a separate relay/admin service. No core feature may depend
   on it.

Deliberately **out of scope**: cloud login, account sync, and a production backend.

## Troubleshooting

**`pnpm: command not found`**: run `corepack enable`. Corepack ships with Node 16.10+. If that
fails because of permissions, use `corepack enable --install-directory ~/.local/bin`.

**`cargo: command not found` right after installing Rust**: open a new terminal or run
`source "$HOME/.cargo/env"`.

**Metro: `Unable to resolve module ...`**: dependencies must be hoisted. Check that `.npmrc`
contains `node-linker=hoisted`, then reinstall:
`rm -rf node_modules apps/mobile/node_modules && pnpm install && pnpm start:reset`.

**Port 8081 already in use**: another Metro is running. Stop it, or use
`pnpm start --port 8082` together with `pnpm android --port 8082` (or `pnpm ios --port 8082`).

**Android: `SDK location not found`**: set `ANDROID_HOME` (see above), or create
`apps/mobile/android/local.properties` containing `sdk.dir=/Users/you/Library/Android/sdk`.

**Android: `Unsupported class file major version` / Gradle JDK errors**: Gradle needs **JDK
17**. Check `java -version`, and set `JAVA_HOME` to JDK 17. In Android Studio, also check
**Settings → Build Tools → Gradle → Gradle JDK**.

**Android: `NDK not configured` / wrong NDK version**: install NDK `26.1.10909125` from the SDK
Manager (Show Package Details).

**Android: app installs but shows a red "Unable to load script" screen**: Metro isn't
reachable. Make sure `pnpm start` is running. On a physical device, run
`adb reverse tcp:8081 tcp:8081`.

**Android: stale build weirdness**: `cd apps/mobile/android && ./gradlew clean`, then
`pnpm android`.

**iOS: `xcode-select: error: tool 'xcodebuild' requires Xcode`**: the command-line tools point
at the standalone CLT, not Xcode. Run
`sudo xcode-select --switch /Applications/Xcode.app`.

**iOS: `pod install` fails or `bundle: command not found`**: run `gem install bundler`, then
`pnpm pods`. After native dependency changes, try
`cd apps/mobile/ios && bundle exec pod install --repo-update`.

**iOS: build fails after pulling changes**: delete derived data and reinstall pods:
`rm -rf ~/Library/Developer/Xcode/DerivedData apps/mobile/ios/Pods apps/mobile/ios/build && pnpm pods`.

**Apple Silicon: simulator build fails with architecture errors**: make sure you opened
`FamilyBeacon.xcworkspace` and are using a simulator rather than "Any iOS Device". Also avoid
running the terminal under Rosetta.

**The app shows "Something went wrong"**: in debug builds the screen shows the error and stack.
The full message is also in the Metro terminal, under **Settings → Show recent logs**.

**Jest: `SyntaxError: Cannot use import statement outside a module`** in a new dependency: add
the package to `transformIgnorePatterns` in `apps/mobile/jest.config.js`.

**Still stuck?** Run `./scripts/doctor.sh` and include its output when asking for help.

## Offline-first architecture and local-only data

**The app has to keep working when cellular service, the internet, and cloud services are all
unavailable.** This repo meets that by construction rather than by fallback:

- **No network in the critical path.** No code path calls a server. Android's `INTERNET`
  permission exists only so debug builds can reach Metro.
- **Local first.** Every action is saved on the phone and shown immediately. There is no
  "sending..." state that can fail.
- **Store-and-forward, sealed.** Updates are immutable envelopes, sealed so only their audience
  (household, neighborhood, or one person) can read them. When any two Family Beacon phones
  meet, each sends what the other lacks. Phones carry other people's sealed envelopes too, so
  updates travel with everyone (bounded hops: 12 for household, 16 for neighborhood and direct).
- **Deterministic convergence.** Check-ins are latest-reported-wins per person. Meeting points
  are causal last-writer-wins with tombstones. Notes and posts are append-only. Neighborhood
  rosters are recomputed from signed membership events. Any phone holding the same envelopes
  shows the same picture.
- **Low-power sync.** There's no background thread. Sync runs when the OS wakes the app, and
  backs off from 30 s to 15 min while alone. Recently synced peers are skipped unless there's
  something new, every tier has a per-encounter cap, BLE is preferred, and Wi-Fi Direct is only
  for bulk catch-up.
- **Bounded storage.** Check-ins and notes expire after 2 days, neighborhood posts after 1, and
  plans after 90. Each tier (household, direct, neighborhood, relay) has its own entry and byte
  budget, so other people's traffic can never push out your family's. "I need help" is the last
  thing ever dropped.
- **Local-only data model.** Households, neighborhoods, keys and every envelope live in the
  on-device store (schema in `crates/sync-core/migrations/`). Membership is set in person and
  is never discovered over the air. A phone reads only its own circles' envelopes; everything
  else it can only carry. No account, no telemetry, no analytics.

## Command reference

| Command | What it does |
| --- | --- |
| `./scripts/setup.sh` | Install everything (JS, Rust, pods on macOS) |
| `./scripts/doctor.sh` | Check prerequisites |
| `pnpm start` / `pnpm start:reset` | Start Metro / with a clean cache |
| `pnpm android` / `pnpm ios` | Build and launch the app |
| `pnpm pods` | Install iOS CocoaPods |
| `pnpm simulate` | Rust store-and-forward walkthrough on the mock transport |
| `pnpm test` | Rust + mobile tests |
| `pnpm test:rust` / `pnpm test:mobile` | One side only |
| `pnpm typecheck` | TypeScript check |
| `pnpm lint` | clippy + eslint |
| `pnpm format` / `pnpm format:check` | rustfmt + prettier |
| `pnpm check` | Everything CI checks |

## License

MIT. See [LICENSE](LICENSE).
