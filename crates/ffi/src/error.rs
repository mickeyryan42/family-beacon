use beacon_sync_core::store::StoreError;
use beacon_sync_core::SyncError;

/// Errors crossing into Swift, Kotlin and TypeScript. Each variant is
/// something the app reacts to differently; `message` is for logs and, for
/// `Invalid`/`LimitReached`/`NotPermitted`, safe to adapt into UI copy.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum CoreError {
    /// Input the core refused: bad JSON, an invalid update, a malformed id.
    #[error("invalid: {message}")]
    Invalid { message: String },
    /// The action isn't allowed for this person (e.g. not a steward).
    #[error("not permitted: {message}")]
    NotPermitted { message: String },
    /// A daily or size limit was reached.
    #[error("limit reached: {message}")]
    LimitReached { message: String },
    /// The on-device database failed or holds data this version can't read.
    #[error("storage: {message}")]
    Storage { message: String },
    /// Keys are missing, damaged, or don't belong to this database.
    #[error("keys: {message}")]
    Secrets { message: String },
    /// A bug. Should never reach the UI as anything but a generic error.
    #[error("internal: {message}")]
    Internal { message: String },
}

impl CoreError {
    pub(crate) fn invalid(message: impl ToString) -> Self {
        Self::Invalid {
            message: message.to_string(),
        }
    }

    pub(crate) fn secrets(message: impl ToString) -> Self {
        Self::Secrets {
            message: message.to_string(),
        }
    }

    pub(crate) fn internal(message: impl ToString) -> Self {
        Self::Internal {
            message: message.to_string(),
        }
    }
}

impl From<SyncError> for CoreError {
    fn from(e: SyncError) -> Self {
        let message = e.to_string();
        match e {
            SyncError::Invalid(_) | SyncError::Codec(_) => Self::Invalid { message },
            SyncError::NotPermitted(_) | SyncError::LocalPersonNotMember => {
                Self::NotPermitted { message }
            }
            SyncError::LimitReached(_) => Self::LimitReached { message },
            SyncError::Store(_) => Self::Storage { message },
            SyncError::Identity(_) | SyncError::Crypto(_) => Self::Secrets { message },
            SyncError::Transport(_) => Self::Internal { message },
        }
    }
}

impl From<StoreError> for CoreError {
    fn from(e: StoreError) -> Self {
        Self::Storage {
            message: e.to_string(),
        }
    }
}

impl From<serde_json::Error> for CoreError {
    fn from(e: serde_json::Error) -> Self {
        Self::invalid(e)
    }
}
