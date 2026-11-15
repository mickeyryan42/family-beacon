//! SQLite-backed [`EnvelopeStore`]: the on-device store.
//!
//! Opening a store applies every migration in `migrations/` that the database
//! hasn't seen yet, each in its own transaction, so a phone can upgrade
//! across several app versions in one launch and a failed upgrade leaves the
//! previous schema intact.
//!
//! Rows keep the sealed envelope exactly as it travels (so it can be
//! forwarded byte for byte) and, when this device could read it, the opened
//! envelope plus a few indexed fields. The encrypted-at-rest variant
//! (SQLCipher) will reuse this code with a keyed connection.

use std::path::Path;

use beacon_protocol::{codec, Audience, EnvelopeId, PriorityClass, Scope, UnixMillis};
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::queue::Tier;
use crate::store::{EnvelopeStore, InsertOutcome, StoreError, StoredEnvelope};

/// Schema migrations, in order. Index + 1 is the schema version.
const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0001_init.sql"),
    include_str!("../migrations/0002_scopes.sql"),
];

/// Columns read back by [`SqliteStore::row_to_entry`], in order.
const ENTRY_COLUMNS: &str = "sealed, opened, tier, received_at";

pub struct SqliteStore {
    conn: Connection,
}

impl std::fmt::Debug for SqliteStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteStore")
            .field("path", &self.conn.path())
            .finish()
    }
}

impl SqliteStore {
    /// Open (creating if needed) the database at `path` and bring its schema
    /// up to date.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let conn = Connection::open(path).map_err(backend)?;
        // WAL keeps reads cheap while a sync round writes. Must be set
        // outside a transaction.
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(backend)?;
        Self::prepare(conn)
    }

    /// A throwaway database, for tests and previews.
    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::prepare(Connection::open_in_memory().map_err(backend)?)
    }

    fn prepare(mut conn: Connection) -> Result<Self, StoreError> {
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(backend)?;
        migrate(&mut conn)?;
        Ok(Self { conn })
    }

    /// Read a value from the local `settings` table.
    pub fn setting(&self, key: &str) -> Result<Option<String>, StoreError> {
        self.conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()
            .map_err(backend)
    }

    /// Write a value to the local `settings` table.
    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), StoreError> {
        self.conn
            .execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                [key, value],
            )
            .map(|_| ())
            .map_err(backend)
    }

    /// Current schema version (the number of migrations applied).
    pub fn schema_version(&self) -> Result<usize, StoreError> {
        schema_version(&self.conn)
    }

    pub fn len(&self) -> Result<usize, StoreError> {
        self.count("SELECT COUNT(*) FROM envelopes")
    }

    pub fn is_empty(&self) -> Result<bool, StoreError> {
        Ok(self.len()? == 0)
    }

    /// Entries this device could read.
    pub fn readable_len(&self) -> Result<usize, StoreError> {
        self.count("SELECT COUNT(*) FROM envelopes WHERE opened IS NOT NULL")
    }

    fn count(&self, sql: &str) -> Result<usize, StoreError> {
        let n: i64 = self
            .conn
            .query_row(sql, [], |r| r.get(0))
            .map_err(backend)?;
        usize::try_from(n).map_err(backend)
    }

    fn row_to_entry(row: &Row<'_>) -> rusqlite::Result<RawEntry> {
        Ok(RawEntry {
            sealed: row.get(0)?,
            opened: row.get(1)?,
            tier: row.get(2)?,
            received_at: row.get(3)?,
        })
    }
}

/// A row before decoding, so codec errors surface as `StoreError`s rather
/// than being squeezed through rusqlite's error type.
struct RawEntry {
    sealed: Vec<u8>,
    opened: Option<Vec<u8>>,
    tier: String,
    received_at: i64,
}

impl RawEntry {
    fn decode(self) -> Result<StoredEnvelope, StoreError> {
        Ok(StoredEnvelope {
            sealed: codec::decode_sealed(&self.sealed).map_err(backend)?,
            opened: self
                .opened
                .map(|b| codec::decode_envelope(&b))
                .transpose()
                .map_err(backend)?,
            tier: tier_from_str(&self.tier)?,
            received_at: from_sql_time(self.received_at)?,
        })
    }
}

impl EnvelopeStore for SqliteStore {
    fn insert(&mut self, entry: StoredEnvelope) -> Result<InsertOutcome, StoreError> {
        let sealed = &entry.sealed;
        let opened = entry.opened.as_ref();
        let opened_bytes = opened
            .map(codec::encode_envelope)
            .transpose()
            .map_err(backend)?;
        let changed = self
            .conn
            .execute(
                "INSERT INTO envelopes (
                    id, scope, tier, route_tag, priority_class, expires_at, hops_remaining,
                    received_at, sealed, opened, author, audience_id, kind, lamport, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
                 ON CONFLICT (id) DO NOTHING",
                params![
                    sealed.id.to_string(),
                    scope_str(sealed.scope),
                    tier_str(entry.tier),
                    sealed.route_tag,
                    priority_str(sealed.priority_class),
                    to_sql_time(sealed.expires_at)?,
                    sealed.hops_remaining,
                    to_sql_time(entry.received_at)?,
                    codec::encode_sealed(sealed).map_err(backend)?,
                    opened_bytes,
                    opened.map(|e| e.author.to_string()),
                    opened.map(|e| audience_id(&e.audience)),
                    opened.map(|e| e.payload.kind()),
                    opened.map(|e| to_sql_time(e.lamport)).transpose()?,
                    opened.map(|e| to_sql_time(e.created_at)).transpose()?,
                ],
            )
            .map_err(backend)?;
        Ok(if changed == 0 {
            InsertOutcome::Duplicate
        } else {
            InsertOutcome::Inserted
        })
    }

    fn get(&self, id: &EnvelopeId) -> Result<Option<StoredEnvelope>, StoreError> {
        self.conn
            .query_row(
                &format!("SELECT {ENTRY_COLUMNS} FROM envelopes WHERE id = ?1"),
                [id.to_string()],
                Self::row_to_entry,
            )
            .optional()
            .map_err(backend)?
            .map(RawEntry::decode)
            .transpose()
    }

    fn contains(&self, id: &EnvelopeId) -> Result<bool, StoreError> {
        self.conn
            .query_row(
                "SELECT 1 FROM envelopes WHERE id = ?1",
                [id.to_string()],
                |_| Ok(()),
            )
            .optional()
            .map(|found| found.is_some())
            .map_err(backend)
    }

    fn remove(&mut self, id: &EnvelopeId) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM envelopes WHERE id = ?1", [id.to_string()])
            .map(|_| ())
            .map_err(backend)
    }

    fn ids(&self, now: UnixMillis) -> Result<Vec<EnvelopeId>, StoreError> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT id FROM envelopes WHERE expires_at > ?1 ORDER BY id")
            .map_err(backend)?;
        let rows = stmt
            .query_map([to_sql_time(now)?], |r| r.get::<_, String>(0))
            .map_err(backend)?;
        rows.map(|id| EnvelopeId::parse(&id.map_err(backend)?).map_err(backend))
            .collect()
    }

    fn all(&self, now: UnixMillis) -> Result<Vec<StoredEnvelope>, StoreError> {
        let mut stmt = self
            .conn
            .prepare_cached(&format!(
                "SELECT {ENTRY_COLUMNS} FROM envelopes WHERE expires_at > ?1
                 ORDER BY received_at, id"
            ))
            .map_err(backend)?;
        let rows = stmt
            .query_map([to_sql_time(now)?], Self::row_to_entry)
            .map_err(backend)?;
        rows.map(|raw| raw.map_err(backend)?.decode()).collect()
    }

    fn prune_expired(&mut self, now: UnixMillis) -> Result<usize, StoreError> {
        self.conn
            .execute(
                "DELETE FROM envelopes WHERE expires_at <= ?1",
                [to_sql_time(now)?],
            )
            .map_err(backend)
    }
}

fn migrate(conn: &mut Connection) -> Result<(), StoreError> {
    let current = schema_version(conn)?;
    if current > MIGRATIONS.len() {
        return Err(StoreError::Backend(format!(
            "database schema v{current} is newer than this app (v{}); update the app",
            MIGRATIONS.len()
        )));
    }
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(current) {
        let tx = conn.transaction().map_err(backend)?;
        tx.execute_batch(sql)
            .map_err(|e| StoreError::Backend(format!("migration {} failed: {e}", index + 1)))?;
        tx.commit().map_err(backend)?;
        log::info!("database migrated to schema v{}", index + 1);
    }
    Ok(())
}

fn schema_version(conn: &Connection) -> Result<usize, StoreError> {
    let has_table: bool = conn
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'schema_version')",
            [],
            |r| r.get(0),
        )
        .map_err(backend)?;
    if !has_table {
        return Ok(0);
    }
    let version: Option<i64> = conn
        .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
        .map_err(backend)?;
    usize::try_from(version.unwrap_or(0)).map_err(backend)
}

fn backend(e: impl std::fmt::Display) -> StoreError {
    StoreError::Backend(e.to_string())
}

fn to_sql_time(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(backend)
}

fn from_sql_time(value: i64) -> Result<u64, StoreError> {
    u64::try_from(value).map_err(backend)
}

fn scope_str(scope: Scope) -> &'static str {
    match scope {
        Scope::Household => "household",
        Scope::Neighborhood => "neighborhood",
        Scope::Direct => "direct",
    }
}

fn priority_str(priority: PriorityClass) -> &'static str {
    match priority {
        PriorityClass::Low => "low",
        PriorityClass::Normal => "normal",
        PriorityClass::Urgent => "urgent",
    }
}

fn tier_str(tier: Tier) -> &'static str {
    match tier {
        Tier::Household => "household",
        Tier::Direct => "direct",
        Tier::Neighborhood => "neighborhood",
        Tier::Relay => "relay",
    }
}

fn tier_from_str(s: &str) -> Result<Tier, StoreError> {
    Tier::ALL
        .into_iter()
        .find(|t| tier_str(*t) == s)
        .ok_or_else(|| StoreError::Backend(format!("unknown tier {s:?}")))
}

fn audience_id(audience: &Audience) -> String {
    match audience {
        Audience::Household { household_id } => household_id.to_string(),
        Audience::Neighborhood { neighborhood_id } => neighborhood_id.to_string(),
        Audience::Direct { recipient } => recipient.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::contract;

    fn mem() -> SqliteStore {
        SqliteStore::open_in_memory().unwrap()
    }

    #[test]
    fn sqlite_store_meets_the_contract() {
        contract::duplicate_inserts_are_ignored(&mut mem());
        contract::expired_envelopes_are_hidden_and_pruned(&mut mem());
        contract::ids_are_sorted_and_all_is_in_receive_order(&mut mem());
    }

    #[test]
    fn settings_round_trip() {
        let store = mem();
        assert_eq!(store.setting("profile").unwrap(), None);
        store.set_setting("profile", "a").unwrap();
        store.set_setting("profile", "b").unwrap();
        assert_eq!(store.setting("profile").unwrap().as_deref(), Some("b"));
    }

    #[test]
    fn applies_every_migration_once() {
        let store = mem();
        assert_eq!(store.schema_version().unwrap(), MIGRATIONS.len());
        let relay: String = store
            .conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'relay_enabled'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(relay, "true");
    }

    #[test]
    fn persists_across_reopen_and_refuses_a_newer_schema() {
        let path = std::env::temp_dir().join(format!("beacon-store-{}.db", EnvelopeId::new()));
        let e = contract::entry(1_000, 5);
        {
            let mut store = SqliteStore::open(&path).unwrap();
            store.insert(e.clone()).unwrap();
        }
        {
            // Reopening must not re-run migrations or lose data.
            let store = SqliteStore::open(&path).unwrap();
            assert_eq!(store.schema_version().unwrap(), MIGRATIONS.len());
            assert_eq!(store.all(0).unwrap(), vec![e]);
            assert_eq!(store.len().unwrap(), 1);
            assert_eq!(store.readable_len().unwrap(), 0);
            store
                .conn
                .execute("UPDATE schema_version SET version = 99", [])
                .unwrap();
        }
        let err = SqliteStore::open(&path).unwrap_err();
        assert!(err.to_string().contains("newer than this app"), "{err}");
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }
}
