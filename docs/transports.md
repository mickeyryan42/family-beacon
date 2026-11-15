# Transports

A transport moves opaque byte frames between this phone and nearby phones. It knows nothing
about envelopes or households. The Rust trait (`crates/sync-core/src/transport/mod.rs`) is
synchronous and poll-based. Native code owns the radios and buffers frames, and the engine
drains them when the OS wakes the app.

| Transport | Status | Role |
| --- | --- | --- |
| `MockTransport` (Rust) | Implemented | In-process "room" for tests and `pnpm simulate`. |
| `MockTransport` (TS) | Implemented | Simulated family phones in the running app. |
| `BleTransport` | Stub | Discovery and small updates. Always on, low duty cycle. |
| `WifiDirectTransport` | Stub | Bulk catch-up after BLE finds a household member. |

## Mock transport

**Rust:** `MockNetwork` is a shared room. Each `MockTransport` joined to it is a phone. Tests
call `set_in_range(a, b, true/false)` to model people moving around. `frames_sent()` lets tests
assert that sync isn't chattier than expected.

**App:** in development the app starts on the mock transport. Every 20 s
(`config.mockExchangeIntervalMs`), one simulated household phone "comes into range". It makes
its next scripted update and swaps envelopes with this phone through the same validation path
real peers will use. Use **Nearby → Simulate a family phone nearby** to trigger an exchange
immediately, and **Settings → Developer** to switch transports or simulate no signal.

## Bluetooth LE (planned)

- **Discovery:** advertise a Family Beacon service UUID with a short identifier that rotates
  every few minutes. Every Family Beacon phone syncs with every other (they relay each
  other's sealed traffic), so no household secret is needed to find peers, and a scanner
  can't link sightings of the same phone over time.
- **Scanning:** OS-managed low-power modes only (iOS background BLE with service filters,
  Android `SCAN_MODE_LOW_POWER` / `ScanFilter`). Never continuous active scans.
- **Data:** a GATT characteristic carries frames, chunked to the negotiated MTU. Frames are up to 8 KiB, enough for the largest sealed envelope.
- **Native homes:** `apps/mobile/ios/FamilyBeacon/Beacon/BleTransport.swift` and
  `apps/mobile/android/.../beacon/BleTransport.kt`. These buffer frames for the Rust engine.
- **Permissions:** `NSBluetoothAlwaysUsageDescription` (iOS); `BLUETOOTH_SCAN`,
  `BLUETOOTH_ADVERTISE`, and `BLUETOOTH_CONNECT` with `neverForLocation` (Android 12+). These
  are not added to the manifests yet, so the app doesn't ask for permissions it doesn't use.

## Wi-Fi Direct / peer-to-peer Wi-Fi (planned)

Much faster and much hungrier for power than BLE. Only used opportunistically, after BLE has
confirmed a household member is nearby and there's a large backlog:

- Android: Wi-Fi Aware where available, Wi-Fi P2P otherwise.
- iOS: Multipeer Connectivity (peer-to-peer Wi-Fi + Bluetooth).

## Adding a transport

1. Implement `Transport` in `crates/sync-core/src/transport/`. Return cached peers from
   `peers()`; don't start a scan there.
2. Respect `max_frame_bytes()`. The engine splits deliveries to fit.
3. Return `TransportError::PeerUnreachable` when a peer walks away. The engine drops the
   session and retries at the next encounter.
4. Add an end-to-end test in `crates/sync-core/tests/` modeled on `store_and_forward.rs`.
