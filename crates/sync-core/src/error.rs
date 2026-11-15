use beacon_crypto::CryptoError;
use beacon_protocol::{CodecError, ValidationError};
use thiserror::Error;

use crate::identity::IdentityError;
use crate::store::StoreError;
use crate::transport::TransportError;

#[derive(Debug, Error)]
pub enum SyncError {
    #[error("invalid envelope: {0}")]
    Invalid(#[from] ValidationError),
    #[error("identity check failed: {0}")]
    Identity(#[from] IdentityError),
    #[error("encryption error: {0}")]
    Crypto(#[from] CryptoError),
    #[error("encoding error: {0}")]
    Codec(#[from] CodecError),
    #[error("storage error: {0}")]
    Store(#[from] StoreError),
    #[error("transport error: {0}")]
    Transport(#[from] TransportError),
    #[error("this device's person is not a member of the household")]
    LocalPersonNotMember,
    #[error("not allowed: {0}")]
    NotPermitted(&'static str),
    #[error("limit reached: {0}")]
    LimitReached(&'static str),
}
