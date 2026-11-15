//! Local envelope storage.
//!
//! On device this is backed by SQLite (see `migrations/0001_init.sql` and
//! `0002_scopes.sql`). The trait is intentionally narrow so the SQLite
//! implementation, the in-memory one used in tests and the mock app, and any
//! future encrypted store all behave the same.
//!
//! Every entry keeps the sealed form (so it can be forwarded byte for byte)
//! and, when this device could read it, the opened envelope.

use std::collections::HashMap;

use beacon_protocol::{Envelope, EnvelopeId, SealedEnvelope, UnixMillis};
use thiserror::Error;

use crate::queue::Tier;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("storage backend failure: {0}")]
    Backend(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredEnvelope {
    pub sealed: SealedEnvelope,
    /// `None` for relay traffic this device can't read.
    pub opened: Option<Envelope>,
    pub tier: Tier,
    pub received_at: UnixMillis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertOutcome {
    Inserted,
    /// Already held. Expected and common in store-and-forward meshes.
    Duplicate,
}

pub trait EnvelopeStore {
    fn insert(&mut self, entry: StoredEnvelope) -> Result<InsertOutcome, StoreError>;
    fn get(&self, id: &EnvelopeId) -> Result<Option<StoredEnvelope>, StoreError>;
    fn contains(&self, id: &EnvelopeId) -> Result<bool, StoreError>;
    fn remove(&mut self, id: &EnvelopeId) -> Result<(), StoreError>;
    /// IDs of every unexpired envelope held, for the sync `Hello`.
    fn ids(&self, now: UnixMillis) -> Result<Vec<EnvelopeId>, StoreError>;
    /// Every unexpired entry, in the order it was received. Used to rebuild
    /// state on launch.
    fn all(&self, now: UnixMillis) -> Result<Vec<StoredEnvelope>, StoreError>;
    /// Delete expired envelopes. Returns how many were removed.
    fn prune_expired(&mut self, now: UnixMillis) -> Result<usize, StoreError>;
}

#[derive(Debug, Default)]
pub struct MemoryStore {
    entries: HashMap<EnvelopeId, StoredEnvelope>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Entries this device could read.
    pub fn readable_len(&self) -> usize {
        self.entries.values().filter(|e| e.opened.is_some()).count()
    }
}

impl EnvelopeStore for MemoryStore {
    fn insert(&mut self, entry: StoredEnvelope) -> Result<InsertOutcome, StoreError> {
        if self.entries.contains_key(&entry.sealed.id) {
            return Ok(InsertOutcome::Duplicate);
        }
        self.entries.insert(entry.sealed.id, entry);
        Ok(InsertOutcome::Inserted)
    }

    fn get(&self, id: &EnvelopeId) -> Result<Option<StoredEnvelope>, StoreError> {
        Ok(self.entries.get(id).cloned())
    }

    fn contains(&self, id: &EnvelopeId) -> Result<bool, StoreError> {
        Ok(self.entries.contains_key(id))
    }

    fn remove(&mut self, id: &EnvelopeId) -> Result<(), StoreError> {
        self.entries.remove(id);
        Ok(())
    }

    fn ids(&self, now: UnixMillis) -> Result<Vec<EnvelopeId>, StoreError> {
        let mut ids: Vec<_> = self
            .entries
            .values()
            .filter(|e| !e.sealed.is_expired(now))
            .map(|e| e.sealed.id)
            .collect();
        ids.sort();
        Ok(ids)
    }

    fn all(&self, now: UnixMillis) -> Result<Vec<StoredEnvelope>, StoreError> {
        let mut all: Vec<_> = self
            .entries
            .values()
            .filter(|e| !e.sealed.is_expired(now))
            .cloned()
            .collect();
        all.sort_by_key(|e| (e.received_at, e.sealed.id));
        Ok(all)
    }

    fn prune_expired(&mut self, now: UnixMillis) -> Result<usize, StoreError> {
        let before = self.entries.len();
        self.entries.retain(|_, e| !e.sealed.is_expired(now));
        Ok(before - self.entries.len())
    }
}

/// Behavior every [`EnvelopeStore`] must share, run against each backend.
#[cfg(test)]
pub(crate) mod contract {
    use super::*;
    use beacon_protocol::{PriorityClass, Scope};

    pub fn entry(expires_at: UnixMillis, received_at: UnixMillis) -> StoredEnvelope {
        StoredEnvelope {
            sealed: SealedEnvelope {
                version: beacon_protocol::PROTOCOL_VERSION,
                id: EnvelopeId::new(),
                scope: Scope::Neighborhood,
                route_tag: vec![7; 16],
                expires_at,
                hops_remaining: 3,
                priority_class: PriorityClass::Urgent,
                body: vec![1, 2, 3, 250],
            },
            opened: None,
            tier: Tier::Relay,
            received_at,
        }
    }

    pub fn duplicate_inserts_are_ignored(store: &mut impl EnvelopeStore) {
        let e = entry(100, 1);
        assert_eq!(store.insert(e.clone()).unwrap(), InsertOutcome::Inserted);
        assert_eq!(store.insert(e.clone()).unwrap(), InsertOutcome::Duplicate);
        assert!(store.contains(&e.sealed.id).unwrap());
        assert_eq!(store.get(&e.sealed.id).unwrap(), Some(e.clone()));
        store.remove(&e.sealed.id).unwrap();
        assert!(!store.contains(&e.sealed.id).unwrap());
        assert_eq!(store.get(&e.sealed.id).unwrap(), None);
    }

    pub fn expired_envelopes_are_hidden_and_pruned(store: &mut impl EnvelopeStore) {
        let short = entry(100, 1);
        let long = entry(200, 2);
        store.insert(short.clone()).unwrap();
        store.insert(long.clone()).unwrap();
        assert_eq!(store.ids(99).unwrap().len(), 2);
        assert_eq!(store.ids(100).unwrap(), vec![long.sealed.id]);
        assert_eq!(store.all(100).unwrap(), vec![long.clone()]);
        assert_eq!(store.prune_expired(100).unwrap(), 1);
        assert!(!store.contains(&short.sealed.id).unwrap());
        assert!(store.contains(&long.sealed.id).unwrap());
    }

    pub fn ids_are_sorted_and_all_is_in_receive_order(store: &mut impl EnvelopeStore) {
        let entries: Vec<_> = [30, 10, 20].iter().map(|r| entry(1_000, *r)).collect();
        for e in &entries {
            store.insert(e.clone()).unwrap();
        }
        let mut expected_ids: Vec<_> = entries.iter().map(|e| e.sealed.id).collect();
        expected_ids.sort();
        assert_eq!(store.ids(0).unwrap(), expected_ids);
        let received: Vec<_> = store
            .all(0)
            .unwrap()
            .iter()
            .map(|e| e.received_at)
            .collect();
        assert_eq!(received, vec![10, 20, 30]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_meets_the_contract() {
        contract::duplicate_inserts_are_ignored(&mut MemoryStore::new());
        contract::expired_envelopes_are_hidden_and_pruned(&mut MemoryStore::new());
        contract::ids_are_sorted_and_all_is_in_receive_order(&mut MemoryStore::new());
    }

    #[test]
    fn memory_store_counts() {
        let mut store = MemoryStore::new();
        assert!(store.is_empty());
        store.insert(contract::entry(100, 0)).unwrap();
        assert_eq!(store.len(), 1);
        assert_eq!(store.readable_len(), 0);
    }
}
