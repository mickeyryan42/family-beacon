//! Conflict handling: folding envelopes into the state the UI shows.
//!
//! Devices receive envelopes in arbitrary order, sometimes days late, sometimes
//! twice. The rules below guarantee every phone that holds the same set of
//! envelopes shows the same state, regardless of arrival order:
//!
//! * **Check-ins: latest event wins, per person.** A check-in describes the
//!   real world ("I'm safe" at 3:05pm), so ordering is by the reported time,
//!   then Lamport time, then envelope ID as a final deterministic tiebreak.
//!   This also handles one person with two phones, or a parent checking in for
//!   a child, where Lamport counters are unrelated.
//! * **Rendezvous points: last writer wins, per point, with tombstones.** Plans
//!   are edited, so ordering is causal (Lamport) first. A device that has seen
//!   an edit always produces a later Lamport time, so "I saw your change and
//!   updated it" is never overwritten by the stale version. Removal leaves a
//!   tombstone so an older copy arriving late cannot resurrect the point.
//! * **Notes are append-only.** Nothing to resolve; keep the most recent few.

use std::collections::HashMap;

use beacon_protocol::{
    CheckIn, Envelope, EnvelopeId, Payload, PersonId, RendezvousId, RendezvousPoint, UnixMillis,
};

/// Notes are a convenience, not a log. Cap them so state stays small.
pub const MAX_NOTES: usize = 50;

/// Total order used to decide which of two competing writes wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Stamp {
    pub primary: u64,
    pub secondary: u64,
    pub tiebreak: EnvelopeId,
}

impl Stamp {
    fn for_check_in(env: &Envelope, check_in: &CheckIn) -> Self {
        Self {
            primary: check_in.recorded_at,
            secondary: env.lamport,
            tiebreak: env.id,
        }
    }

    fn for_edit(env: &Envelope) -> Self {
        Self {
            primary: env.lamport,
            secondary: env.created_at,
            tiebreak: env.id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RendezvousSlot {
    Live(RendezvousPoint),
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub envelope_id: EnvelopeId,
    pub author: PersonId,
    pub text: String,
    pub created_at: UnixMillis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyOutcome {
    Applied,
    /// A newer write for the same record is already known.
    Superseded,
    /// Not a household payload; handled elsewhere.
    NotApplicable,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct HouseholdState {
    check_ins: HashMap<PersonId, (Stamp, CheckIn)>,
    rendezvous: HashMap<RendezvousId, (Stamp, RendezvousSlot)>,
    notes: Vec<Note>,
}

impl HouseholdState {
    pub fn apply(&mut self, env: &Envelope) -> ApplyOutcome {
        match &env.payload {
            Payload::CheckIn(check_in) => {
                let stamp = Stamp::for_check_in(env, check_in);
                upsert(
                    &mut self.check_ins,
                    check_in.person_id,
                    stamp,
                    check_in.clone(),
                )
            }
            Payload::RendezvousUpsert(point) => {
                let stamp = Stamp::for_edit(env);
                upsert(
                    &mut self.rendezvous,
                    point.id,
                    stamp,
                    RendezvousSlot::Live(point.clone()),
                )
            }
            Payload::RendezvousRemove { id } => upsert(
                &mut self.rendezvous,
                *id,
                Stamp::for_edit(env),
                RendezvousSlot::Removed,
            ),
            Payload::Note { text } => {
                if self.notes.iter().any(|n| n.envelope_id == env.id) {
                    return ApplyOutcome::Superseded;
                }
                self.notes.push(Note {
                    envelope_id: env.id,
                    author: env.author,
                    text: text.clone(),
                    created_at: env.created_at,
                });
                // Newest first, deterministic on ties.
                self.notes.sort_by(|a, b| {
                    (b.created_at, b.envelope_id).cmp(&(a.created_at, a.envelope_id))
                });
                self.notes.truncate(MAX_NOTES);
                ApplyOutcome::Applied
            }
            _ => ApplyOutcome::NotApplicable,
        }
    }

    pub fn check_in(&self, person: &PersonId) -> Option<&CheckIn> {
        self.check_ins.get(person).map(|(_, c)| c)
    }

    /// Latest check-in per person, most recent first.
    pub fn check_ins(&self) -> Vec<&CheckIn> {
        let mut all: Vec<_> = self.check_ins.values().map(|(_, c)| c).collect();
        all.sort_by(|a, b| {
            b.recorded_at
                .cmp(&a.recorded_at)
                .then(a.person_id.cmp(&b.person_id))
        });
        all
    }

    pub fn rendezvous(&self, id: &RendezvousId) -> Option<&RendezvousPoint> {
        match self.rendezvous.get(id) {
            Some((_, RendezvousSlot::Live(point))) => Some(point),
            _ => None,
        }
    }

    /// Live meeting points, primary first.
    pub fn rendezvous_points(&self) -> Vec<&RendezvousPoint> {
        let mut live: Vec<_> = self
            .rendezvous
            .values()
            .filter_map(|(_, slot)| match slot {
                RendezvousSlot::Live(p) => Some(p),
                RendezvousSlot::Removed => None,
            })
            .collect();
        live.sort_by(|a, b| a.rank.cmp(&b.rank).then_with(|| a.name.cmp(&b.name)));
        live
    }

    pub fn notes(&self) -> &[Note] {
        &self.notes
    }
}

fn upsert<K: std::hash::Hash + Eq, V>(
    map: &mut HashMap<K, (Stamp, V)>,
    key: K,
    stamp: Stamp,
    value: V,
) -> ApplyOutcome {
    match map.get(&key) {
        Some((existing, _)) if *existing >= stamp => ApplyOutcome::Superseded,
        _ => {
            map.insert(key, (stamp, value));
            ApplyOutcome::Applied
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use beacon_protocol::{Audience, CheckInState, DeviceId, HouseholdId, NewEnvelope};

    struct Ctx {
        household: HouseholdId,
        author: PersonId,
    }

    impl Ctx {
        fn new() -> Self {
            Self {
                household: HouseholdId::new(),
                author: PersonId::new(),
            }
        }

        fn env(&self, lamport: u64, now: UnixMillis, payload: Payload) -> Envelope {
            Envelope::new(NewEnvelope {
                audience: Audience::Household {
                    household_id: self.household,
                },
                author: self.author,
                origin_device: DeviceId::new(),
                lamport,
                now,
                payload,
            })
        }

        fn check_in(&self, lamport: u64, at: UnixMillis, state: CheckInState) -> Envelope {
            self.env(
                lamport,
                at,
                Payload::CheckIn(CheckIn {
                    person_id: self.author,
                    state,
                    note: None,
                    rendezvous_id: None,
                    recorded_at: at,
                }),
            )
        }

        fn point(&self, id: RendezvousId, name: &str, lamport: u64) -> Envelope {
            self.env(
                lamport,
                1_000,
                Payload::RendezvousUpsert(RendezvousPoint {
                    id,
                    household_id: self.household,
                    name: name.into(),
                    details: String::new(),
                    location: None,
                    when_to_go: String::new(),
                    rank: 1,
                }),
            )
        }
    }

    fn state_from(envelopes: &[&Envelope]) -> HouseholdState {
        let mut state = HouseholdState::default();
        for e in envelopes {
            state.apply(e);
        }
        state
    }

    #[test]
    fn newest_check_in_wins_regardless_of_arrival_order() {
        let ctx = Ctx::new();
        let earlier = ctx.check_in(5, 1_000, CheckInState::OnMyWay);
        // A second phone with a lower Lamport counter but a later report.
        let later = ctx.check_in(1, 2_000, CheckInState::Safe);

        for order in [[&earlier, &later], [&later, &earlier]] {
            let state = state_from(&order);
            assert_eq!(
                state.check_in(&ctx.author).unwrap().state,
                CheckInState::Safe
            );
        }
    }

    #[test]
    fn ties_break_deterministically_on_every_device() {
        let ctx = Ctx::new();
        let a = ctx.check_in(1, 1_000, CheckInState::Safe);
        let b = ctx.check_in(1, 1_000, CheckInState::NeedHelp);

        let one = state_from(&[&a, &b]);
        let two = state_from(&[&b, &a]);
        assert_eq!(one, two);
    }

    #[test]
    fn causal_edit_beats_stale_edit() {
        let ctx = Ctx::new();
        let id = RendezvousId::new();
        let original = ctx.point(id, "Park gate", 3);
        let edited = ctx.point(id, "Library steps", 4);

        for order in [[&original, &edited], [&edited, &original]] {
            assert_eq!(
                state_from(&order).rendezvous(&id).unwrap().name,
                "Library steps"
            );
        }
    }

    #[test]
    fn removal_tombstone_blocks_late_arriving_older_copy() {
        let ctx = Ctx::new();
        let id = RendezvousId::new();
        let created = ctx.point(id, "Park gate", 1);
        let removed = ctx.env(2, 2_000, Payload::RendezvousRemove { id });

        let state = state_from(&[&removed, &created]);
        assert!(state.rendezvous(&id).is_none());
        assert!(state.rendezvous_points().is_empty());

        // Re-adding after seeing the removal (higher Lamport) brings it back.
        let readded = ctx.point(id, "Park gate (north side)", 3);
        let state = state_from(&[&created, &removed, &readded]);
        assert_eq!(
            state.rendezvous(&id).unwrap().name,
            "Park gate (north side)"
        );
    }

    #[test]
    fn reapplying_is_idempotent() {
        let ctx = Ctx::new();
        let e = ctx.check_in(1, 1_000, CheckInState::Safe);
        let n = ctx.env(
            2,
            1_500,
            Payload::Note {
                text: "Phones charged".into(),
            },
        );

        let mut state = state_from(&[&e, &n]);
        let snapshot = state.clone();
        assert_eq!(state.apply(&e), ApplyOutcome::Superseded);
        assert_eq!(state.apply(&n), ApplyOutcome::Superseded);
        assert_eq!(state, snapshot);
    }

    #[test]
    fn notes_are_newest_first_and_capped() {
        let ctx = Ctx::new();
        let mut state = HouseholdState::default();
        for i in 0..(MAX_NOTES as u64 + 5) {
            state.apply(&ctx.env(
                i,
                i * 10,
                Payload::Note {
                    text: format!("note {i}"),
                },
            ));
        }
        assert_eq!(state.notes().len(), MAX_NOTES);
        assert_eq!(state.notes()[0].text, format!("note {}", MAX_NOTES + 4));
    }
}
