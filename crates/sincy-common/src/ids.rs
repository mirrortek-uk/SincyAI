//! Strongly-typed identifiers. Every id is UUIDv4; tenant-scoped ids are paired
//! with AgencyId at the database layer (TDD 11.2 composite keys).

use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

macro_rules! id_type {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
            pub fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }
            pub fn into_uuid(self) -> Uuid {
                self.0
            }
            pub fn as_uuid(&self) -> &Uuid {
                &self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl std::str::FromStr for $name {
            type Err = uuid::Error;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ok(Self(Uuid::parse_str(s)?))
            }
        }
    };
}

id_type!(
    AgencyId,
    "Tenant (Agency) identifier — RLS tenant context key."
);
id_type!(BrandId, "Brand workspace identifier.");
id_type!(DeviceId, "Registered execution device identifier.");
id_type!(TaskId, "Execution task identifier.");
id_type!(RequestId, "API request correlation identifier (TDD 12.1).");

/// Idempotency key for external side effects (TDD 14.2).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
    pub fn generate() -> Self {
        Self(Uuid::new_v4().to_string())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for IdempotencyKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn ids_roundtrip() {
        let id = AgencyId::new();
        let s = id.to_string();
        let parsed = AgencyId::from_str(&s).expect("parse");
        assert_eq!(id, parsed);
    }

    #[test]
    fn idempotency_key_unique() {
        assert_ne!(IdempotencyKey::generate(), IdempotencyKey::generate());
    }
}
