//! Outbound store-and-forward queue, with a budget per tier.
//!
//! Holds the sealed envelopes this device is willing to pass on: its own, its
//! circles', and (when relaying is on) other people's opaque traffic. Each
//! tier has its own entry, byte and per-session limits (docs §10.3), so a busy
//! neighborhood can never crowd out your own family's updates.
//!
//! Send order when two phones meet (docs §10.1):
//! 1. anything marked urgent, in any tier
//! 2. own household, 3. direct, 4. neighborhood, 5. relay for others
//!
//! When a tier is full, the lowest-priority, oldest envelope in that tier is
//! evicted first; urgent envelopes are the last to go.

use std::collections::HashMap;

use beacon_protocol::{EnvelopeId, IdSummary, PriorityClass, Scope, SealedEnvelope, UnixMillis};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Tier {
    Household = 0,
    Direct = 1,
    Neighborhood = 2,
    /// Sealed traffic this device can't read and is only carrying.
    Relay = 3,
}

impl Tier {
    pub fn for_scope(scope: Scope) -> Self {
        match scope {
            Scope::Household => Tier::Household,
            Scope::Neighborhood => Tier::Neighborhood,
            Scope::Direct => Tier::Direct,
        }
    }

    pub const ALL: [Tier; 4] = [
        Tier::Household,
        Tier::Direct,
        Tier::Neighborhood,
        Tier::Relay,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TierBudget {
    pub entries: usize,
    pub bytes: usize,
    pub per_session: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budgets {
    pub household: TierBudget,
    pub neighborhood: TierBudget,
    pub direct: TierBudget,
    pub relay: TierBudget,
}

impl Default for Budgets {
    fn default() -> Self {
        const MB: usize = 1024 * 1024;
        Self {
            household: TierBudget {
                entries: 500,
                bytes: 2 * MB,
                per_session: 200,
            },
            neighborhood: TierBudget {
                entries: 2_000,
                bytes: 4 * MB,
                per_session: 100,
            },
            direct: TierBudget {
                entries: 200,
                bytes: MB,
                per_session: 50,
            },
            relay: TierBudget {
                entries: 1_000,
                bytes: 2 * MB,
                per_session: 100,
            },
        }
    }
}

impl Budgets {
    pub fn for_tier(&self, tier: Tier) -> TierBudget {
        match tier {
            Tier::Household => self.household,
            Tier::Neighborhood => self.neighborhood,
            Tier::Direct => self.direct,
            Tier::Relay => self.relay,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushOutcome {
    Queued,
    Duplicate,
    /// Queued after evicting lower-value envelopes from the same tier.
    QueuedWithEviction(Vec<EnvelopeId>),
    /// The tier is full of more important envelopes; this one was not queued.
    Rejected,
    /// Expired or has no hops left, so there is nothing to forward.
    NotForwardable,
}

#[derive(Debug, Clone)]
struct Item {
    sealed: SealedEnvelope,
    tier: Tier,
    first_seen: UnixMillis,
    bytes: usize,
}

/// Lower sorts first = evicted first.
fn eviction_key(i: &Item) -> (PriorityClass, UnixMillis, EnvelopeId) {
    (i.sealed.priority_class, i.first_seen, i.sealed.id)
}

#[derive(Debug, Default)]
pub struct OutboundQueue {
    budgets: Budgets,
    items: HashMap<EnvelopeId, Item>,
}

impl OutboundQueue {
    pub fn new(budgets: Budgets) -> Self {
        Self {
            budgets,
            items: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn tier_len(&self, tier: Tier) -> usize {
        self.items.values().filter(|i| i.tier == tier).count()
    }

    pub fn contains(&self, id: &EnvelopeId) -> bool {
        self.items.contains_key(id)
    }

    pub fn remove(&mut self, id: &EnvelopeId) {
        self.items.remove(id);
    }

    fn tier_usage(&self, tier: Tier) -> (usize, usize) {
        self.items
            .values()
            .filter(|i| i.tier == tier)
            .fold((0, 0), |(n, b), i| (n + 1, b + i.bytes))
    }

    pub fn push(&mut self, sealed: SealedEnvelope, tier: Tier, now: UnixMillis) -> PushOutcome {
        if sealed.is_expired(now) {
            return PushOutcome::NotForwardable;
        }
        if self.items.contains_key(&sealed.id) {
            return PushOutcome::Duplicate;
        }
        let budget = self.budgets.for_tier(tier);
        let item = Item {
            bytes: sealed.approx_bytes(),
            sealed,
            tier,
            first_seen: now,
        };

        // Plan evictions before touching anything: drop the least valuable
        // items in this tier until the newcomer fits, but only if every one of
        // them is worth less than the newcomer.
        let (mut count, mut bytes) = self.tier_usage(tier);
        let mut by_value: Vec<&Item> = self.items.values().filter(|i| i.tier == tier).collect();
        by_value.sort_by_key(|i| eviction_key(i));
        let mut evicted = Vec::new();
        for victim in by_value {
            if count < budget.entries && bytes + item.bytes <= budget.bytes {
                break;
            }
            if eviction_key(victim) >= eviction_key(&item) {
                return PushOutcome::Rejected;
            }
            count -= 1;
            bytes -= victim.bytes;
            evicted.push(victim.sealed.id);
        }
        if count >= budget.entries || bytes + item.bytes > budget.bytes {
            return PushOutcome::Rejected;
        }

        for id in &evicted {
            self.items.remove(id);
        }
        self.items.insert(item.sealed.id, item);
        if evicted.is_empty() {
            PushOutcome::Queued
        } else {
            PushOutcome::QueuedWithEviction(evicted)
        }
    }

    /// Envelopes to send to a peer whose `Hello` summarized what it holds,
    /// in send order and within each tier's per-session cap.
    pub fn batch_for(&self, peer_has: &IdSummary, now: UnixMillis) -> Vec<SealedEnvelope> {
        let mut candidates: Vec<&Item> = self
            .items
            .values()
            .filter(|i| !i.sealed.is_expired(now) && i.sealed.hops_remaining > 0)
            .filter(|i| !peer_has.might_contain(&i.sealed.id))
            .collect();
        candidates.sort_by_key(|i| {
            (
                i.sealed.priority_class != PriorityClass::Urgent,
                i.tier,
                std::cmp::Reverse(i.sealed.priority_class),
                i.first_seen,
                i.sealed.id,
            )
        });

        let mut sent_per_tier: HashMap<Tier, usize> = HashMap::new();
        let mut out = Vec::new();
        for item in candidates {
            let sent = sent_per_tier.entry(item.tier).or_default();
            if *sent < self.budgets.for_tier(item.tier).per_session {
                *sent += 1;
                out.push(item.sealed.clone());
            }
        }
        out
    }

    /// Expired envelopes removed from the queue.
    pub fn prune_expired(&mut self, now: UnixMillis) -> Vec<EnvelopeId> {
        let expired: Vec<_> = self
            .items
            .values()
            .filter(|i| i.sealed.is_expired(now))
            .map(|i| i.sealed.id)
            .collect();
        for id in &expired {
            self.items.remove(id);
        }
        expired
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sealed(class: PriorityClass, body: usize) -> SealedEnvelope {
        SealedEnvelope {
            version: beacon_protocol::PROTOCOL_VERSION,
            id: EnvelopeId::new(),
            scope: Scope::Neighborhood,
            route_tag: vec![0; 16],
            expires_at: 1_000_000,
            hops_remaining: 5,
            priority_class: class,
            body: vec![0; body],
        }
    }

    fn small_budgets() -> Budgets {
        let tight = TierBudget {
            entries: 2,
            bytes: 1_000_000,
            per_session: 10,
        };
        Budgets {
            household: tight,
            neighborhood: tight,
            direct: tight,
            relay: tight,
        }
    }

    #[test]
    fn deduplicates_by_envelope_id() {
        let mut q = OutboundQueue::default();
        let e = sealed(PriorityClass::Normal, 10);
        assert_eq!(q.push(e.clone(), Tier::Household, 0), PushOutcome::Queued);
        assert_eq!(q.push(e, Tier::Household, 0), PushOutcome::Duplicate);
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn sends_urgent_first_then_by_tier() {
        let mut q = OutboundQueue::default();
        let relay = sealed(PriorityClass::Normal, 10);
        let hood = sealed(PriorityClass::Normal, 10);
        let home = sealed(PriorityClass::Low, 10);
        let urgent_relay = sealed(PriorityClass::Urgent, 10);
        q.push(relay.clone(), Tier::Relay, 1);
        q.push(hood.clone(), Tier::Neighborhood, 2);
        q.push(home.clone(), Tier::Household, 3);
        q.push(urgent_relay.clone(), Tier::Relay, 4);

        let order: Vec<_> = q
            .batch_for(&IdSummary::empty(), 5)
            .into_iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(order, vec![urgent_relay.id, home.id, hood.id, relay.id]);
    }

    #[test]
    fn a_full_tier_never_evicts_from_another_tier() {
        let mut q = OutboundQueue::new(small_budgets());
        let home = sealed(PriorityClass::Low, 10);
        q.push(home.clone(), Tier::Household, 0);
        for i in 0..5 {
            q.push(sealed(PriorityClass::Normal, 10), Tier::Relay, i);
        }
        assert!(q.contains(&home.id));
        assert_eq!(q.tier_len(Tier::Relay), 2);
    }

    #[test]
    fn full_tier_evicts_lowest_value_and_protects_urgent() {
        let mut q = OutboundQueue::new(small_budgets());
        let urgent = sealed(PriorityClass::Urgent, 10);
        let low = sealed(PriorityClass::Low, 10);
        q.push(urgent.clone(), Tier::Neighborhood, 1);
        q.push(low.clone(), Tier::Neighborhood, 2);

        let normal = sealed(PriorityClass::Normal, 10);
        assert_eq!(
            q.push(normal.clone(), Tier::Neighborhood, 3),
            PushOutcome::QueuedWithEviction(vec![low.id])
        );
        assert_eq!(
            q.push(sealed(PriorityClass::Low, 10), Tier::Neighborhood, 4),
            PushOutcome::Rejected
        );
        assert!(q.contains(&urgent.id) && q.contains(&normal.id));
    }

    #[test]
    fn byte_budgets_are_enforced() {
        let tight = TierBudget {
            entries: 100,
            bytes: 2_000,
            per_session: 10,
        };
        let mut q = OutboundQueue::new(Budgets {
            relay: tight,
            ..Budgets::default()
        });
        q.push(sealed(PriorityClass::Normal, 1_000), Tier::Relay, 1);
        let big = sealed(PriorityClass::Normal, 1_000);
        assert!(matches!(
            q.push(big, Tier::Relay, 2),
            PushOutcome::QueuedWithEviction(_)
        ));
        assert_eq!(q.tier_len(Tier::Relay), 1);
    }

    #[test]
    fn respects_per_session_caps_and_peer_summary() {
        let tight = TierBudget {
            entries: 100,
            bytes: 1_000_000,
            per_session: 2,
        };
        let mut q = OutboundQueue::new(Budgets {
            neighborhood: tight,
            ..Budgets::default()
        });
        let items: Vec<_> = (0..4).map(|_| sealed(PriorityClass::Normal, 10)).collect();
        for (i, e) in items.iter().enumerate() {
            q.push(e.clone(), Tier::Neighborhood, i as u64);
        }
        assert_eq!(q.batch_for(&IdSummary::empty(), 10).len(), 2);

        let peer = IdSummary::build(items.iter().map(|e| &e.id), items.len(), 1);
        assert!(q.batch_for(&peer, 10).is_empty());
    }

    #[test]
    fn refuses_expired_and_prunes() {
        let mut q = OutboundQueue::default();
        let e = sealed(PriorityClass::Normal, 10);
        assert_eq!(
            q.push(e.clone(), Tier::Relay, e.expires_at),
            PushOutcome::NotForwardable
        );
        q.push(e.clone(), Tier::Relay, 0);
        assert_eq!(q.prune_expired(e.expires_at), vec![e.id]);
        assert!(q.is_empty());
    }
}
