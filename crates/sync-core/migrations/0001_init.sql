-- Family Beacon on-device schema (SQLite), version 1.
--
-- Applied in order by `SqliteStore::open` (sync-core/src/sqlite_store.rs),
-- each migration inside one transaction. Connection pragmas (WAL, foreign
-- keys) are set by the store itself, because SQLite refuses to change the
-- journal mode inside a transaction.
--
-- Design notes:
-- * `envelopes` is the source of truth. Everything the UI shows is derived by
--   folding envelopes (see sync-core/src/conflict.rs), so a corrupt view can
--   always be rebuilt.
-- * The full encoded envelope is kept in `body` so it can be forwarded
--   byte-for-byte; the other columns are indexes over it.
-- * No table holds anything from a server. All data is local by default.

CREATE TABLE IF NOT EXISTS schema_version (
    version INTEGER NOT NULL
);
INSERT INTO schema_version (version) VALUES (1);

-- This installation.
CREATE TABLE IF NOT EXISTS device (
    device_id      TEXT PRIMARY KEY,
    person_id      TEXT NOT NULL,
    household_id   TEXT NOT NULL,
    lamport        INTEGER NOT NULL DEFAULT 0,
    created_at     INTEGER NOT NULL
);

-- The trusted circle, as agreed in person during onboarding.
CREATE TABLE IF NOT EXISTS households (
    id    TEXT PRIMARY KEY,
    name  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS people (
    id            TEXT PRIMARY KEY,
    household_id  TEXT NOT NULL REFERENCES households(id) ON DELETE CASCADE,
    display_name  TEXT NOT NULL,
    role          TEXT NOT NULL CHECK (role IN ('adult', 'child', 'caregiver')),
    public_key    BLOB
);

CREATE TABLE IF NOT EXISTS envelopes (
    id              TEXT PRIMARY KEY,
    household_id    TEXT NOT NULL,
    author          TEXT NOT NULL,
    origin_device   TEXT NOT NULL,
    kind            TEXT NOT NULL,
    priority        INTEGER NOT NULL,
    lamport         INTEGER NOT NULL,
    created_at      INTEGER NOT NULL,
    expires_at      INTEGER NOT NULL,
    hops_remaining  INTEGER NOT NULL,
    received_at     INTEGER NOT NULL,
    body            BLOB NOT NULL
);
CREATE INDEX IF NOT EXISTS envelopes_expiry ON envelopes (expires_at);
CREATE INDEX IF NOT EXISTS envelopes_order ON envelopes (lamport, created_at, id);

-- What this device is willing to pass on. A subset of `envelopes`.
CREATE TABLE IF NOT EXISTS outbound_queue (
    envelope_id  TEXT PRIMARY KEY REFERENCES envelopes(id) ON DELETE CASCADE,
    priority     INTEGER NOT NULL,
    created_at   INTEGER NOT NULL,
    enqueued_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS outbound_send_order ON outbound_queue (priority DESC, created_at ASC);

-- Small key/value bag for local preferences (theme, transport mode, ...).
CREATE TABLE IF NOT EXISTS settings (
    key    TEXT PRIMARY KEY,
    value  TEXT NOT NULL
);
