//! What the bridge keeps between launches, split by sensitivity.
//!
//! * **Secrets** (private keys, the household key, neighborhood keys) are
//!   handed to the app as one opaque string to keep in the OS keystore
//!   (Keychain / Android Keystore). They never touch the database.
//! * **Profile** (device and person ids, the household roster with public
//!   keys only, relay setting, blocks) lives in the SQLite `settings` table.
//!
//! Envelopes themselves are in the SQLite `envelopes` table, owned by
//! `SqliteStore`.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use beacon_sync_core::crypto::{EncryptionKeypair, IdentityKeypair, SymmetricKey};
use beacon_sync_core::protocol::{DeviceId, Household, Neighborhood, PersonId};
use beacon_sync_core::{JoinedNeighborhood, LocalIdentity};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::error::CoreError;

const SECRETS_VERSION: u8 = 1;
const PROFILE_VERSION: u8 = 1;
pub(crate) const PROFILE_KEY: &str = "ffi.profile";

#[derive(Serialize, Deserialize)]
struct SecretsFile {
    version: u8,
    /// Ties the keys to one database, so mismatched keys fail loudly.
    device_id: DeviceId,
    signing_key: String,
    encryption_key: String,
    household_key: String,
    neighborhoods: Vec<NeighborhoodSecret>,
}

#[derive(Serialize, Deserialize)]
struct NeighborhoodSecret {
    info: Neighborhood,
    keys: Vec<EpochKey>,
}

#[derive(Serialize, Deserialize)]
struct EpochKey {
    epoch: u32,
    key: String,
}

pub(crate) struct Secrets {
    pub device_id: DeviceId,
    pub signing: IdentityKeypair,
    pub encryption: EncryptionKeypair,
    pub household_key: SymmetricKey,
    pub neighborhoods: Vec<JoinedNeighborhood>,
}

pub(crate) fn encode_secrets(
    device_id: DeviceId,
    identity: &LocalIdentity,
    household_key: &SymmetricKey,
    neighborhoods: &[JoinedNeighborhood],
) -> Result<Zeroizing<String>, CoreError> {
    let file = Zeroizing::new(SecretsFile {
        version: SECRETS_VERSION,
        device_id,
        signing_key: B64.encode(*identity.signing.secret_bytes()),
        encryption_key: B64.encode(*identity.encryption.secret_bytes()),
        household_key: B64.encode(household_key.as_bytes()),
        neighborhoods: neighborhoods
            .iter()
            .map(|n| NeighborhoodSecret {
                info: n.info.clone(),
                keys: n
                    .keys
                    .iter()
                    .map(|(epoch, key)| EpochKey {
                        epoch: *epoch,
                        key: B64.encode(key.as_bytes()),
                    })
                    .collect(),
            })
            .collect(),
    });
    Ok(Zeroizing::new(
        serde_json::to_string(&*file).map_err(CoreError::internal)?,
    ))
}

pub(crate) fn decode_secrets(raw: &str) -> Result<Secrets, CoreError> {
    let file: SecretsFile =
        serde_json::from_str(raw).map_err(|e| CoreError::secrets(format!("unreadable: {e}")))?;
    if file.version != SECRETS_VERSION {
        return Err(CoreError::secrets(format!(
            "unsupported key format v{}",
            file.version
        )));
    }
    let neighborhoods = file
        .neighborhoods
        .iter()
        .map(|n| {
            let keys = n
                .keys
                .iter()
                .map(|k| {
                    Ok((
                        k.epoch,
                        SymmetricKey::from_bytes(&bytes(&k.key)?).map_err(CoreError::secrets)?,
                    ))
                })
                .collect::<Result<Vec<_>, CoreError>>()?;
            Ok(JoinedNeighborhood {
                info: n.info.clone(),
                keys,
            })
        })
        .collect::<Result<Vec<_>, CoreError>>()?;
    Ok(Secrets {
        device_id: file.device_id,
        signing: IdentityKeypair::from_secret_bytes(&bytes(&file.signing_key)?)
            .map_err(CoreError::secrets)?,
        encryption: EncryptionKeypair::from_secret_bytes(&bytes(&file.encryption_key)?)
            .map_err(CoreError::secrets)?,
        household_key: SymmetricKey::from_bytes(&bytes(&file.household_key)?)
            .map_err(CoreError::secrets)?,
        neighborhoods,
    })
}

fn bytes(b64: &str) -> Result<Zeroizing<Vec<u8>>, CoreError> {
    B64.decode(b64)
        .map(Zeroizing::new)
        .map_err(|e| CoreError::secrets(format!("bad key encoding: {e}")))
}

impl zeroize::Zeroize for SecretsFile {
    fn zeroize(&mut self) {
        self.signing_key.zeroize();
        self.encryption_key.zeroize();
        self.household_key.zeroize();
        for n in &mut self.neighborhoods {
            for k in &mut n.keys {
                k.key.zeroize();
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Profile {
    pub version: u8,
    pub device_id: DeviceId,
    pub person_id: PersonId,
    pub display_name: String,
    pub household: Household,
    pub relay_enabled: bool,
    #[serde(default)]
    pub blocked: Vec<PersonId>,
}

impl Profile {
    pub fn new(device_id: DeviceId, identity: &LocalIdentity, household: Household) -> Self {
        Self {
            version: PROFILE_VERSION,
            device_id,
            person_id: identity.person_id,
            display_name: identity.display_name.clone(),
            household,
            relay_enabled: true,
            blocked: Vec::new(),
        }
    }

    pub fn decode(raw: &str) -> Result<Self, CoreError> {
        let profile: Self = serde_json::from_str(raw).map_err(|e| CoreError::Storage {
            message: format!("profile is damaged: {e}"),
        })?;
        if profile.version != PROFILE_VERSION {
            return Err(CoreError::Storage {
                message: format!("unsupported profile v{}", profile.version),
            });
        }
        Ok(profile)
    }
}
