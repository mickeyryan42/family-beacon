# Design: Scopes and neighborhood networks

| | |
| --- | --- |
| **Status** | Implemented (protocol v2). Section 16 lists where the build differs from this draft. |
| **Date** | 2026-11-14 |
| **Affects** | `crates/crypto`, `crates/protocol`, `crates/sync-core`, `apps/mobile`, `docs/security.md` |

## 1. Summary

Today a Family Beacon network is one household. Phones only exchange updates with members of
their own household, which keeps family data private but limits how far updates can travel.

This design grows the network to neighborhood scale without weakening family privacy. It does
that by separating **who can carry a message** from **who can read it**:

- Every message gets a **scope**: `household`, `neighborhood`, or `direct`. The scope decides
  who can read it.
- Every Family Beacon phone **relays sealed messages for everyone**. A phone carries other
  people's encrypted traffic but can't read it, so reach grows with every install.
- **Neighborhood networks** grow through **invitation chains**: A invites B, B invites C. The
  network can grow exponentially, and every member can be traced back through who invited them.

The outcome: a whole neighborhood can share outage information and message each other, and a
family's check-ins are still only readable by that family.

## 2. Goals

1. **Neighborhood reach.** An update can cross a neighborhood by hopping between any phones
   running the app, not only family phones.
2. **Exponential, accountable growth.** Members can invite others into a neighborhood network,
   and every membership traces back through a signed invite chain.
3. **Talk beyond your household.** Members of the same neighborhood can post to everyone, or
   send a private message to one person.
4. **Family privacy unchanged.** Household check-ins and meeting points are readable only by
   household members, however large the surrounding network gets.
5. **Same constraints as today.** No cellular, no internet, no cloud. Battery and storage stay
   bounded on every phone.

## 3. Non-goals

- **Instant chat.** Delivery is store-and-forward and can take minutes to hours. The product
  must say so plainly.
- **City-wide or open public networks.** Neighborhoods are bounded and invite-only. An open
  "public alerts" channel is a possible later addition; see §14.
- **Phone numbers as identity.** They can't be verified offline. They may be stored as optional
  contact details (for Call / Text when there is signal), never as identity.
- **Anonymity from your own neighborhood.** Neighborhood members see each other's display
  names and who invited whom. The privacy goal is protection from outsiders and non-members.

## 4. Principles carried over

- **Trusted circle for family data.** Membership of a household is still set in person only.
- **Local by default.** Nothing leaves the device except over nearby radio links.
- **Privacy by construction.** A relay sees only what it needs to route a message.
- **Battery is a feature.** New traffic has explicit budgets. Your own household's traffic is
  always served first.
- **Simple for households.** A family that never joins a neighborhood sees no difference,
  except that its updates travel further.

## 5. Concepts

| Term | Meaning |
| --- | --- |
| **Person** | Someone with an identity key pair (or a key-less member, such as a child without a phone, checked in for by others). |
| **Device** | One installation, holding a device key that is linked to its owner's person key. |
| **Household** | Private group, joined in person. Has a symmetric **household key**. |
| **Neighborhood** | Invite-only group of households and individuals. Has a **neighborhood key** and a set of **stewards**. |
| **Steward** | A neighborhood member allowed to remove people. The founder is the first steward. |
| **Scope** | Who can read an envelope: `household`, `neighborhood`, or `direct`. |
| **Relay** | Any Family Beacon phone carrying an envelope it can't read. |

## 6. Scopes

| Scope | Readable by | Encrypted with | Typical payloads | Default TTL | Default hops |
| --- | --- | --- | --- | --- | --- |
| `household` | Household members | Household key | Check-ins, meeting points, family notes | 2 d (90 d for plans) | 12 |
| `neighborhood` | Neighborhood members | Neighborhood key (current epoch) | Alerts, offers and needs, opt-in "all safe" | 1 d | 16 |
| `direct` | One recipient | Recipient's public key (X25519 sealed box) | Private message, "Are you OK?" request | 2 d | 16 |

All three scopes are carried by **any** phone as sealed relay traffic. Readability is the only
thing a scope controls.

Hop limits rise from today's 6 because relays are now any phone, not just family phones.
Section 10 sets per-scope budgets so higher hop limits can't flood the mesh.

### 6.1 What each scope can carry

- **`household`:** everything that exists today (`check_in`, `rendezvous_upsert`,
  `rendezvous_remove`, `note`). No change in meaning.
- **`neighborhood`:**
  - `alert`: a category (power, water, road, medical, fire, other), a short text, and an
    optional coarse location.
  - `offer` / `need`: "Generator at #12 can charge phones", "Need insulin fridge space".
  - `household_status`: an opt-in "The Lee household: everyone safe". It never includes
    individual members, notes, or locations.
  - `withdraw`: retracts the author's own earlier post.
  - Membership events (invites, removals, key rotations); see §7.
- **`direct`:**
  - `message`: short text.
  - `wellness_request`: "Are you OK?", which the recipient can answer with one tap.
  - `wellness_reply`.

Family check-ins **never** leave household scope automatically. Sharing anything with the
neighborhood is always a deliberate, separate action.

## 7. Identity, joining, and growth

### 7.1 Keys

The keys are planned in `docs/security.md`; this design depends on them landing first.

- **Person identity key** (Ed25519, kept in the OS keystore). Signs everything the person
  authors.
- **Device key**, linked to the person key by a signed device certificate. This handles one
  person with two phones.
- **Encryption key** (X25519), derived alongside the person key and published with it, so others
  can seal direct messages to that person.
- **Household key**, shared at in-person pairing.
- **Neighborhood key**, a symmetric key per epoch. It rotates after removals (see §8).

### 7.2 Joining a household

Unchanged: in person only, by QR code or tap, as described in `docs/security.md`.

### 7.3 Joining a neighborhood: invitation chains

1. A **founder** creates a neighborhood: a name, a short description, an approximate area in
   words ("Elm & Oak streets"), and a key. The founder becomes the first steward.
2. Any member can **invite** someone **in person**, by QR code. The invite is a signed
   membership certificate:

   ```text
   MembershipCert {
     neighborhood_id, epoch,
     member_person_id, member_identity_key, member_encryption_key,
     display_name,
     invited_by: person_id, invited_at,
     signature: inviter's identity key over all of the above
   }
   ```

   The invitee receives the neighborhood key during the same QR exchange.
3. The new member's phone broadcasts the certificate as a `neighborhood` membership event, so
   every member learns who joined and who vouched for them.
4. **Growth is exponential and accountable.** Everyone invited can invite others, and any
   member's chain back to the founder can be shown.

**Why in person only (v1):** with no server and no signal, meeting face to face is the only
trustworthy way to hand over keys. It also slows down mass sign-ups by spammers. Remote invites
(sent as a `direct` message to someone already reachable on the mesh) are a later option; see
§14.

**Households in a neighborhood:** each person is a member, not each household. A household that
joins together is just several invites done at once, and the UI can do them as a batch.

### 7.4 Contacting someone outside your household

A can reach C if both are members of the same neighborhood:

- **Post to everyone** with a `neighborhood` envelope.
- **Message C privately** with a `direct` envelope sealed to C's encryption key, which A learned
  from C's membership certificate. Every phone on the way relays it without being able to read
  it.

A can never read C's household traffic.

## 8. Removal, moderation, and key rotation

Invite chains make growth easy, so removal has to be just as clear.

- **Stewards** can publish a `member_removed` event, signed by the steward, giving the person
  and a reason code. The founder can promote other stewards with `steward_added`.
- **Optional cascade.** A removal can also remove everyone the person invited who has not since
  been re-vouched for by another member. This is the main defence against one bad invite
  bringing in many.
- **Key rotation.** After a removal, a steward starts a new **epoch**: a fresh neighborhood key,
  sent to each remaining member as a `direct` envelope. Posts are always encrypted with the
  newest epoch key a phone holds. Older keys are kept only long enough to read in-flight posts.
- **Local blocking.** Any member can block a person on their own phone. That hides the person's
  posts and stops relaying their neighborhood posts from this phone. Blocking doesn't remove
  anyone from the network.
- **Accept removals after the fact.** A phone that receives a removal after already showing the
  person's posts hides those posts from then on.

**Cost of rotation:** one `direct` envelope per remaining member. For a 300-person neighborhood
that is about 300 small envelopes, roughly 60 KB in total. That's acceptable but not free, so
rotations happen at most daily, batching several removals together. Sender-key schemes
(MLS-style group key agreement) are noted as a future option in §14.

**Expectation to set in the UI:** a neighborhood is semi-public. Anyone invited can read
neighborhood posts. The UI labels this clearly and never puts family data there.

## 9. Envelope format: sealed relay

Today every phone reads the whole envelope. To let any phone relay, the envelope splits into an
outer **routing header** that relays can see, and a sealed **body** that only readers can open.

```text
SealedEnvelope (what relays see)
  version            u16
  id                 EnvelopeId        // dedup across the whole mesh
  scope              household | neighborhood | direct
  route_tag          [u8; 16]          // rotating tag, see below
  expires_at         UnixMillis
  hops_remaining     u8
  priority_class     urgent | normal | low
  body               bytes             // AEAD ciphertext of InnerEnvelope

InnerEnvelope (what readers see after decrypting)
  household_id / neighborhood_id / recipient
  author, origin_device
  created_at, lamport
  payload
  signature          // author's identity key over the inner fields
```

- **`route_tag`** lets a phone recognize traffic meant for it without decrypting everything. It
  is derived from the scope key and the current time window (for example, a 6-hour window). That
  makes it unlinkable across windows for anyone without the key. For `direct` messages it is
  derived from the recipient's encryption key, so only the recipient recognizes it.
- **`priority_class`** is deliberately coarse, so relays can make queueing decisions without
  learning the content. `urgent` is used only for need-help check-ins and `medical` / `fire`
  alerts. Readers re-check the claim after decrypting, and an envelope that claims `urgent`
  without qualifying loses its priority on the next hop.
- **What relays still learn:** that an envelope exists, its rough size, its scope, its priority
  class and its expiry. Envelopes are padded to fixed sizes (512 B, 1 KB, 4 KB) to blur size.
- **Conflict rules don't change.** `conflict.rs` keeps working on decrypted inner envelopes,
  exactly as today.

## 10. Sync protocol and budgets

### 10.1 Prioritization

When two phones meet, each sends in this order. Within each tier, the existing queue ordering
applies (highest priority first, then oldest first).

1. `urgent`, in any scope
2. Own household
3. `direct` envelopes addressed to the peer (recognized by route tag)
4. Own neighborhood
5. Sealed relay traffic for others

### 10.2 Set reconciliation

Today's `Hello` lists every envelope id the phone holds. That's fine for a household, but
neighborhood stores may hold thousands of envelopes. `Hello` changes to send:

- a **compact summary per scope tier**, for example a Bloom filter with about a 1% false
  positive rate (~1.2 KB per 1,000 ids), or range-based set reconciliation for large sets, and
- a **since-last-sync watermark per peer**, so phones that meet often only compare what's new.

### 10.3 Budgets (initial values, to tune with field data)

| Budget | Household | Neighborhood | Direct | Relay (others) |
| --- | --- | --- | --- | --- |
| Queue entries | 500 | 2,000 | 200 | 1,000 |
| Storage cap | 2 MB | 4 MB | 1 MB | 2 MB |
| Per-session send cap | 200 | 100 | 50 | 100 |
| Per-sender rate limit | none | 20 posts/day | 30 messages/day | enforced as neighborhood/direct |

- **Per-sender limits** are enforced by every relay that can identify the sender (members of the
  same neighborhood). Non-members can only enforce the per-tier caps.
- **Eviction** follows today's queue rule: lowest priority and oldest go first. Relay traffic
  for others is evicted before anything this phone can read.
- **Battery:** total radio time per encounter is capped, and the existing backoff applies to
  everything. A busy neighborhood increases how much is exchanged per encounter, not how often
  the radio wakes up.

### 10.4 Expected propagation

These are rough figures, to set expectations rather than promise them. Bluetooth reaches about
10–50 m per hop. In a typical suburban street with a few dozen active phones, a neighborhood
alert reaches most members within **30–120 minutes** in daytime, and much more slowly overnight.
The UI shows the age of every update and never implies it was delivered instantly.

## 11. User experience

- **Every compose screen states its scope**, with distinct labels and colors: "Family only",
  "Elm & Oak neighborhood (142 people)", "Private to Dana".
- **Separate tabs.** The family view (today's tabs) stays as it is. A new **Neighborhood** tab
  holds alerts, offers and needs, and the people directory.
- **Joining is explicit.** "Join a neighborhood" and "Invite a neighbor" show a QR flow and
  explain what neighbors can and can't see before anything is shared.
- **"All safe" is opt-in per post.** Sharing family status with the neighborhood is a separate
  button with a preview of exactly what will be shared.
- **Relay is on by default, with an off switch.** Settings: "Help carry neighbors' sealed
  messages (uses a little extra battery)". It is on by default because the network only works
  if most phones relay.
- **Delivery honesty.** Every post says "Shared from this phone · reaches neighbors as people
  move around". There are no read receipts across the mesh.

## 12. Changes to the existing code

| Area | Change |
| --- | --- |
| `crates/protocol` | Add `Scope`, `SealedEnvelope` / `InnerEnvelope`, new payloads (`alert`, `offer`, `need`, `household_status`, `withdraw`, `message`, `wellness_*`, membership events) and `MembershipCert`. Per-scope TTL and hop defaults. Validation split into outer (relay) and inner (reader) checks. |
| `crates/sync-core` | Per-scope queues and budgets, the tiered send order, per-sender rate limits, summary-based `Hello`, route-tag matching, and a neighborhood membership state (members, stewards, removals, epochs). `Signer` / `Verifier` / `FrameCipher` hooks become real implementations. |
| `crates/sync-core/migrations` | `0002`: a `scope` column, a sealed `body`, a `neighborhoods` table, a `memberships` table (certificates, invite chain), a `blocks` table and a `rate_limits` table. |
| `apps/mobile` | Neighborhood tab, join and invite QR flows, scope labels on compose screens, block and report actions, the relay toggle. The mock transport gains simulated neighbors and sealed relay traffic. |
| `docs/security.md` | Add the neighborhood threat model and key rotation. |

## 13. Rollout

1. **Protocol groundwork.** Add `scope` (household only for now), the outer/inner envelope
   split, and padding. Bump `PROTOCOL_VERSION`. No user-visible change.
2. **Identity (prerequisite).** Real keys, signing, household pairing, and the household key
   (the plan in `docs/security.md`).
3. **Sealed relay.** Any Family Beacon phone carries other households' sealed envelopes. This
   immediately extends family reach, with no new UI beyond the relay toggle.
4. **Neighborhoods v1.** Create, invite in person, alerts, offers and needs, opt-in "all safe",
   stewards, removal with rotation, local blocking.
5. **Direct messages.** Private messages and "Are you OK?" requests.
6. **Scale work.** Range-based set reconciliation, budget tuning from field tests, and
   compact binary encoding for Bluetooth.

## 14. Alternatives considered

- **Transitive trust ("B added C, so A trusts C").** Rejected. Within a few hops this reaches
  most of a town and exposes family check-ins, including children's movements, to strangers. It
  conflicts directly with the trusted-circle principle. Invitation chains keep the growth and
  drop the exposure.
- **Open public mesh with no membership.** Rejected for v1. It invites spam and
  misinformation, and there's no one to remove bad actors. A read-only "public alerts" scope
  signed by recognized local bodies (fire service, utility) could be added later.
- **Phone numbers as identity.** Rejected. They can't be verified without a carrier network,
  and they leak personal data across the mesh. They remain an optional contact detail.
- **MLS / sender keys for neighborhood encryption.** Deferred. They give better forward secrecy
  and cheaper rotation for large groups, but they're complex and need more rounds of message
  exchange than an intermittent mesh handles well. A symmetric key per epoch is simpler and
  good enough for semi-public neighborhood content.
- **Remote invites over the mesh.** Deferred. They're convenient, but lose the in-person check.
  If added, they should require the inviter to have met the invitee in person before (holding
  their key from an earlier exchange).

## 15. Product decisions

Every recommendation from the draft was accepted as-is.
| # | Question | Decision (all recommendations adopted, 2026-11-14) |
| --- | --- | --- |
| 1 | Is joining a neighborhood opt-in for each person, or does the whole household join together? | Per person, with a "whole household" shortcut |
| 2 | Is relaying for others on by default? | On, with a clear off switch |
| 3 | Can members see the full member list and invite chains? | Names yes; invite chains visible to stewards only |
| 4 | Who can remove people? | Stewards. Founder appoints stewards. |
| 5 | Does removal cascade to invitees by default? | Ask the steward each time; default "no" |
| 6 | Limit on how many people one member can invite? | 10 per day per member |
| 7 | Can someone belong to more than one neighborhood? | Yes, up to 3 |
| 8 | Do neighborhood alerts carry locations? | Coarse only ("Elm St, 400 block"), never GPS |
| 9 | Is there a size cap per neighborhood? | Soft cap ~500 members, then suggest splitting |
| 10 | Should a household's "all safe" ever be shared automatically? | No, always a deliberate tap |

## 16. Implementation notes

What was built, and where it differs from the draft above.

**Where things live**

| Piece | Code |
| --- | --- |
| Keys, signatures, sealing, route tags, padding | `crates/crypto` (Ed25519, X25519, HKDF-SHA256, XChaCha20-Poly1305, HMAC-SHA256) |
| Scopes, audiences, payloads, sealed envelope, `Hello` summary, validation | `crates/protocol` (protocol v2) |
| Sealing and opening | `crates/sync-core/src/sealing.rs` |
| Membership roster, stewards, removals, rate limits | `crates/sync-core/src/neighborhood.rs` |
| Neighborhood feed and direct inbox | `crates/sync-core/src/feed.rs` |
| Tiered queue and budgets | `crates/sync-core/src/queue.rs` |
| Engine (create, invite, accept, post, send, remove, rotate, relay) | `crates/sync-core/src/engine.rs` |
| End-to-end scenarios | `crates/sync-core/tests/store_and_forward.rs` |
| Storage schema | `crates/sync-core/migrations/0002_scopes.sql` |
| App UI | Neighbors tab, `NeighborhoodPeopleScreen`, `ConversationScreen`, relay toggle in Settings |

**Deliberate differences from the draft**

- **Route tags are per envelope, not per time window (§9).** The tag is an HMAC over
  the envelope's id and expiry. Readers check one tag per key they hold, with no window
  arithmetic, and to outsiders no two tags are related. A direct tag is keyed with the
  recipient's *public* encryption key, so neighborhood members who know that key could test
  whether an envelope is addressed to that person. Outsiders can't.
- **The key rotates immediately on every removal (§8)**, rather than being batched daily.
  That's simpler and closes the window sooner. If neighborhoods grow large enough for
  rotation traffic to matter, add batching.
- **`Hello` sends exact ids for small stores (§10.2).** At 64 ids or fewer, it lists them
  exactly, so household-sized syncs never skip an envelope because of a Bloom false
  positive. Above that it uses the 1% Bloom filter. The per-peer watermark isn't built yet.
- **Direct envelopes are sent before neighborhood posts (§10.1)**, but all of them, not only
  those addressed to the peer, because a peer's identity isn't known at the radio level.
  Urgent envelopes still go first.
- **A phone counts its own signed invites immediately** toward the 10-a-day limit, without
  waiting for the newcomer's join to come back over the mesh. Validators also enforce the
  limit from the certificates they see.
- **Removing someone without cascade keeps the people they invited** (they were
  legitimately invited at the time). Invites a removed person signs after their removal are
  never valid.
- **Delivered direct messages aren't relayed further.** A phone that receives a direct
  message addressed to it doesn't forward copies, which saves radio time.

**Still to do**

- Bridge the Rust engine into the app (the app still runs the TypeScript mirror, which
  follows the same rules but doesn't encrypt).
- Real QR invites (camera). Until then, joining and inviting are simulated in development
  builds.
- SQLite store and OS-keystore key storage.
- Per-peer sync watermarks and range-based set reconciliation (rollout phase 6).
