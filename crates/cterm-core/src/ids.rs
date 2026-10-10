//! Strongly-typed identifiers.
//!
//! We deliberately avoid passing bare `Uuid`/`String` values around: a
//! `SessionId` and an `ExecutionId` are not interchangeable, and the type
//! system should say so. Each id is a thin newtype over a v4 [`Uuid`].

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Generates the newtype id boilerplate for a given name.
macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident, $prefix:literal) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// Creates a fresh random id.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            /// Wraps an existing [`Uuid`].
            #[must_use]
            pub const fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }

            /// The underlying [`Uuid`].
            #[must_use]
            pub const fn as_uuid(&self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                // A short, human-scannable prefix plus the uuid.
                write!(f, concat!($prefix, "-{}"), self.0)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                // Accept both the prefixed display form and a bare uuid.
                let raw = s.strip_prefix(concat!($prefix, "-")).unwrap_or(s);
                Ok(Self(Uuid::parse_str(raw)?))
            }
        }
    };
}

define_id! {
    /// Identifies one terminal session (one shell lifetime, one host).
    SessionId, "ses"
}

define_id! {
    /// Identifies one command execution within a session.
    ExecutionId, "exe"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_roundtrip_through_display_and_fromstr() {
        let id = ExecutionId::new();
        let parsed: ExecutionId = id.to_string().parse().expect("display form parses");
        assert_eq!(id, parsed);
    }

    #[test]
    fn ids_parse_bare_uuid() {
        let uuid = Uuid::new_v4();
        let from_bare: SessionId = uuid.to_string().parse().expect("bare uuid parses");
        assert_eq!(from_bare, SessionId::from_uuid(uuid));
    }

    #[test]
    fn distinct_ids_are_distinct() {
        assert_ne!(ExecutionId::new(), ExecutionId::new());
    }
}
