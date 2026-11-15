//! UniFFI bridge between the Family Beacon apps and the Rust sync core.
//!
//! One [`BeaconCore`] per app process wraps a [`SyncEngine`] over the
//! on-device SQLite store and a native-fed radio seam ([`bridge`]). The
//! surface is deliberately small and coarse:
//!
//! * domain data crosses as **protocol JSON**: the same shapes the
//!   TypeScript app already uses (`Payload`, `Household`, views), so screens
//!   don't change when the TypeScript mirror is retired;
//! * control calls (sync rounds, radio state, frames) use typed records;
//! * time comes from the system clock inside Rust, so no call takes a
//!   timestamp (JS would otherwise pass 64-bit integers as `bigint`).
//!
//! Persistence is split by sensitivity ([`persist`]): keys go to the app as
//! one opaque string for the OS keystore; everything else is in SQLite.
//!
//! All methods are synchronous and short. Call them off the UI thread.

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use beacon_sync_core::crypto::SymmetricKey;
use beacon_sync_core::protocol::{
    DeviceId, Envelope, Household, HouseholdId, MembershipCert, Neighborhood, NeighborhoodId,
    Payload, PersonId, PersonRole, RemovalReason, UnixMillis,
};
use beacon_sync_core::sqlite_store::SqliteStore;
use beacon_sync_core::{
    InvitePackage, InviteRequest, LocalIdentity, RoundReport, SyncEngine, SyncEngineConfig,
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

mod bridge;
mod error;
mod persist;
mod views;

pub use error::CoreError;

use bridge::BridgeTransport;
use persist::{decode_secrets, encode_secrets, Profile, PROFILE_KEY};

uniffi::setup_scaffolding!();

type Engine = SyncEngine<SqliteStore, BridgeTransport>;
type Clock = Box<dyn Fn() -> UnixMillis + Send>;

/// A frame for the native radio layer to transmit.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct OutboundFrame {
    /// Transport address of the peer, as given to [`BeaconCore::set_peers`].
    pub to: String,
    pub bytes: Vec<u8>,
}

/// What one sync round did.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncReport {
    pub peers_in_range: u32,
    pub sessions_started: u32,
    pub sessions_completed: u32,
    pub envelopes_sent: u32,
    pub envelopes_received: u32,
    pub envelopes_relayed: u32,
    pub envelopes_rejected: u32,
    pub errors: u32,
    pub skipped: Option<String>,
    /// Frames now waiting in [`BeaconCore::take_outbound`].
    pub frames_to_send: u32,
    /// Whether this round changed what the screens show.
    pub view_changed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Removal {
    Spam,
    Abuse,
    LeftArea,
    Other,
}

impl From<Removal> for RemovalReason {
    fn from(r: Removal) -> Self {
        match r {
            Removal::Spam => RemovalReason::Spam,
            Removal::Abuse => RemovalReason::Abuse,
            Removal::LeftArea => RemovalReason::LeftArea,
            Removal::Other => RemovalReason::Other,
        }
    }
}

/// Whether this database already holds a circle (so the app opens it rather
/// than showing onboarding).
#[uniffi::export]
pub fn has_stored_circle(db_path: String) -> Result<bool, CoreError> {
    if !Path::new(&db_path).exists() {
        return Ok(false);
    }
    Ok(SqliteStore::open(&db_path)?.setting(PROFILE_KEY)?.is_some())
}

/// Delete the database. The app must also delete the keys it keeps.
#[uniffi::export]
pub fn erase_stored_circle(db_path: String) -> Result<(), CoreError> {
    for suffix in ["", "-wal", "-shm"] {
        let path = format!("{db_path}{suffix}");
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(CoreError::Storage {
                    message: format!("could not delete {path}: {e}"),
                })
            }
        }
    }
    Ok(())
}

#[derive(uniffi::Object)]
pub struct BeaconCore {
    inner: Mutex<Inner>,
}

struct Inner {
    engine: Engine,
    profile: Profile,
    household_key: SymmetricKey,
    /// Last secrets handed out, to tell the app when to re-save them.
    exported_secrets: Option<Zeroizing<String>>,
    clock: Clock,
}

#[uniffi::export]
impl BeaconCore {
    /// Start a new circle on this phone, with this person as its first adult.
    #[uniffi::constructor]
    pub fn create_household(
        db_path: String,
        household_name: String,
        your_name: String,
    ) -> Result<Arc<Self>, CoreError> {
        Self::create_with_clock(&db_path, &household_name, &your_name, Box::new(system_now))
            .map(Arc::new)
    }

    /// Reopen the circle in `db_path` with the keys from [`Self::secrets`].
    #[uniffi::constructor]
    pub fn open(db_path: String, secrets: String) -> Result<Arc<Self>, CoreError> {
        Self::open_with_clock(&db_path, &secrets, Box::new(system_now)).map(Arc::new)
    }

    // --- keys ---

    /// Everything secret, as one string for the OS keystore. Treat it like a
    /// password: never log it, never send it anywhere.
    pub fn secrets(&self) -> Result<String, CoreError> {
        let mut inner = self.lock()?;
        let secrets = inner.encode_secrets()?;
        inner.exported_secrets = Some(secrets.clone());
        Ok((*secrets).clone())
    }

    /// The secrets, only if they changed since last handed out (after
    /// creating, joining or leaving a neighborhood, or a key rotation that
    /// arrived during sync). Check after any such call and after sync rounds.
    pub fn take_changed_secrets(&self) -> Result<Option<String>, CoreError> {
        let mut inner = self.lock()?;
        let secrets = inner.encode_secrets()?;
        if inner.exported_secrets.as_ref() == Some(&secrets) {
            return Ok(None);
        }
        inner.exported_secrets = Some(secrets.clone());
        Ok(Some((*secrets).clone()))
    }

    // --- reading ---

    /// Everything the screens show, as JSON. See `views.rs` for the shape.
    pub fn snapshot_json(&self) -> Result<String, CoreError> {
        let inner = self.lock()?;
        let now = (inner.clock)();
        let snap = views::snapshot(&inner.engine, inner.profile.blocked.clone(), now);
        Ok(serde_json::to_string(&snap)?)
    }

    // --- authoring (payloads are protocol JSON: {"type": ..., "data": ...}) ---

    /// Record a household update. Returns the new envelope id.
    pub fn record(&self, payload_json: String) -> Result<String, CoreError> {
        let payload: Payload = serde_json::from_str(&payload_json)?;
        let mut inner = self.lock()?;
        let now = (inner.clock)();
        Ok(inner.engine.record(payload, now)?.id.to_string())
    }

    /// Post to a neighborhood. Returns the new envelope id.
    pub fn post(&self, neighborhood_id: String, payload_json: String) -> Result<String, CoreError> {
        let neighborhood = parse_id(&neighborhood_id, NeighborhoodId::parse)?;
        let payload: Payload = serde_json::from_str(&payload_json)?;
        let mut inner = self.lock()?;
        let now = (inner.clock)();
        Ok(inner
            .engine
            .post(neighborhood, payload, now)?
            .id
            .to_string())
    }

    /// Send a private message, "Are you OK?" or reply. Returns the envelope id.
    pub fn send_direct(&self, to: String, payload_json: String) -> Result<String, CoreError> {
        let to = parse_id(&to, PersonId::parse)?;
        let payload: Payload = serde_json::from_str(&payload_json)?;
        let mut inner = self.lock()?;
        let now = (inner.clock)();
        Ok(inner.engine.send_direct(to, payload, now)?.id.to_string())
    }

    // --- neighborhoods ---

    pub fn create_neighborhood(&self, name: String, area: String) -> Result<String, CoreError> {
        let mut inner = self.lock()?;
        let now = (inner.clock)();
        Ok(inner
            .engine
            .create_neighborhood(&name, &area, now)?
            .to_string())
    }

    /// What this person shows (as a QR code) so a neighbor can invite them.
    pub fn invite_request_json(&self) -> Result<String, CoreError> {
        let inner = self.lock()?;
        Ok(serde_json::to_string(&InviteRequestDto::from(
            inner.engine.invite_request(),
        ))?)
    }

    /// Invite the person whose request was scanned. Returns the package to
    /// hand back to them in person. It contains the neighborhood key.
    pub fn invite(
        &self,
        neighborhood_id: String,
        request_json: String,
    ) -> Result<String, CoreError> {
        let neighborhood = parse_id(&neighborhood_id, NeighborhoodId::parse)?;
        let request: InviteRequest =
            serde_json::from_str::<InviteRequestDto>(&request_json)?.try_into()?;
        let mut inner = self.lock()?;
        let now = (inner.clock)();
        let package = inner.engine.invite(neighborhood, &request, now)?;
        Ok(serde_json::to_string(&InvitePackageDto::from(&package))?)
    }

    /// Join with a package received in person. Returns the neighborhood id.
    pub fn accept_invite(&self, package_json: String) -> Result<String, CoreError> {
        let package: InvitePackageDto = serde_json::from_str(&package_json)?;
        let package = package.into_package()?;
        let mut inner = self.lock()?;
        let now = (inner.clock)();
        Ok(inner.engine.accept_invite(package, now)?.to_string())
    }

    pub fn add_steward(&self, neighborhood_id: String, member: String) -> Result<(), CoreError> {
        let neighborhood = parse_id(&neighborhood_id, NeighborhoodId::parse)?;
        let member = parse_id(&member, PersonId::parse)?;
        let mut inner = self.lock()?;
        let now = (inner.clock)();
        inner.engine.add_steward(neighborhood, member, now)?;
        Ok(())
    }

    /// Remove a member (stewards only) and rotate the neighborhood key.
    /// Returns how many people were removed, counting a cascade.
    pub fn remove_member(
        &self,
        neighborhood_id: String,
        member: String,
        reason: Removal,
        cascade: bool,
    ) -> Result<u32, CoreError> {
        let neighborhood = parse_id(&neighborhood_id, NeighborhoodId::parse)?;
        let member = parse_id(&member, PersonId::parse)?;
        let mut inner = self.lock()?;
        let now = (inner.clock)();
        Ok(inner
            .engine
            .remove_member(neighborhood, member, reason.into(), cascade, now)?)
    }

    // --- local settings ---

    pub fn set_relay_enabled(&self, enabled: bool) -> Result<(), CoreError> {
        let mut inner = self.lock()?;
        inner.engine.set_relay_enabled(enabled);
        inner.profile.relay_enabled = enabled;
        inner.save_profile()
    }

    /// Hide someone's posts and messages on this phone only.
    pub fn block(&self, person: String) -> Result<(), CoreError> {
        let person = parse_id(&person, PersonId::parse)?;
        let mut inner = self.lock()?;
        inner.engine.block(person);
        if !inner.profile.blocked.contains(&person) {
            inner.profile.blocked.push(person);
        }
        inner.save_profile()
    }

    pub fn unblock(&self, person: String) -> Result<(), CoreError> {
        let person = parse_id(&person, PersonId::parse)?;
        let mut inner = self.lock()?;
        inner.engine.unblock(&person);
        inner.profile.blocked.retain(|p| p != &person);
        inner.save_profile()
    }

    // --- radio seam (driven by the native BLE / Wi-Fi Direct layer) ---

    /// This phone's address as the native transport advertises it.
    pub fn local_address(&self) -> Result<String, CoreError> {
        let inner = self.lock()?;
        Ok(inner.profile.device_id.to_string())
    }

    /// Whether the radio is present, permitted and on.
    pub fn set_radio_available(&self, available: bool) -> Result<(), CoreError> {
        self.lock()?.engine.transport_mut().set_available(available);
        Ok(())
    }

    /// Peers currently in range, by transport address.
    pub fn set_peers(&self, peers: Vec<String>) -> Result<(), CoreError> {
        self.lock()?.engine.transport_mut().set_peers(peers);
        Ok(())
    }

    /// Largest frame the active transport carries (BLE: 8 KiB).
    pub fn set_max_frame_bytes(&self, bytes: u32) -> Result<(), CoreError> {
        let bytes = usize::try_from(bytes).map_err(CoreError::internal)?;
        self.lock()?
            .engine
            .transport_mut()
            .set_max_frame_bytes(bytes);
        Ok(())
    }

    /// A frame arrived from `from`. Returns false if it was dropped because
    /// too many frames are waiting; run a sync round and resend later.
    pub fn receive_frame(&self, from: String, bytes: Vec<u8>) -> Result<bool, CoreError> {
        Ok(self
            .lock()?
            .engine
            .transport_mut()
            .push_inbound(from, bytes))
    }

    /// Exchange with peers in range: process frames received so far and
    /// queue replies. Then send everything from [`Self::take_outbound`].
    pub fn sync_round(&self) -> Result<SyncReport, CoreError> {
        let mut inner = self.lock()?;
        let now = (inner.clock)();
        let report = inner.engine.sync_round(now);
        let frames = inner.engine.transport_mut().pending_outbound();
        Ok(sync_report(&report, frames))
    }

    pub fn take_outbound(&self) -> Result<Vec<OutboundFrame>, CoreError> {
        Ok(self
            .lock()?
            .engine
            .transport_mut()
            .take_outbound()
            .into_iter()
            .map(|(to, bytes)| OutboundFrame { to: to.0, bytes })
            .collect())
    }

    /// Whether a sync round is due (battery-aware backoff).
    pub fn should_sync(&self) -> Result<bool, CoreError> {
        let inner = self.lock()?;
        Ok(inner.engine.should_sync((inner.clock)()))
    }

    /// Milliseconds until the next round is due; 0 if due now.
    pub fn next_sync_in_ms(&self) -> Result<u32, CoreError> {
        let inner = self.lock()?;
        let wait = inner.engine.next_sync_at().saturating_sub((inner.clock)());
        Ok(u32::try_from(wait).unwrap_or(u32::MAX))
    }
}

impl BeaconCore {
    fn lock(&self) -> Result<MutexGuard<'_, Inner>, CoreError> {
        self.inner
            .lock()
            .map_err(|_| CoreError::internal("core was poisoned by an earlier panic"))
    }

    fn create_with_clock(
        db_path: &str,
        household_name: &str,
        your_name: &str,
        clock: Clock,
    ) -> Result<Self, CoreError> {
        let (household_name, your_name) = (household_name.trim(), your_name.trim());
        if household_name.is_empty() || your_name.is_empty() {
            return Err(CoreError::invalid(
                "a circle name and your name are required",
            ));
        }
        let store = SqliteStore::open(db_path)?;
        if store.setting(PROFILE_KEY)?.is_some() {
            return Err(CoreError::NotPermitted {
                message: "this phone already has a circle; erase it first".into(),
            });
        }
        let identity = LocalIdentity::generate(your_name);
        let household = Household {
            id: HouseholdId::new(),
            name: household_name.into(),
            members: vec![identity.as_person(PersonRole::Adult)],
        };
        let device_id = DeviceId::new();
        let profile = Profile::new(device_id, &identity, household.clone());
        let household_key = SymmetricKey::generate();
        let config = SyncEngineConfig::new(device_id, identity, household, household_key.clone());
        let core = Self::start(store, config, profile, household_key, clock)?;
        core.lock()?.save_profile()?;
        Ok(core)
    }

    fn open_with_clock(db_path: &str, secrets: &str, clock: Clock) -> Result<Self, CoreError> {
        let store = SqliteStore::open(db_path)?;
        let raw = store
            .setting(PROFILE_KEY)?
            .ok_or_else(|| CoreError::Storage {
                message: "no circle on this phone".into(),
            })?;
        let profile = Profile::decode(&raw)?;
        let secrets = decode_secrets(secrets)?;
        if secrets.device_id != profile.device_id {
            return Err(CoreError::secrets(
                "these keys belong to a different install",
            ));
        }
        let identity = LocalIdentity {
            person_id: profile.person_id,
            display_name: profile.display_name.clone(),
            signing: secrets.signing,
            encryption: secrets.encryption,
        };
        let mut config = SyncEngineConfig::new(
            profile.device_id,
            identity,
            profile.household.clone(),
            secrets.household_key.clone(),
        );
        config.relay_enabled = profile.relay_enabled;
        config.neighborhoods = secrets.neighborhoods;
        Self::start(store, config, profile, secrets.household_key, clock)
    }

    fn start(
        store: SqliteStore,
        config: SyncEngineConfig,
        profile: Profile,
        household_key: SymmetricKey,
        clock: Clock,
    ) -> Result<Self, CoreError> {
        let transport = BridgeTransport::new(profile.device_id.to_string());
        let mut engine = SyncEngine::new(config, store, transport, clock())?;
        for person in &profile.blocked {
            engine.block(*person);
        }
        Ok(Self {
            inner: Mutex::new(Inner {
                engine,
                profile,
                household_key,
                exported_secrets: None,
                clock,
            }),
        })
    }
}

impl Inner {
    fn encode_secrets(&self) -> Result<Zeroizing<String>, CoreError> {
        encode_secrets(
            self.profile.device_id,
            self.engine.identity(),
            &self.household_key,
            &self.engine.joined_neighborhoods(),
        )
    }

    fn save_profile(&self) -> Result<(), CoreError> {
        let raw = serde_json::to_string(&self.profile).map_err(CoreError::internal)?;
        Ok(self.engine.store().set_setting(PROFILE_KEY, &raw)?)
    }
}

fn sync_report(r: &RoundReport, frames_to_send: usize) -> SyncReport {
    let n = |v: usize| u32::try_from(v).unwrap_or(u32::MAX);
    SyncReport {
        peers_in_range: n(r.peers_in_range),
        sessions_started: n(r.sessions_started),
        sessions_completed: n(r.sessions_completed),
        envelopes_sent: n(r.envelopes_sent),
        envelopes_received: n(r.envelopes_received),
        envelopes_relayed: n(r.envelopes_relayed),
        envelopes_rejected: n(r.envelopes_rejected),
        errors: n(r.errors),
        skipped: r.skipped.clone(),
        frames_to_send: n(frames_to_send),
        view_changed: r.envelopes_received > 0,
    }
}

fn system_now() -> UnixMillis {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn parse_id<T, E: std::fmt::Display>(
    raw: &str,
    parse: impl Fn(&str) -> Result<T, E>,
) -> Result<T, CoreError> {
    parse(raw).map_err(|e| CoreError::invalid(format!("bad id {raw:?}: {e}")))
}

// --- in-person exchange formats (shown and scanned as QR codes) ---

#[derive(Serialize, Deserialize)]
struct InviteRequestDto {
    person_id: PersonId,
    display_name: String,
    identity_key: String,
    encryption_key: String,
}

impl From<InviteRequest> for InviteRequestDto {
    fn from(r: InviteRequest) -> Self {
        use base64::Engine as _;
        let b64 = base64::engine::general_purpose::STANDARD;
        Self {
            person_id: r.person_id,
            display_name: r.display_name,
            identity_key: b64.encode(r.identity_key),
            encryption_key: b64.encode(r.encryption_key),
        }
    }
}

impl TryFrom<InviteRequestDto> for InviteRequest {
    type Error = CoreError;

    fn try_from(r: InviteRequestDto) -> Result<Self, CoreError> {
        use base64::Engine as _;
        let b64 = base64::engine::general_purpose::STANDARD;
        let key = |k: &str| {
            b64.decode(k)
                .map_err(|e| CoreError::invalid(format!("bad key in invite request: {e}")))
        };
        Ok(Self {
            person_id: r.person_id,
            identity_key: key(&r.identity_key)?,
            encryption_key: key(&r.encryption_key)?,
            display_name: r.display_name,
        })
    }
}

#[derive(Serialize, Deserialize)]
struct InvitePackageDto {
    neighborhood: Neighborhood,
    epoch: u32,
    key: String,
    cert: MembershipCert,
    roster: Vec<Envelope>,
}

impl From<&InvitePackage> for InvitePackageDto {
    fn from(p: &InvitePackage) -> Self {
        use base64::Engine as _;
        Self {
            neighborhood: p.neighborhood.clone(),
            epoch: p.epoch,
            key: base64::engine::general_purpose::STANDARD.encode(p.key.as_bytes()),
            cert: p.cert.clone(),
            roster: p.roster.clone(),
        }
    }
}

impl InvitePackageDto {
    fn into_package(self) -> Result<InvitePackage, CoreError> {
        use base64::Engine as _;
        let key = Zeroizing::new(
            base64::engine::general_purpose::STANDARD
                .decode(&self.key)
                .map_err(|e| CoreError::invalid(format!("bad invite key: {e}")))?,
        );
        Ok(InvitePackage {
            neighborhood: self.neighborhood,
            epoch: self.epoch,
            key: SymmetricKey::from_bytes(&key).map_err(CoreError::invalid)?,
            cert: self.cert,
            roster: self.roster,
        })
    }
}

#[cfg(test)]
mod tests;
