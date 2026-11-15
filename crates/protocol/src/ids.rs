//! Strongly typed identifiers.
//!
//! All IDs are random UUIDv4 values generated on-device. There is no central
//! authority handing out IDs, which is what lets two phones that have never
//! talked to a server agree on identity.

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! id_type {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            pub fn parse(s: &str) -> Result<Self, uuid::Error> {
                Uuid::parse_str(s).map(Self)
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

id_type!(
    /// A person in a trusted circle.
    PersonId
);
id_type!(
    /// A household / trusted circle.
    HouseholdId
);
id_type!(
    /// A single installation of the app. One person may own several devices.
    DeviceId
);
id_type!(
    /// A unique envelope. Used for de-duplication during store-and-forward.
    EnvelopeId
);
id_type!(
    /// A rendezvous (meeting) point.
    RendezvousId
);
id_type!(
    /// A neighborhood network.
    NeighborhoodId
);
