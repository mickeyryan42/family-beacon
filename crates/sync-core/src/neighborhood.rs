//! Neighborhood membership: invite chains, stewards, removals, keys.
//!
//! The roster is never edited in place. It is recomputed from every
//! membership event this device has seen, so two phones holding the same
//! events always agree on who is in, regardless of arrival order. See
//! docs/neighborhood-network.md §7–8.
//!
//! Rules:
//! * The founder's certificate is self-signed with the key pinned in the
//!   [`Neighborhood`] record.
//! * Anyone else is a member if their certificate is signed by an existing
//!   member, they haven't been removed, and their inviter hadn't already used
//!   up [`INVITES_PER_DAY`] in the 24 hours before.
//! * The founder can appoint stewards. Stewards can remove anyone except the
//!   founder, optionally together with everyone that person invited.

use std::collections::{BTreeMap, HashMap, HashSet};

use beacon_crypto::{verify, SymmetricKey};
use beacon_protocol::{
    Envelope, EnvelopeId, MembershipCert, Neighborhood, Payload, PersonId, RemovalReason,
    UnixMillis, DAY_MS,
};

/// How many neighborhoods one person can belong to.
pub const MAX_NEIGHBORHOODS: usize = 3;
/// Invites one member can hand out per rolling 24 hours.
pub const INVITES_PER_DAY: usize = 10;
/// Neighborhood posts one author can make per day.
pub const POSTS_PER_DAY: usize = 20;
/// Direct envelopes one author can send per day.
pub const DIRECT_PER_DAY: usize = 30;
/// Past this size the app suggests splitting the neighborhood.
pub const SOFT_MEMBER_CAP: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Removal {
    by: PersonId,
    member: PersonId,
    reason: RemovalReason,
    cascade: bool,
    at: UnixMillis,
}

/// Everything this device knows about one neighborhood it has joined.
#[derive(Debug)]
pub struct NeighborhoodState {
    pub info: Neighborhood,
    keys: BTreeMap<u32, SymmetricKey>,
    /// Membership event envelopes, kept so they can be handed to new members.
    events: Vec<Envelope>,
    certs: Vec<MembershipCert>,
    steward_grants: Vec<(PersonId, PersonId)>,
    /// Invites signed on this device: (inviter, member, when).
    issued: Vec<(PersonId, PersonId, UnixMillis)>,
    removals: Vec<Removal>,
    // Derived by `rebuild`.
    members: HashMap<PersonId, MembershipCert>,
    stewards: HashSet<PersonId>,
    removed: HashSet<PersonId>,
}

impl NeighborhoodState {
    pub fn new(info: Neighborhood, epoch: u32, key: SymmetricKey) -> Self {
        let mut keys = BTreeMap::new();
        keys.insert(epoch, key);
        Self {
            info,
            keys,
            events: Vec::new(),
            certs: Vec::new(),
            steward_grants: Vec::new(),
            issued: Vec::new(),
            removals: Vec::new(),
            members: HashMap::new(),
            stewards: HashSet::new(),
            removed: HashSet::new(),
        }
    }

    // --- keys ---

    pub fn add_key(&mut self, epoch: u32, key: SymmetricKey) {
        self.keys.entry(epoch).or_insert(key);
    }

    /// The key new posts are sealed with.
    pub fn current_key(&self) -> Option<(u32, &SymmetricKey)> {
        self.keys.iter().next_back().map(|(e, k)| (*e, k))
    }

    /// All keys this device holds, newest first, for recognizing posts.
    pub fn keys(&self) -> impl Iterator<Item = (u32, &SymmetricKey)> {
        self.keys.iter().rev().map(|(e, k)| (*e, k))
    }

    // --- events ---

    /// Record a membership event (already signature-checked by the caller).
    /// Returns whether it was new.
    pub fn apply_event(&mut self, envelope: &Envelope) -> bool {
        if self.events.iter().any(|e| e.id == envelope.id) {
            return false;
        }
        match &envelope.payload {
            Payload::MemberJoined(cert) => self.certs.push(cert.clone()),
            Payload::StewardAdded { member } => {
                self.steward_grants.push((envelope.author, *member))
            }
            Payload::MemberRemoved {
                member,
                reason,
                cascade,
            } => self.removals.push(Removal {
                by: envelope.author,
                member: *member,
                reason: *reason,
                cascade: *cascade,
                at: envelope.created_at,
            }),
            _ => return false,
        }
        self.events.push(envelope.clone());
        self.rebuild();
        true
    }

    pub fn events(&self) -> &[Envelope] {
        &self.events
    }

    pub fn has_event(&self, id: &EnvelopeId) -> bool {
        self.events.iter().any(|e| &e.id == id)
    }

    fn founder_cert_is_valid(&self, cert: &MembershipCert) -> bool {
        cert.member == self.info.founder
            && cert.invited_by.is_none()
            && cert.identity_key == self.info.founder_identity_key
            && verify(&cert.identity_key, &cert.signing_bytes(), &cert.signature).is_ok()
    }

    /// Certificates that exceed their inviter's daily allowance. Computed over
    /// all certificates, accepted or not, so the answer doesn't depend on the
    /// order the roster was built in.
    fn over_invite_limit(&self) -> HashSet<(PersonId, PersonId)> {
        let mut by_inviter: HashMap<PersonId, Vec<&MembershipCert>> = HashMap::new();
        for cert in &self.certs {
            if let Some(inviter) = cert.invited_by {
                by_inviter.entry(inviter).or_default().push(cert);
            }
        }
        let mut over = HashSet::new();
        for (inviter, mut certs) in by_inviter {
            certs.sort_by_key(|c| (c.invited_at, c.member));
            for (i, cert) in certs.iter().enumerate() {
                let earlier_in_window = certs[..i]
                    .iter()
                    .filter(|c| cert.invited_at.saturating_sub(c.invited_at) < DAY_MS)
                    .count();
                if earlier_in_window >= INVITES_PER_DAY {
                    over.insert((inviter, cert.member));
                }
            }
        }
        over
    }

    fn rebuild(&mut self) {
        let founder = self.info.founder;

        self.stewards = std::iter::once(founder)
            .chain(
                self.steward_grants
                    .iter()
                    .filter(|(by, _)| *by == founder)
                    .map(|(_, m)| *m),
            )
            .collect();

        // Removals by a steward, never of the founder, plus cascades down the
        // invite tree. Remember when each person was removed: invites they
        // sign afterwards don't count.
        let mut removed_at: HashMap<PersonId, UnixMillis> = HashMap::new();
        let mark = |p: PersonId, at: UnixMillis, map: &mut HashMap<PersonId, UnixMillis>| {
            let entry = map.entry(p).or_insert(at);
            *entry = (*entry).min(at);
        };
        for r in &self.removals {
            if !self.stewards.contains(&r.by) || r.member == founder {
                continue;
            }
            mark(r.member, r.at, &mut removed_at);
            if r.cascade {
                let mut frontier = vec![r.member];
                let mut seen = vec![r.member];
                while let Some(p) = frontier.pop() {
                    for cert in self.certs.iter().filter(|c| c.invited_by == Some(p)) {
                        if cert.member != founder && !seen.contains(&cert.member) {
                            seen.push(cert.member);
                            mark(cert.member, r.at, &mut removed_at);
                            frontier.push(cert.member);
                        }
                    }
                }
            }
        }

        // Valid invite chains: who could legitimately prove membership at the
        // time they were invited, regardless of later removals.
        let over_limit = self.over_invite_limit();
        let mut valid: HashMap<PersonId, MembershipCert> = HashMap::new();
        if let Some(cert) = self.certs.iter().find(|c| self.founder_cert_is_valid(c)) {
            valid.insert(founder, cert.clone());
        }
        let mut candidates: Vec<&MembershipCert> = self.certs.iter().collect();
        candidates.sort_by_key(|c| (c.invited_at, c.member));
        loop {
            let mut grew = false;
            for cert in &candidates {
                let Some(inviter) = cert.invited_by else {
                    continue;
                };
                if valid.contains_key(&cert.member) || over_limit.contains(&(inviter, cert.member))
                {
                    continue;
                }
                if removed_at
                    .get(&inviter)
                    .is_some_and(|at| *at <= cert.invited_at)
                {
                    continue;
                }
                let Some(inviter_cert) = valid.get(&inviter) else {
                    continue;
                };
                if verify(
                    &inviter_cert.identity_key,
                    &cert.signing_bytes(),
                    &cert.signature,
                )
                .is_ok()
                {
                    valid.insert(cert.member, (*cert).clone());
                    grew = true;
                }
            }
            if !grew {
                break;
            }
        }

        valid.retain(|p, _| !removed_at.contains_key(p));
        self.stewards.retain(|s| valid.contains_key(s));
        self.members = valid;
        self.removed = removed_at.into_keys().collect();
    }

    // --- queries ---

    pub fn is_member(&self, person: &PersonId) -> bool {
        self.members.contains_key(person)
    }

    pub fn is_steward(&self, person: &PersonId) -> bool {
        self.stewards.contains(person)
    }

    pub fn is_founder(&self, person: &PersonId) -> bool {
        self.info.founder == *person
    }

    pub fn is_removed(&self, person: &PersonId) -> bool {
        self.removed.contains(person)
    }

    /// Why someone was removed, if they were removed directly (not by cascade).
    pub fn removal_reason(&self, person: &PersonId) -> Option<RemovalReason> {
        self.removals
            .iter()
            .find(|r| r.member == *person && self.removed.contains(person))
            .map(|r| r.reason)
    }

    pub fn member(&self, person: &PersonId) -> Option<&MembershipCert> {
        self.members.get(person)
    }

    /// Members sorted by name.
    pub fn members(&self) -> Vec<&MembershipCert> {
        let mut all: Vec<_> = self.members.values().collect();
        all.sort_by(|a, b| {
            a.display_name
                .cmp(&b.display_name)
                .then(a.member.cmp(&b.member))
        });
        all
    }

    pub fn member_count(&self) -> usize {
        self.members.len()
    }

    pub fn over_soft_cap(&self) -> bool {
        self.members.len() > SOFT_MEMBER_CAP
    }

    /// Who invited whom, from this person back to the founder.
    pub fn invite_chain(&self, person: &PersonId) -> Vec<PersonId> {
        let mut chain = vec![*person];
        let mut current = *person;
        while let Some(inviter) = self.members.get(&current).and_then(|c| c.invited_by) {
            if chain.contains(&inviter) {
                break;
            }
            chain.push(inviter);
            current = inviter;
        }
        chain
    }

    /// Invites `inviter` has handed out in the 24 hours before `now`.
    pub fn invites_in_last_day(&self, inviter: &PersonId, now: UnixMillis) -> usize {
        let recent = |at: UnixMillis| now.saturating_sub(at) < DAY_MS;
        let mut members: HashSet<PersonId> = self
            .certs
            .iter()
            .filter(|c| c.invited_by == Some(*inviter) && recent(c.invited_at))
            .map(|c| c.member)
            .collect();
        // Invites this device signed count immediately, before the newcomer's
        // join has come back over the mesh.
        members.extend(
            self.issued
                .iter()
                .filter(|(by, _, at)| by == inviter && recent(*at))
                .map(|(_, member, _)| *member),
        );
        members.len()
    }

    /// Remember an invite this device just signed.
    pub fn record_issued_invite(&mut self, inviter: PersonId, member: PersonId, at: UnixMillis) {
        self.issued.push((inviter, member, at));
    }

    /// Everyone `person` invited, directly or indirectly, who is still a member.
    pub fn invited_subtree(&self, person: &PersonId) -> Vec<PersonId> {
        let mut out = Vec::new();
        let mut frontier = vec![*person];
        while let Some(p) = frontier.pop() {
            for cert in self.members.values().filter(|c| c.invited_by == Some(p)) {
                if !out.contains(&cert.member) {
                    out.push(cert.member);
                    frontier.push(cert.member);
                }
            }
        }
        out
    }
}

/// Counts actions per person per calendar day (UTC, by the envelope's own
/// timestamp) and refuses past a limit.
#[derive(Debug, Default)]
pub struct RateLimiter {
    counts: HashMap<(PersonId, u64), usize>,
}

impl RateLimiter {
    pub fn allow(&mut self, person: PersonId, at: UnixMillis, limit: usize) -> bool {
        let count = self.counts.entry((person, at / DAY_MS)).or_default();
        if *count >= limit {
            return false;
        }
        *count += 1;
        true
    }

    pub fn used(&self, person: PersonId, at: UnixMillis) -> usize {
        self.counts
            .get(&(person, at / DAY_MS))
            .copied()
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use beacon_crypto::IdentityKeypair;
    use beacon_protocol::{Audience, DeviceId, NeighborhoodId, NewEnvelope};

    const T0: UnixMillis = 1_700_000_000_000;

    struct Who {
        id: PersonId,
        key: IdentityKeypair,
    }

    fn who() -> Who {
        Who {
            id: PersonId::new(),
            key: IdentityKeypair::generate(),
        }
    }

    fn cert(
        n: NeighborhoodId,
        member: &Who,
        inviter: Option<&Who>,
        at: UnixMillis,
    ) -> MembershipCert {
        let mut cert = MembershipCert {
            neighborhood_id: n,
            member: member.id,
            display_name: format!("member {}", &member.id.to_string()[..4]),
            identity_key: member.key.public_bytes().to_vec(),
            encryption_key: vec![9; 32],
            invited_by: inviter.map(|i| i.id),
            invited_at: at,
            signature: vec![],
        };
        let signer = inviter.unwrap_or(member);
        cert.signature = signer.key.sign(&cert.signing_bytes());
        cert
    }

    fn event(n: NeighborhoodId, author: PersonId, payload: Payload) -> Envelope {
        Envelope::new(NewEnvelope {
            audience: Audience::Neighborhood { neighborhood_id: n },
            author,
            origin_device: DeviceId::new(),
            lamport: 1,
            // After the invites in these tests, as removals would be in practice.
            now: T0 + 1_000,
            payload,
        })
    }

    fn setup() -> (NeighborhoodState, Who) {
        let founder = who();
        let info = Neighborhood {
            id: NeighborhoodId::new(),
            name: "Elm & Oak".into(),
            area: "Elm and Oak streets".into(),
            founder: founder.id,
            founder_identity_key: founder.key.public_bytes().to_vec(),
            created_at: T0,
        };
        let mut state = NeighborhoodState::new(info, 0, SymmetricKey::generate());
        let n = state.info.id;
        state.apply_event(&event(
            n,
            founder.id,
            Payload::MemberJoined(cert(n, &founder, None, T0)),
        ));
        (state, founder)
    }

    fn join(state: &mut NeighborhoodState, member: &Who, inviter: &Who, at: UnixMillis) {
        let n = state.info.id;
        state.apply_event(&event(
            n,
            member.id,
            Payload::MemberJoined(cert(n, member, Some(inviter), at)),
        ));
    }

    #[test]
    fn invite_chains_grow_the_neighborhood() {
        let (mut s, founder) = setup();
        let (b, c) = (who(), who());
        join(&mut s, &b, &founder, T0 + 1);
        join(&mut s, &c, &b, T0 + 2);
        assert!(s.is_member(&founder.id) && s.is_member(&b.id) && s.is_member(&c.id));
        assert_eq!(s.invite_chain(&c.id), vec![c.id, b.id, founder.id]);
    }

    #[test]
    fn roster_is_the_same_whatever_order_events_arrive() {
        let (mut s, founder) = setup();
        let (b, c) = (who(), who());
        // C's certificate arrives before B's: C is pending until B is known.
        join(&mut s, &c, &b, T0 + 2);
        assert!(!s.is_member(&c.id));
        join(&mut s, &b, &founder, T0 + 1);
        assert!(s.is_member(&c.id));
    }

    #[test]
    fn forged_certificates_are_ignored() {
        let (mut s, founder) = setup();
        let (b, mallory) = (who(), who());
        let n = s.info.id;
        // Mallory signs B's certificate but claims the founder invited them.
        let mut forged = cert(n, &b, Some(&mallory), T0);
        forged.invited_by = Some(founder.id);
        s.apply_event(&event(n, b.id, Payload::MemberJoined(forged)));
        assert!(!s.is_member(&b.id));

        // And nobody but the founder can be the founder.
        let fake_founder = cert(n, &mallory, None, T0);
        s.apply_event(&event(n, mallory.id, Payload::MemberJoined(fake_founder)));
        assert!(!s.is_member(&mallory.id));
    }

    #[test]
    fn stewards_remove_with_optional_cascade() {
        let (mut s, founder) = setup();
        let (steward, b, c, d) = (who(), who(), who(), who());
        join(&mut s, &steward, &founder, T0 + 1);
        join(&mut s, &b, &founder, T0 + 2);
        join(&mut s, &c, &b, T0 + 3);
        join(&mut s, &d, &c, T0 + 4);
        let n = s.info.id;

        // Ordinary members can't remove anyone.
        s.apply_event(&event(
            n,
            c.id,
            Payload::MemberRemoved {
                member: steward.id,
                reason: RemovalReason::Abuse,
                cascade: false,
            },
        ));
        assert!(s.is_member(&steward.id));

        s.apply_event(&event(
            n,
            founder.id,
            Payload::StewardAdded { member: steward.id },
        ));
        assert!(s.is_steward(&steward.id));

        // Without cascade, only C goes. D was legitimately invited while C was
        // a member, so D stays.
        s.apply_event(&event(
            n,
            steward.id,
            Payload::MemberRemoved {
                member: c.id,
                reason: RemovalReason::Spam,
                cascade: false,
            },
        ));
        assert!(!s.is_member(&c.id) && s.is_removed(&c.id));
        assert!(s.is_member(&b.id) && s.is_member(&d.id));
        assert_eq!(s.removal_reason(&c.id), Some(RemovalReason::Spam));

        // A removed member can no longer invite anyone.
        let e = who();
        join(&mut s, &e, &c, T0 + DAY_MS);
        assert!(!s.is_member(&e.id));

        // With cascade, B and everyone below are removed.
        s.apply_event(&event(
            n,
            steward.id,
            Payload::MemberRemoved {
                member: b.id,
                reason: RemovalReason::Spam,
                cascade: true,
            },
        ));
        assert!(s.is_removed(&b.id) && s.is_removed(&d.id));
        assert_eq!(s.member_count(), 2);

        // The founder can never be removed.
        s.apply_event(&event(
            n,
            steward.id,
            Payload::MemberRemoved {
                member: founder.id,
                reason: RemovalReason::Other,
                cascade: true,
            },
        ));
        assert!(s.is_member(&founder.id) && s.is_member(&steward.id));
    }

    #[test]
    fn only_the_founder_appoints_stewards() {
        let (mut s, founder) = setup();
        let b = who();
        join(&mut s, &b, &founder, T0 + 1);
        let n = s.info.id;
        s.apply_event(&event(n, b.id, Payload::StewardAdded { member: b.id }));
        assert!(!s.is_steward(&b.id));
    }

    #[test]
    fn invites_are_limited_per_day() {
        let (mut s, founder) = setup();
        let invitees: Vec<Who> = (0..12).map(|_| who()).collect();
        for (i, p) in invitees.iter().enumerate() {
            join(&mut s, p, &founder, T0 + 10 + i as u64);
        }
        assert_eq!(s.member_count(), 1 + INVITES_PER_DAY);
        assert!(!s.is_member(&invitees[11].id));

        // A day later the allowance resets.
        let late = who();
        join(&mut s, &late, &founder, T0 + DAY_MS + 100);
        assert!(s.is_member(&late.id));
    }

    #[test]
    fn rate_limiter_counts_per_person_per_day() {
        let mut r = RateLimiter::default();
        let p = PersonId::new();
        assert!(r.allow(p, T0, 2));
        assert!(r.allow(p, T0 + 1, 2));
        assert!(!r.allow(p, T0 + 2, 2));
        assert!(r.allow(PersonId::new(), T0, 2));
        assert!(r.allow(p, T0 + DAY_MS, 2));
    }

    #[test]
    fn keys_are_tracked_by_epoch() {
        let (mut s, _) = setup();
        let newer = SymmetricKey::generate();
        s.add_key(1, newer.clone());
        assert_eq!(
            s.current_key().map(|(e, k)| (e, k.clone())),
            Some((1, newer))
        );
        assert_eq!(s.keys().count(), 2);
    }
}
