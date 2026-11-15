//! The sync engine: one per device.
//!
//! Owns this device's view of its household, the neighborhoods it has joined,
//! and its direct messages, plus the sealed traffic it carries for others.

use std::collections::{HashMap, HashSet};

use beacon_crypto::SymmetricKey;
use beacon_protocol::{
    decode_message, encode_message, validate_envelope, validate_sealed, Audience, DeviceId,
    Envelope, Hello, Household, IdSummary, Membership, MembershipCert, Neighborhood,
    NeighborhoodId, NewEnvelope, Payload, PersonId, Priority, PriorityClass, RemovalReason, Scope,
    SealedEnvelope, SyncMessage, UnixMillis, ValidationLimits, DAY_MS, PROTOCOL_VERSION,
};
use log::{debug, info, warn};

use crate::clock::LamportClock;
use crate::conflict::HouseholdState;
use crate::error::SyncError;
use crate::feed::{Inbox, NeighborhoodFeed, Post};
use crate::identity::{verify_envelope, IdentityError, LocalIdentity};
use crate::neighborhood::{
    NeighborhoodState, RateLimiter, DIRECT_PER_DAY, INVITES_PER_DAY, MAX_NEIGHBORHOODS,
    POSTS_PER_DAY,
};
use crate::queue::{Budgets, OutboundQueue, PushOutcome, Tier};
use crate::schedule::{ScheduleConfig, SyncSchedule};
use crate::sealing::{
    is_for_group, is_for_person, open_direct, open_group, seal_envelope, SealFor,
};
use crate::session::{pack_deliveries, SyncSession};
use crate::store::{EnvelopeStore, StoredEnvelope};
use crate::transport::{PeerAddress, Transport, TransportError};

/// A neighborhood this device has joined, as persisted between launches.
#[derive(Debug, Clone)]
pub struct JoinedNeighborhood {
    pub info: Neighborhood,
    pub keys: Vec<(u32, SymmetricKey)>,
}

pub struct SyncEngineConfig {
    pub device_id: DeviceId,
    pub identity: LocalIdentity,
    pub household: Household,
    /// Shared at in-person pairing.
    pub household_key: SymmetricKey,
    pub limits: ValidationLimits,
    pub schedule: ScheduleConfig,
    pub budgets: Budgets,
    /// Carry sealed envelopes this device can't read. On by default: the
    /// mesh only reaches far if most phones help.
    pub relay_enabled: bool,
    /// Don't re-initiate with a peer we just synced with unless we have
    /// something new for them.
    pub peer_cooldown_ms: u64,
    /// Neighborhoods joined in an earlier session.
    pub neighborhoods: Vec<JoinedNeighborhood>,
}

impl SyncEngineConfig {
    pub fn new(
        device_id: DeviceId,
        identity: LocalIdentity,
        household: Household,
        household_key: SymmetricKey,
    ) -> Self {
        Self {
            device_id,
            identity,
            household,
            household_key,
            limits: ValidationLimits::default(),
            schedule: ScheduleConfig::default(),
            budgets: Budgets::default(),
            relay_enabled: true,
            peer_cooldown_ms: 5 * 60_000,
            neighborhoods: Vec::new(),
        }
    }
}

/// What someone shows (as a QR code) so a member can invite them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteRequest {
    pub person_id: PersonId,
    pub display_name: String,
    pub identity_key: Vec<u8>,
    pub encryption_key: Vec<u8>,
}

/// Handed to a new member in person: everything needed to join.
#[derive(Debug, Clone)]
pub struct InvitePackage {
    pub neighborhood: Neighborhood,
    pub epoch: u32,
    pub key: SymmetricKey,
    pub cert: MembershipCert,
    /// Signed membership events the inviter knows, so the newcomer can
    /// verify existing members right away instead of waiting for the mesh.
    pub roster: Vec<Envelope>,
}

/// What happened during one [`SyncEngine::sync_round`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RoundReport {
    pub peers_in_range: usize,
    pub sessions_started: usize,
    pub sessions_completed: usize,
    pub envelopes_sent: usize,
    /// New envelopes this device could read.
    pub envelopes_received: usize,
    /// New sealed envelopes this device is only carrying for others.
    pub envelopes_relayed: usize,
    pub envelopes_rejected: usize,
    pub errors: usize,
    /// Set when the round did nothing, e.g. the radio is off.
    pub skipped: Option<String>,
}

impl RoundReport {
    pub fn exchanged_anything(&self) -> bool {
        self.envelopes_sent + self.envelopes_received + self.envelopes_relayed > 0
    }
}

/// Outcome of receiving one sealed envelope.
#[derive(Debug, PartialEq, Eq)]
enum Receipt {
    Readable,
    Relayed,
    Duplicate,
    Dropped,
    Rejected,
}

/// Read-side view of the circles this device belongs to.
struct Circles<'a> {
    me: PersonId,
    household: &'a Household,
    neighborhoods: &'a HashMap<NeighborhoodId, NeighborhoodState>,
}

impl Membership for Circles<'_> {
    fn local_person(&self) -> PersonId {
        self.me
    }
    fn household(&self) -> &Household {
        self.household
    }
    fn in_neighborhood(&self, n: &NeighborhoodId) -> bool {
        self.neighborhoods.contains_key(n)
    }
    fn is_neighborhood_member(&self, n: &NeighborhoodId, p: &PersonId) -> bool {
        self.neighborhoods.get(n).is_some_and(|s| s.is_member(p))
    }
    fn knows(&self, p: &PersonId) -> bool {
        self.household.is_member(p) || self.neighborhoods.values().any(|s| s.is_member(p))
    }
}

/// Validation view used when *sending* a direct message: the recipient plays
/// the reader, and they must know us.
struct Sending<'a> {
    circles: Circles<'a>,
    recipient: PersonId,
}

impl Membership for Sending<'_> {
    fn local_person(&self) -> PersonId {
        self.recipient
    }
    fn household(&self) -> &Household {
        self.circles.household
    }
    fn in_neighborhood(&self, n: &NeighborhoodId) -> bool {
        self.circles.in_neighborhood(n)
    }
    fn is_neighborhood_member(&self, n: &NeighborhoodId, p: &PersonId) -> bool {
        self.circles.is_neighborhood_member(n, p)
    }
    fn knows(&self, p: &PersonId) -> bool {
        *p == self.circles.me || self.circles.knows(p)
    }
}

pub struct SyncEngine<S: EnvelopeStore, T: Transport> {
    config: SyncEngineConfig,
    store: S,
    transport: T,
    queue: OutboundQueue,
    household: HouseholdState,
    neighborhoods: HashMap<NeighborhoodId, NeighborhoodState>,
    feeds: HashMap<NeighborhoodId, NeighborhoodFeed>,
    inbox: Inbox,
    blocked: HashSet<PersonId>,
    post_limits: RateLimiter,
    direct_limits: RateLimiter,
    clock: LamportClock,
    schedule: SyncSchedule,
    sessions: HashMap<PeerAddress, SyncSession>,
    /// Peer -> (when we last finished a session, queue generation at the time).
    recent_peers: HashMap<PeerAddress, (UnixMillis, u64)>,
    /// Bumped whenever something new enters the outbound queue.
    queue_generation: u64,
}

impl<S: EnvelopeStore, T: Transport> SyncEngine<S, T> {
    pub fn new(
        mut config: SyncEngineConfig,
        store: S,
        transport: T,
        now: UnixMillis,
    ) -> Result<Self, SyncError> {
        if !config.household.is_member(&config.identity.person_id) {
            return Err(SyncError::LocalPersonNotMember);
        }
        let mut neighborhoods = HashMap::new();
        for joined in std::mem::take(&mut config.neighborhoods) {
            let mut keys = joined.keys.into_iter();
            let Some((epoch, key)) = keys.next() else {
                continue;
            };
            let mut state = NeighborhoodState::new(joined.info, epoch, key);
            for (e, k) in keys {
                state.add_key(e, k);
            }
            neighborhoods.insert(state.info.id, state);
        }

        let mut engine = Self {
            queue: OutboundQueue::new(config.budgets),
            schedule: SyncSchedule::new(config.schedule, now),
            config,
            store,
            transport,
            household: HouseholdState::default(),
            neighborhoods,
            feeds: HashMap::new(),
            inbox: Inbox::default(),
            blocked: HashSet::new(),
            post_limits: RateLimiter::default(),
            direct_limits: RateLimiter::default(),
            clock: LamportClock::default(),
            sessions: HashMap::new(),
            recent_peers: HashMap::new(),
            queue_generation: 0,
        };
        engine.rebuild_from_store(now)?;
        Ok(engine)
    }

    /// Restore state, clock and outbound queue from what is already stored.
    fn rebuild_from_store(&mut self, now: UnixMillis) -> Result<(), SyncError> {
        let mut entries = self.store.all(now)?;
        // Membership events first, so posts and keys find their authors.
        entries.sort_by_key(|e| {
            !e.opened
                .as_ref()
                .is_some_and(|o| o.payload.is_membership_event())
        });
        for entry in entries {
            if let Some(env) = &entry.opened {
                self.apply_opened(env);
            }
            let own = entry
                .opened
                .as_ref()
                .is_some_and(|o| o.origin_device == self.config.device_id);
            let outgoing = if own {
                Some(entry.sealed.clone())
            } else {
                entry.sealed.for_relay()
            };
            if let Some(sealed) = outgoing {
                self.enqueue(sealed, entry.tier, now);
            }
        }
        Ok(())
    }

    fn circles(&self) -> Circles<'_> {
        Circles {
            me: self.config.identity.person_id,
            household: &self.config.household,
            neighborhoods: &self.neighborhoods,
        }
    }

    // --- read-only views ---

    pub fn me(&self) -> PersonId {
        self.config.identity.person_id
    }

    pub fn state(&self) -> &HouseholdState {
        &self.household
    }

    /// This device's person and keys, e.g. to back them up to the OS keystore.
    pub fn identity(&self) -> &LocalIdentity {
        &self.config.identity
    }

    pub fn household(&self) -> &Household {
        &self.config.household
    }

    pub fn device_id(&self) -> DeviceId {
        self.config.device_id
    }

    pub fn store(&self) -> &S {
        &self.store
    }

    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    pub fn pending_outbound(&self) -> usize {
        self.queue.len()
    }

    pub fn carrying_for_others(&self) -> usize {
        self.queue.tier_len(Tier::Relay)
    }

    pub fn should_sync(&self, now: UnixMillis) -> bool {
        self.schedule.is_due(now)
    }

    pub fn next_sync_at(&self) -> UnixMillis {
        self.schedule.next_at()
    }

    pub fn neighborhood(&self, id: &NeighborhoodId) -> Option<&NeighborhoodState> {
        self.neighborhoods.get(id)
    }

    pub fn neighborhoods(&self) -> Vec<&NeighborhoodState> {
        let mut all: Vec<_> = self.neighborhoods.values().collect();
        all.sort_by(|a, b| a.info.name.cmp(&b.info.name));
        all
    }

    /// Visible posts in a neighborhood: current members only, minus anyone
    /// this device has blocked.
    pub fn feed(&self, id: &NeighborhoodId) -> Vec<&Post> {
        let (Some(feed), Some(state)) = (self.feeds.get(id), self.neighborhoods.get(id)) else {
            return Vec::new();
        };
        feed.posts(|author| state.is_member(author) && !self.blocked.contains(author))
    }

    pub fn inbox(&self) -> &Inbox {
        &self.inbox
    }

    pub fn is_blocked(&self, person: &PersonId) -> bool {
        self.blocked.contains(person)
    }

    pub fn relay_enabled(&self) -> bool {
        self.config.relay_enabled
    }

    /// For persisting neighborhood keys between launches.
    pub fn joined_neighborhoods(&self) -> Vec<JoinedNeighborhood> {
        self.neighborhoods
            .values()
            .map(|s| JoinedNeighborhood {
                info: s.info.clone(),
                keys: s.keys().map(|(e, k)| (e, k.clone())).collect(),
            })
            .collect()
    }

    /// What to show as a QR code so a neighbor can invite this person.
    pub fn invite_request(&self) -> InviteRequest {
        let id = &self.config.identity;
        InviteRequest {
            person_id: id.person_id,
            display_name: id.display_name.clone(),
            identity_key: id.signing.public_bytes().to_vec(),
            encryption_key: id.encryption.public_bytes().to_vec(),
        }
    }

    // --- local settings ---

    pub fn set_relay_enabled(&mut self, enabled: bool) {
        self.config.relay_enabled = enabled;
        if !enabled {
            // Stop carrying what we were holding for others.
            let ids: Vec<_> = self
                .store
                .all(0)
                .unwrap_or_default()
                .into_iter()
                .filter(|e| e.tier == Tier::Relay)
                .map(|e| e.sealed.id)
                .collect();
            for id in ids {
                self.queue.remove(&id);
                let _ = self.store.remove(&id);
            }
        }
    }

    /// Hide a person's posts and messages on this phone and stop relaying
    /// their neighborhood posts. Doesn't remove them from any neighborhood.
    pub fn block(&mut self, person: PersonId) {
        self.blocked.insert(person);
    }

    pub fn unblock(&mut self, person: &PersonId) {
        self.blocked.remove(person);
    }

    // --- authoring ---

    /// Record a household update (check-in, meeting point, family note).
    pub fn record(&mut self, payload: Payload, now: UnixMillis) -> Result<Envelope, SyncError> {
        let audience = Audience::Household {
            household_id: self.config.household.id,
        };
        self.author(audience, payload, now)
    }

    /// Post to a neighborhood (alert, offer, need, household status, withdraw).
    pub fn post(
        &mut self,
        neighborhood: NeighborhoodId,
        payload: Payload,
        now: UnixMillis,
    ) -> Result<Envelope, SyncError> {
        if payload.is_membership_event() {
            return Err(SyncError::NotPermitted(
                "use the membership methods for membership events",
            ));
        }
        if !self.post_limits.allow(self.me(), now, POSTS_PER_DAY) {
            return Err(SyncError::LimitReached("neighborhood posts for today"));
        }
        self.author(
            Audience::Neighborhood {
                neighborhood_id: neighborhood,
            },
            payload,
            now,
        )
    }

    /// Send a private message, "Are you OK?" request, or reply.
    pub fn send_direct(
        &mut self,
        to: PersonId,
        payload: Payload,
        now: UnixMillis,
    ) -> Result<Envelope, SyncError> {
        if matches!(payload, Payload::KeyRotation { .. }) {
            return Err(SyncError::NotPermitted(
                "key rotation is sent by remove_member",
            ));
        }
        if !self.direct_limits.allow(self.me(), now, DIRECT_PER_DAY) {
            return Err(SyncError::LimitReached("direct messages for today"));
        }
        self.author(Audience::Direct { recipient: to }, payload, now)
    }

    fn author(
        &mut self,
        audience: Audience,
        payload: Payload,
        now: UnixMillis,
    ) -> Result<Envelope, SyncError> {
        let mut envelope = Envelope::new(NewEnvelope {
            audience,
            author: self.me(),
            origin_device: self.config.device_id,
            lamport: self.clock.current() + 1,
            now,
            payload,
        });
        match audience {
            Audience::Direct { recipient } => {
                let view = Sending {
                    circles: self.circles(),
                    recipient,
                };
                validate_envelope(&envelope, &view, now, &self.config.limits)?;
            }
            _ => validate_envelope(&envelope, &self.circles(), now, &self.config.limits)?,
        }
        self.clock.tick();
        self.config.identity.sign(&mut envelope);

        let sealed = self.seal(&envelope)?;
        let tier = Tier::for_scope(envelope.scope());
        self.store.insert(StoredEnvelope {
            sealed: sealed.clone(),
            opened: Some(envelope.clone()),
            tier,
            received_at: now,
        })?;
        self.apply_opened(&envelope);
        self.enqueue(sealed, tier, now);
        self.schedule.on_local_change(now);
        debug!(
            "recorded {} ({:?})",
            envelope.payload.kind(),
            envelope.scope()
        );
        Ok(envelope)
    }

    fn seal(&self, envelope: &Envelope) -> Result<SealedEnvelope, SyncError> {
        match envelope.audience {
            Audience::Household { .. } => {
                seal_envelope(envelope, SealFor::Group(&self.config.household_key))
            }
            Audience::Neighborhood { neighborhood_id } => {
                let (_, key) = self
                    .neighborhoods
                    .get(&neighborhood_id)
                    .and_then(|s| s.current_key())
                    .ok_or(SyncError::NotPermitted("not in that neighborhood"))?;
                seal_envelope(envelope, SealFor::Group(key))
            }
            Audience::Direct { recipient } => {
                let public = self
                    .encryption_key_of(&recipient)
                    .ok_or(SyncError::NotPermitted("no key on record for that person"))?;
                seal_envelope(envelope, SealFor::Person(&public))
            }
        }
    }

    fn encryption_key_of(&self, person: &PersonId) -> Option<Vec<u8>> {
        if let Some(key) = self
            .config
            .household
            .member(person)
            .and_then(|p| p.encryption_key.clone())
        {
            return Some(key);
        }
        self.neighborhoods
            .values()
            .find_map(|s| s.member(person).map(|c| c.encryption_key.clone()))
    }

    fn identity_key_of(&self, envelope: &Envelope) -> Option<Vec<u8>> {
        let author = &envelope.author;
        match (&envelope.audience, &envelope.payload) {
            (Audience::Household { .. }, _) => {
                self.config.household.member(author)?.public_key.clone()
            }
            (Audience::Neighborhood { .. }, Payload::MemberJoined(cert)) => {
                Some(cert.identity_key.clone())
            }
            (Audience::Neighborhood { neighborhood_id }, _) => Some(
                self.neighborhoods
                    .get(neighborhood_id)?
                    .member(author)?
                    .identity_key
                    .clone(),
            ),
            (Audience::Direct { .. }, _) => self
                .config
                .household
                .member(author)
                .and_then(|p| p.public_key.clone())
                .or_else(|| {
                    self.neighborhoods
                        .values()
                        .find_map(|s| s.member(author).map(|c| c.identity_key.clone()))
                }),
        }
    }

    // --- neighborhoods ---

    /// Start a new neighborhood with this person as founder and first steward.
    pub fn create_neighborhood(
        &mut self,
        name: &str,
        area: &str,
        now: UnixMillis,
    ) -> Result<NeighborhoodId, SyncError> {
        if self.neighborhoods.len() >= MAX_NEIGHBORHOODS {
            return Err(SyncError::LimitReached("neighborhoods per person"));
        }
        let identity = &self.config.identity;
        let info = Neighborhood {
            id: NeighborhoodId::new(),
            name: name.trim().to_owned(),
            area: area.trim().to_owned(),
            founder: identity.person_id,
            founder_identity_key: identity.signing.public_bytes().to_vec(),
            created_at: now,
        };
        let mut cert = MembershipCert {
            neighborhood_id: info.id,
            member: identity.person_id,
            display_name: identity.display_name.clone(),
            identity_key: identity.signing.public_bytes().to_vec(),
            encryption_key: identity.encryption.public_bytes().to_vec(),
            invited_by: None,
            invited_at: now,
            signature: Vec::new(),
        };
        cert.signature = identity.signing.sign(&cert.signing_bytes());

        let id = info.id;
        self.neighborhoods.insert(
            id,
            NeighborhoodState::new(info, 0, SymmetricKey::generate()),
        );
        if let Err(err) = self.author(
            Audience::Neighborhood {
                neighborhood_id: id,
            },
            Payload::MemberJoined(cert),
            now,
        ) {
            self.neighborhoods.remove(&id);
            return Err(err);
        }
        info!("created neighborhood {id}");
        Ok(id)
    }

    /// Invite someone standing in front of us. Returns what to hand them.
    pub fn invite(
        &mut self,
        neighborhood: NeighborhoodId,
        request: &InviteRequest,
        now: UnixMillis,
    ) -> Result<InvitePackage, SyncError> {
        let me = self.me();
        let state = self
            .neighborhoods
            .get(&neighborhood)
            .ok_or(SyncError::NotPermitted("not in that neighborhood"))?;
        if !state.is_member(&me) {
            return Err(SyncError::NotPermitted("only members can invite"));
        }
        if state.is_removed(&request.person_id) {
            return Err(SyncError::NotPermitted(
                "that person was removed from this neighborhood",
            ));
        }
        if state.invites_in_last_day(&me, now) >= INVITES_PER_DAY {
            return Err(SyncError::LimitReached("invites for today"));
        }
        if state.over_soft_cap() {
            warn!(
                "neighborhood {} has {} members; consider splitting it",
                state.info.name,
                state.member_count()
            );
        }
        let (epoch, key) = state
            .current_key()
            .ok_or(SyncError::NotPermitted("no neighborhood key"))?;
        let mut cert = MembershipCert {
            neighborhood_id: neighborhood,
            member: request.person_id,
            display_name: request.display_name.trim().to_owned(),
            identity_key: request.identity_key.clone(),
            encryption_key: request.encryption_key.clone(),
            invited_by: Some(me),
            invited_at: now,
            signature: Vec::new(),
        };
        cert.signature = self.config.identity.signing.sign(&cert.signing_bytes());
        let package = InvitePackage {
            neighborhood: state.info.clone(),
            epoch,
            key: key.clone(),
            cert,
            roster: state.events().to_vec(),
        };
        if let Some(state) = self.neighborhoods.get_mut(&neighborhood) {
            state.record_issued_invite(me, request.person_id, now);
        }
        Ok(package)
    }

    /// Join using a package handed over in person.
    pub fn accept_invite(
        &mut self,
        package: InvitePackage,
        now: UnixMillis,
    ) -> Result<NeighborhoodId, SyncError> {
        let identity = &self.config.identity;
        if package.cert.member != identity.person_id
            || package.cert.identity_key != identity.signing.public_bytes()
            || package.cert.neighborhood_id != package.neighborhood.id
        {
            return Err(SyncError::NotPermitted("this invite is for someone else"));
        }
        let id = package.neighborhood.id;
        if !self.neighborhoods.contains_key(&id) && self.neighborhoods.len() >= MAX_NEIGHBORHOODS {
            return Err(SyncError::LimitReached("neighborhoods per person"));
        }
        let state = self.neighborhoods.entry(id).or_insert_with(|| {
            NeighborhoodState::new(
                package.neighborhood.clone(),
                package.epoch,
                package.key.clone(),
            )
        });
        state.add_key(package.epoch, package.key.clone());

        // Learn the roster: joins first (each verifies with its own key), then
        // the rest against the members those joins establish.
        let (joins, others): (Vec<_>, Vec<_>) = package
            .roster
            .into_iter()
            .partition(|e| matches!(e.payload, Payload::MemberJoined(_)));
        for env in joins.into_iter().chain(others) {
            if let Err(err) = self.accept_roster_event(&env, now) {
                debug!("ignored roster event {}: {err}", env.id);
            }
        }

        self.author(
            Audience::Neighborhood {
                neighborhood_id: id,
            },
            Payload::MemberJoined(package.cert),
            now,
        )?;
        if !self
            .neighborhoods
            .get(&id)
            .is_some_and(|s| s.is_member(&self.me()))
        {
            warn!("joined {id}, but the invite chain isn't verifiable yet");
        }
        Ok(id)
    }

    fn accept_roster_event(&mut self, env: &Envelope, now: UnixMillis) -> Result<(), SyncError> {
        let Audience::Neighborhood { neighborhood_id } = env.audience else {
            return Err(SyncError::NotPermitted("not a neighborhood event"));
        };
        if self
            .neighborhoods
            .get(&neighborhood_id)
            .is_some_and(|s| s.has_event(&env.id))
        {
            return Ok(());
        }
        let key = self.identity_key_of(env).ok_or(IdentityError::UnknownKey)?;
        verify_envelope(env, &key)?;
        // Store it sealed under our key, so it survives restarts and can be
        // handed on to the next person we invite.
        let sealed = self.seal(env)?;
        self.store.insert(StoredEnvelope {
            sealed,
            opened: Some(env.clone()),
            tier: Tier::Neighborhood,
            received_at: now,
        })?;
        self.apply_opened(env);
        Ok(())
    }

    /// Founder only: let someone else remove members too.
    pub fn add_steward(
        &mut self,
        neighborhood: NeighborhoodId,
        member: PersonId,
        now: UnixMillis,
    ) -> Result<Envelope, SyncError> {
        let state = self
            .neighborhoods
            .get(&neighborhood)
            .ok_or(SyncError::NotPermitted("not in that neighborhood"))?;
        if !state.is_founder(&self.me()) {
            return Err(SyncError::NotPermitted(
                "only the founder can appoint stewards",
            ));
        }
        if !state.is_member(&member) {
            return Err(SyncError::NotPermitted("stewards must be members"));
        }
        self.author(
            Audience::Neighborhood {
                neighborhood_id: neighborhood,
            },
            Payload::StewardAdded { member },
            now,
        )
    }

    /// Steward only: remove someone (and optionally everyone they invited),
    /// then rotate the neighborhood key so they can't read new posts.
    pub fn remove_member(
        &mut self,
        neighborhood: NeighborhoodId,
        member: PersonId,
        reason: RemovalReason,
        cascade: bool,
        now: UnixMillis,
    ) -> Result<u32, SyncError> {
        let me = self.me();
        let state = self
            .neighborhoods
            .get(&neighborhood)
            .ok_or(SyncError::NotPermitted("not in that neighborhood"))?;
        if !state.is_steward(&me) {
            return Err(SyncError::NotPermitted("only stewards can remove people"));
        }
        if state.is_founder(&member) || member == me {
            return Err(SyncError::NotPermitted(
                "the founder and yourself can't be removed",
            ));
        }
        let audience = Audience::Neighborhood {
            neighborhood_id: neighborhood,
        };
        self.author(
            audience,
            Payload::MemberRemoved {
                member,
                reason,
                cascade,
            },
            now,
        )?;
        self.rotate_key(neighborhood, now)
    }

    /// Start a new key epoch and send it to every remaining member.
    fn rotate_key(
        &mut self,
        neighborhood: NeighborhoodId,
        now: UnixMillis,
    ) -> Result<u32, SyncError> {
        let me = self.me();
        let state = self
            .neighborhoods
            .get_mut(&neighborhood)
            .ok_or(SyncError::NotPermitted("not in that neighborhood"))?;
        let epoch = state.current_key().map_or(0, |(e, _)| e + 1);
        let key = SymmetricKey::generate();
        state.add_key(epoch, key.clone());
        let recipients: Vec<PersonId> = state
            .members()
            .into_iter()
            .map(|c| c.member)
            .filter(|p| *p != me)
            .collect();

        for recipient in recipients {
            let payload = Payload::KeyRotation {
                neighborhood_id: neighborhood,
                epoch,
                key: key.as_bytes().to_vec(),
            };
            if let Err(err) = self.author(Audience::Direct { recipient }, payload, now) {
                warn!("could not send new key to {recipient}: {err}");
            }
        }
        info!("rotated key for {neighborhood} to epoch {epoch}");
        Ok(epoch)
    }

    // --- receiving ---

    /// Recognize, open, check and store one envelope from a peer, or carry it
    /// on for others if it isn't ours to read.
    fn receive(&mut self, sealed: SealedEnvelope, now: UnixMillis) -> Receipt {
        if let Err(err) = validate_sealed(&sealed, now) {
            debug!("dropping malformed envelope {}: {err}", sealed.id);
            return Receipt::Rejected;
        }
        if self.store.contains(&sealed.id).unwrap_or(false) {
            return Receipt::Duplicate;
        }

        match self.try_open(&sealed) {
            Some(Ok(envelope)) => match self.accept(&envelope, now) {
                Ok(()) => {
                    self.store_readable(sealed, envelope, now);
                    Receipt::Readable
                }
                Err(err) => {
                    debug!("rejected {} from {}: {err}", envelope.id, envelope.author);
                    Receipt::Rejected
                }
            },
            Some(Err(err)) => {
                debug!(
                    "could not open {} although its tag matched: {err}",
                    sealed.id
                );
                Receipt::Rejected
            }
            None if self.config.relay_enabled => {
                let Some(relay) = sealed.for_relay() else {
                    return Receipt::Dropped;
                };
                let entry = StoredEnvelope {
                    sealed,
                    opened: None,
                    tier: Tier::Relay,
                    received_at: now,
                };
                if self.store.insert(entry).is_err() {
                    return Receipt::Dropped;
                }
                self.enqueue(relay, Tier::Relay, now);
                Receipt::Relayed
            }
            None => Receipt::Dropped,
        }
    }

    fn try_open(&self, sealed: &SealedEnvelope) -> Option<Result<Envelope, SyncError>> {
        match sealed.scope {
            Scope::Household => is_for_group(sealed, &self.config.household_key)
                .then(|| open_group(sealed, &self.config.household_key)),
            Scope::Neighborhood => self
                .neighborhoods
                .values()
                .flat_map(|s| s.keys())
                .find(|(_, key)| is_for_group(sealed, key))
                .map(|(_, key)| open_group(sealed, key)),
            Scope::Direct => {
                let me = &self.config.identity.encryption;
                is_for_person(sealed, &me.public_bytes()).then(|| open_direct(sealed, me))
            }
        }
    }

    /// Checks on an opened envelope: well-formed, from someone who may send
    /// it, correctly signed, within rate limits.
    fn accept(&mut self, env: &Envelope, now: UnixMillis) -> Result<(), SyncError> {
        validate_envelope(env, &self.circles(), now, &self.config.limits)?;
        let key = self.identity_key_of(env).ok_or(IdentityError::UnknownKey)?;
        verify_envelope(env, &key)?;

        match (&env.audience, &env.payload) {
            (
                Audience::Direct { .. },
                Payload::KeyRotation {
                    neighborhood_id, ..
                },
            ) => {
                let is_steward = self
                    .neighborhoods
                    .get(neighborhood_id)
                    .is_some_and(|s| s.is_steward(&env.author));
                if !is_steward {
                    return Err(SyncError::NotPermitted("key rotation from a non-steward"));
                }
            }
            (Audience::Direct { .. }, _) => {
                if !self
                    .direct_limits
                    .allow(env.author, env.created_at, DIRECT_PER_DAY)
                {
                    return Err(SyncError::LimitReached(
                        "sender's direct messages for today",
                    ));
                }
            }
            (Audience::Neighborhood { .. }, p)
                if !p.is_membership_event()
                    && !self
                        .post_limits
                        .allow(env.author, env.created_at, POSTS_PER_DAY) =>
            {
                return Err(SyncError::LimitReached("author's posts for today"));
            }
            _ => {}
        }
        Ok(())
    }

    fn store_readable(&mut self, sealed: SealedEnvelope, envelope: Envelope, now: UnixMillis) {
        let tier = Tier::for_scope(envelope.scope());
        // Pass it on to the rest of the circle, unless it was a direct
        // message that has now reached its recipient, or it's from someone
        // we've blocked.
        let forward = match envelope.audience {
            Audience::Direct { .. } => None,
            _ if self.blocked.contains(&envelope.author) => None,
            _ => sealed.for_relay().map(|mut copy| {
                // A sender can claim "urgent" in the header; readers check
                // and quietly downgrade false claims for the next hop.
                if copy.priority_class == PriorityClass::Urgent
                    && envelope.priority() != Priority::Urgent
                {
                    copy.priority_class = PriorityClass::Normal;
                }
                copy
            }),
        };
        if let Err(err) = self.store.insert(StoredEnvelope {
            sealed,
            opened: Some(envelope.clone()),
            tier,
            received_at: now,
        }) {
            warn!("could not store {}: {err}", envelope.id);
            return;
        }
        self.apply_opened(&envelope);
        if let Some(copy) = forward {
            self.enqueue(copy, tier, now);
        }
    }

    /// Fold an opened envelope into the views.
    fn apply_opened(&mut self, env: &Envelope) {
        self.clock.observe(env.lamport);
        match env.audience {
            Audience::Household { .. } => {
                self.household.apply(env);
            }
            Audience::Neighborhood { neighborhood_id } => {
                if env.payload.is_membership_event() {
                    if let Some(state) = self.neighborhoods.get_mut(&neighborhood_id) {
                        state.apply_event(env);
                    }
                } else {
                    self.feeds.entry(neighborhood_id).or_default().apply(env);
                }
            }
            Audience::Direct { recipient } => match &env.payload {
                Payload::KeyRotation {
                    neighborhood_id,
                    epoch,
                    key,
                } => {
                    if recipient == self.me() {
                        if let (Some(state), Ok(key)) = (
                            self.neighborhoods.get_mut(neighborhood_id),
                            SymmetricKey::from_bytes(key),
                        ) {
                            state.add_key(*epoch, key);
                        }
                    }
                }
                _ => {
                    self.inbox.apply(env, recipient);
                }
            },
        }
    }

    fn enqueue(&mut self, sealed: SealedEnvelope, tier: Tier, now: UnixMillis) {
        match self.queue.push(sealed, tier, now) {
            PushOutcome::Queued => self.queue_generation += 1,
            PushOutcome::QueuedWithEviction(evicted) => {
                self.queue_generation += 1;
                if tier == Tier::Relay {
                    // Nothing else keeps unreadable traffic around.
                    for id in evicted {
                        let _ = self.store.remove(&id);
                    }
                }
            }
            PushOutcome::Rejected => debug!("{tier:?} queue full; dropped a lower-priority update"),
            PushOutcome::Duplicate | PushOutcome::NotForwardable => {}
        }
    }

    // --- sync rounds ---

    /// Run one sync round: greet peers in range, process everything received,
    /// push what peers are missing. Safe to call at any time; does nothing
    /// expensive when nobody is around.
    pub fn sync_round(&mut self, now: UnixMillis) -> RoundReport {
        let mut report = RoundReport::default();
        self.housekeeping(now);

        if !self.transport.is_available() {
            report.skipped = Some(format!("{:?} transport unavailable", self.transport.kind()));
            self.schedule.on_round_finished(now, false);
            return report;
        }

        match self.transport.peers() {
            Ok(peers) => {
                report.peers_in_range = peers.len();
                for peer in peers {
                    if self.should_initiate(&peer, now) {
                        report.sessions_started += 1;
                        self.start_session(peer, now, &mut report);
                    }
                }
            }
            Err(err) => {
                warn!("peer discovery failed: {err}");
                report.errors += 1;
            }
        }

        match self.transport.poll() {
            Ok(frames) => {
                for frame in frames {
                    if let Err(err) = self.handle_frame(&frame.from, frame.bytes, now, &mut report)
                    {
                        warn!("error handling frame from {}: {err}", frame.from);
                        report.errors += 1;
                    }
                }
            }
            Err(err) => {
                warn!("receiving frames failed: {err}");
                report.errors += 1;
            }
        }

        self.finish_sessions(now, &mut report);
        self.schedule
            .on_round_finished(now, report.exchanged_anything());
        if report.exchanged_anything() {
            info!(
                "sync round: sent {}, received {}, relaying {}, rejected {}",
                report.envelopes_sent,
                report.envelopes_received,
                report.envelopes_relayed,
                report.envelopes_rejected
            );
        }
        report
    }

    fn housekeeping(&mut self, now: UnixMillis) {
        if let Err(err) = self.store.prune_expired(now) {
            warn!("pruning store failed: {err}");
        }
        self.queue.prune_expired(now);
        for feed in self.feeds.values_mut() {
            feed.prune(now, DAY_MS);
        }
        self.sessions.retain(|peer, s| {
            let keep = !s.is_timed_out(now);
            if !keep {
                debug!("session with {peer} timed out");
            }
            keep
        });
    }

    fn should_initiate(&self, peer: &PeerAddress, now: UnixMillis) -> bool {
        if self.sessions.contains_key(peer) {
            return false;
        }
        match self.recent_peers.get(peer) {
            None => true,
            Some((finished_at, generation)) => {
                *generation != self.queue_generation
                    || now.saturating_sub(*finished_at) >= self.config.peer_cooldown_ms
            }
        }
    }

    fn summary(&self, now: UnixMillis) -> IdSummary {
        let ids = self.store.ids(now).unwrap_or_default();
        // A fresh seed per hello, so a false positive doesn't repeat.
        let seed = now
            ^ (self.config.device_id.0.as_u128() as u64)
            ^ self.queue_generation.rotate_left(32);
        IdSummary::build(&ids, ids.len(), seed)
    }

    fn start_session(&mut self, peer: PeerAddress, now: UnixMillis, report: &mut RoundReport) {
        let hello = SyncMessage::Hello(Hello {
            protocol_version: PROTOCOL_VERSION,
            device_id: self.config.device_id,
            have: self.summary(now),
        });
        match self.send(&peer, &hello) {
            Ok(()) => {
                self.sessions
                    .insert(peer.clone(), SyncSession::new(peer, now));
            }
            Err(err) => {
                warn!("could not greet {peer}: {err}");
                report.errors += 1;
            }
        }
    }

    fn handle_frame(
        &mut self,
        from: &PeerAddress,
        bytes: Vec<u8>,
        now: UnixMillis,
        report: &mut RoundReport,
    ) -> Result<(), SyncError> {
        let message = decode_message(&bytes)?;

        if let SyncMessage::Reject { reason } = &message {
            info!("{from} declined to sync: {reason}");
            if let Some(session) = self.sessions.get_mut(from) {
                session.mark_rejected();
            }
            return Ok(());
        }

        // The peer initiated; answer with our own hello first.
        if !self.sessions.contains_key(from) {
            report.sessions_started += 1;
            self.start_session(from.clone(), now, report);
        }

        match message {
            SyncMessage::Hello(hello) => self.on_hello(from, hello, now, report),
            SyncMessage::Deliver { envelopes } => {
                for sealed in envelopes {
                    let receipt = self.receive(sealed, now);
                    let Some(session) = self.sessions.get_mut(from) else {
                        continue;
                    };
                    match receipt {
                        Receipt::Readable => {
                            session.stats.received += 1;
                            report.envelopes_received += 1;
                        }
                        Receipt::Relayed => {
                            session.stats.received += 1;
                            report.envelopes_relayed += 1;
                        }
                        Receipt::Duplicate | Receipt::Dropped => session.stats.duplicates += 1,
                        Receipt::Rejected => {
                            session.stats.rejected += 1;
                            report.envelopes_rejected += 1;
                        }
                    }
                }
                Ok(())
            }
            SyncMessage::Done => {
                if let Some(session) = self.sessions.get_mut(from) {
                    session.mark_received_done();
                }
                Ok(())
            }
            SyncMessage::Reject { .. } => Ok(()),
        }
    }

    fn on_hello(
        &mut self,
        from: &PeerAddress,
        hello: Hello,
        now: UnixMillis,
        report: &mut RoundReport,
    ) -> Result<(), SyncError> {
        if hello.protocol_version != PROTOCOL_VERSION {
            let reason = format!("unsupported protocol version {}", hello.protocol_version);
            if let Some(session) = self.sessions.get_mut(from) {
                session.mark_rejected();
            }
            return self.send(from, &SyncMessage::Reject { reason });
        }
        if let Some(session) = self.sessions.get_mut(from) {
            session.mark_hello_received(hello.device_id);
        }

        let batch = self.queue.batch_for(&hello.have, now);
        let count = batch.len();
        let max_frame = self
            .transport
            .max_frame_bytes()
            .min(beacon_protocol::codec::MAX_FRAME_BYTES);
        for frame in pack_deliveries(batch, max_frame)? {
            self.send_raw(from, frame)?;
        }
        self.send(from, &SyncMessage::Done)?;

        report.envelopes_sent += count;
        if let Some(session) = self.sessions.get_mut(from) {
            session.stats.sent += count;
            session.mark_sent_done();
        }
        Ok(())
    }

    fn finish_sessions(&mut self, now: UnixMillis, report: &mut RoundReport) {
        let finished: Vec<PeerAddress> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.is_finished())
            .map(|(p, _)| p.clone())
            .collect();
        for peer in finished {
            if let Some(session) = self.sessions.remove(&peer) {
                debug!("session with {peer} finished: {:?}", session.stats);
                report.sessions_completed += 1;
                self.recent_peers.insert(peer, (now, self.queue_generation));
            }
        }
    }

    fn send(&mut self, to: &PeerAddress, message: &SyncMessage) -> Result<(), SyncError> {
        let frame = encode_message(message)?;
        self.send_raw(to, frame)
    }

    fn send_raw(&mut self, to: &PeerAddress, frame: Vec<u8>) -> Result<(), SyncError> {
        match self.transport.send(to, frame) {
            Ok(()) => Ok(()),
            Err(err @ TransportError::PeerUnreachable(_)) => {
                // They walked away. Drop the session; we'll try again next time.
                self.sessions.remove(to);
                Err(err.into())
            }
            Err(err) => Err(err.into()),
        }
    }
}
