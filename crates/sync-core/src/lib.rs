//! Family Beacon sync core.
//!
//! A small, synchronous, I/O-free engine that:
//!
//! * turns local actions into signed envelopes and seals them for their
//!   audience: the household, a neighborhood, or one person
//!   ([`sealing`]),
//! * keeps them in a local [`store::EnvelopeStore`] (SQLite on device:
//!   [`sqlite_store::SqliteStore`]) and a tiered, budgeted
//!   [`queue::OutboundQueue`],
//! * exchanges sealed envelopes with **any** nearby Family Beacon phone over
//!   a pluggable [`transport::Transport`], carrying what it can't read for
//!   others,
//! * tracks neighborhood membership through signed invite chains
//!   ([`neighborhood`]), and
//! * folds everything it can read into views the UI renders
//!   ([`conflict::HouseholdState`], [`feed`]).
//!
//! There is no async runtime and no background thread. The host app (iOS /
//! Android) decides *when* to call [`engine::SyncEngine::sync_round`], usually
//! when the OS wakes it for a BLE event, guided by [`schedule::SyncSchedule`].
//! That keeps battery behavior under the platform's control and makes the
//! whole engine deterministic in tests.
//!
//! Design: docs/neighborhood-network.md.

pub mod clock;
pub mod conflict;
pub mod engine;
pub mod error;
pub mod feed;
pub mod identity;
pub mod neighborhood;
pub mod queue;
pub mod schedule;
pub mod sealing;
pub mod session;
pub mod sqlite_store;
pub mod store;
pub mod transport;

pub use beacon_crypto as crypto;
pub use beacon_protocol as protocol;
pub use engine::{
    InvitePackage, InviteRequest, JoinedNeighborhood, RoundReport, SyncEngine, SyncEngineConfig,
};
pub use error::SyncError;
pub use identity::LocalIdentity;
