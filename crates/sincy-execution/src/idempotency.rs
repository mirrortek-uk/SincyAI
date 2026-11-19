//! Idempotency for external side effects (TDD 14): (agency_id, idempotency_key)
//! is unique; a repeated delivery returns the original record instead of
//! re-executing (at-least-once delivery + consumer dedup).

use sincy_common::{AgencyId, IdempotencyKey};
use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

pub struct IdempotencyRecord {
    pub agency_id: AgencyId,
    pub key: IdempotencyKey,
    pub task_id: Option<Uuid>,
}

pub trait IdempotencyStore: Send + Sync {
    /// Returns Some(existing) when the key was already seen, else inserts and
    /// returns None. Implementations must be atomic (DB unique constraint or lock).
    fn register(
        &self,
        agency_id: AgencyId,
        key: &IdempotencyKey,
        task_id: Option<Uuid>,
    ) -> Option<IdempotencyRecord>;
}

/// In-memory store for unit tests; PG-backed store uses UNIQUE(agency_id, idempotency_key).
pub struct MemoryIdempotencyStore {
    inner: Mutex<HashMap<(AgencyId, String), IdempotencyRecord>>,
}

impl Default for MemoryIdempotencyStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryIdempotencyStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }
}

impl IdempotencyStore for MemoryIdempotencyStore {
    fn register(
        &self,
        agency_id: AgencyId,
        key: &IdempotencyKey,
        task_id: Option<Uuid>,
    ) -> Option<IdempotencyRecord> {
        let mut map = self.inner.lock().expect("idempotency lock poisoned");
        let k = (agency_id, key.as_str().to_string());
        if let Some(existing) = map.get(&k) {
            return Some(IdempotencyRecord {
                agency_id: existing.agency_id,
                key: IdempotencyKey::new(existing.key.as_str()),
                task_id: existing.task_id,
            });
        }
        map.insert(
            k,
            IdempotencyRecord {
                agency_id,
                key: IdempotencyKey::new(key.as_str()),
                task_id,
            },
        );
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_key_returns_original() {
        let store = MemoryIdempotencyStore::new();
        let agency = AgencyId::new();
        let key = IdempotencyKey::generate();
        let t1 = Uuid::new_v4();
        assert!(store.register(agency, &key, Some(t1)).is_none());
        let again = store.register(agency, &key, Some(Uuid::new_v4()));
        assert_eq!(again.expect("existing").task_id, Some(t1));
    }

    #[test]
    fn different_agencies_do_not_share_keys() {
        let store = MemoryIdempotencyStore::new();
        let key = IdempotencyKey::generate();
        assert!(store.register(AgencyId::new(), &key, None).is_none());
        assert!(store.register(AgencyId::new(), &key, None).is_none());
    }
}
