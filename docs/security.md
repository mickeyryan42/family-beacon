# Privacy and security

## Principles

- **Trusted circle only.** People join a circle in person. Nothing can be discovered or joined
  over the air, and there are no public profiles.
- **Local by default.** All data lives on household phones. No account, no cloud copy, no
  analytics.
- **Honest gaps.** Where protection isn't built yet, the docs and UI say so plainly
  (keystore storage, QR pairing, and encryption in the app are listed below).

## Current state (this repo)

| Concern | Status |
| --- | --- |
| Envelope signatures (Ed25519) | Implemented in the Rust core. Every envelope is signed and verified against the author's key. |
| Sealing for the audience | Implemented. Household and neighborhood traffic uses XChaCha20-Poly1305 with the group key; direct messages use X25519 + HKDF to the recipient. |
| Relays can't read what they carry | Implemented. Relays see only the routing header (id, scope, expiry, hops, coarse priority). |
| Size hiding | Implemented. Plaintexts are padded to 512 B, 1 KB or 4 KB. |
| Membership checks on every envelope | Implemented: household roster, neighborhood invite chains, direct-message senders. |
| Neighborhood removal and key rotation | Implemented. A steward removes someone, and a new key goes to every remaining member. |
| Abuse limits | Implemented: 20 posts and 30 direct messages per person per day, 10 invites per member per day, local blocking. |
| Key storage in the OS keystore | Not started. Keys are in memory in the engine. |
| In-person pairing (QR / tap) | Not started. The app simulates household pairing and neighborhood invites in development. |
| Encrypted local database | Not started |
| The mobile app | Still runs the TypeScript mirror, which follows the same rules by audience but doesn't encrypt. Encryption reaches the app when the Rust core is bridged in. |

The neighborhood threat model and key rotation are designed in
[neighborhood-network.md](neighborhood-network.md) §8–9.

## Design

**Identity (implemented in the core; keystore storage still to do):** each person has an
Ed25519 signing key and an X25519 key for receiving direct messages. On a phone, the secret
halves will live in the OS keystore (Secure Enclave or Android Keystore). The author signs
`Envelope::signing_bytes()`, which covers everything but the signature. Readers verify
against the author's key from the household roster or the neighborhood membership certificate.

**Sealing (implemented):** every envelope leaves the device sealed for its audience. The
routing header (id, scope, expiry) is bound to the ciphertext as associated data. Hop count and
priority class are not bound, so relays can lower them. Any Family Beacon phone can carry any
sealed envelope; only the audience can open it. See
[neighborhood-network.md](neighborhood-network.md) §9.

**Pairing (to do):** adding someone to a household means holding the phones together. A QR
code (or NFC tap) carries the household id, the household key, and both people's public keys.
Neighborhood invites work the same way, carrying a signed membership certificate and the
current neighborhood key. Keys are only ever exchanged face to face.

**Radio privacy (to do, with BLE):** advertised tokens rotate so phones can't be tracked.
Someone nearby with a scanner learns only that a Family Beacon device is present.

**Removing someone:** in a neighborhood, a steward's removal triggers a new key epoch that is
sent to every remaining member (implemented). In a household, rotate the household key and
re-pair the remaining members (to do).

**At rest (to do):** SQLite encrypted with a key held in the OS keystore (e.g. SQLCipher),
backed up by the OS's device-level encryption.

## Reporting a vulnerability

Please email the maintainers privately rather than opening a public issue.
