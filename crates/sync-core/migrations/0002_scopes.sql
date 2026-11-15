-- Family Beacon on-device schema, version 2: scopes, sealed relay,
-- neighborhoods and direct messages. See docs/neighborhood-network.md.
--
-- Applied by `SqliteStore::open`, like 0001.

-- Version 1 envelopes predate sealing and can't be forwarded as-is. They are
-- dropped; check-ins and notes have expired long before anyone upgrades, and
-- meeting points are re-published from the household view on first launch.
-- (Dropped outright rather than renamed: a renamed table keeps its index
-- names, which would collide with the new indexes below.)
DROP TABLE IF EXISTS outbound_queue;
DROP TABLE envelopes;

-- Every envelope is now stored sealed (exactly as it travels), plus the
-- opened form when this device could read it. Relay traffic has no opened
-- form and lives in tier 'relay'.

CREATE TABLE envelopes (
    id              TEXT PRIMARY KEY,
    scope           TEXT NOT NULL CHECK (scope IN ('household', 'neighborhood', 'direct')),
    tier            TEXT NOT NULL CHECK (tier IN ('household', 'direct', 'neighborhood', 'relay')),
    route_tag       BLOB NOT NULL,
    priority_class  TEXT NOT NULL CHECK (priority_class IN ('low', 'normal', 'urgent')),
    expires_at      INTEGER NOT NULL,
    hops_remaining  INTEGER NOT NULL,
    received_at     INTEGER NOT NULL,
    sealed          BLOB NOT NULL,           -- encoded SealedEnvelope
    opened          BLOB,                    -- encoded Envelope, NULL for relay traffic
    -- Indexed fields from the opened envelope, NULL for relay traffic.
    author          TEXT,
    audience_id     TEXT,                    -- household, neighborhood or recipient id
    kind            TEXT,
    lamport         INTEGER,
    created_at      INTEGER
);
CREATE INDEX envelopes_expiry ON envelopes (expires_at);
CREATE INDEX envelopes_tier ON envelopes (tier, priority_class, received_at);
CREATE INDEX envelopes_audience ON envelopes (audience_id, created_at);

CREATE TABLE outbound_queue (
    envelope_id  TEXT PRIMARY KEY REFERENCES envelopes(id) ON DELETE CASCADE,
    tier         TEXT NOT NULL,
    priority_class TEXT NOT NULL,
    first_seen   INTEGER NOT NULL,
    bytes        INTEGER NOT NULL
);
CREATE INDEX outbound_send_order ON outbound_queue (tier, priority_class, first_seen);

-- This person's keys are in the OS keystore; only public halves live here.
ALTER TABLE people ADD COLUMN encryption_key BLOB;

-- The household key is wrapped by a key held in the OS keystore.
ALTER TABLE households ADD COLUMN wrapped_key BLOB;

-- Neighborhoods this device has joined.
CREATE TABLE neighborhoods (
    id                    TEXT PRIMARY KEY,
    name                  TEXT NOT NULL,
    area                  TEXT NOT NULL DEFAULT '',
    founder               TEXT NOT NULL,
    founder_identity_key  BLOB NOT NULL,
    created_at            INTEGER NOT NULL,
    joined_at             INTEGER NOT NULL
);

-- One row per key epoch; keys are wrapped like the household key.
CREATE TABLE neighborhood_keys (
    neighborhood_id  TEXT NOT NULL REFERENCES neighborhoods(id) ON DELETE CASCADE,
    epoch            INTEGER NOT NULL,
    wrapped_key      BLOB NOT NULL,
    PRIMARY KEY (neighborhood_id, epoch)
);

-- Membership certificates seen, for showing invite chains. The roster itself
-- is always recomputed from membership envelopes.
CREATE TABLE memberships (
    neighborhood_id  TEXT NOT NULL REFERENCES neighborhoods(id) ON DELETE CASCADE,
    member           TEXT NOT NULL,
    display_name     TEXT NOT NULL,
    identity_key     BLOB NOT NULL,
    encryption_key   BLOB NOT NULL,
    invited_by       TEXT,
    invited_at       INTEGER NOT NULL,
    signature        BLOB NOT NULL,
    PRIMARY KEY (neighborhood_id, member, invited_at)
);

-- Invites this device signed, counted toward the daily limit immediately.
CREATE TABLE issued_invites (
    neighborhood_id  TEXT NOT NULL REFERENCES neighborhoods(id) ON DELETE CASCADE,
    member           TEXT NOT NULL,
    issued_at        INTEGER NOT NULL,
    PRIMARY KEY (neighborhood_id, member)
);

-- People blocked on this phone only.
CREATE TABLE blocks (
    person_id   TEXT PRIMARY KEY,
    blocked_at  INTEGER NOT NULL
);

-- Per-person, per-day counters for posts and direct messages.
CREATE TABLE rate_limits (
    person_id  TEXT NOT NULL,
    bucket     TEXT NOT NULL CHECK (bucket IN ('posts', 'direct')),
    day        INTEGER NOT NULL,
    count      INTEGER NOT NULL,
    PRIMARY KEY (person_id, bucket, day)
);

INSERT INTO settings (key, value) VALUES ('relay_enabled', 'true')
    ON CONFLICT (key) DO NOTHING;

UPDATE schema_version SET version = 2;
