//! Brand safety kill switch (FR-025): pause all scheduled/pending tasks for a
//! brand | client | agency | global scope. Recovery requires an explicit,
//! audited action. Acceptance: 1 分钟内生效 — the check below is a pure,
//! synchronous read used on every task dispatch path.

use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KillScope {
    Brand,
    Client,
    Agency,
    Global,
}

#[derive(Debug, Clone)]
pub struct ActiveKillSwitch {
    pub scope: KillScope,
    pub target_id: Option<Uuid>,
    pub reason: String,
}

pub trait KillSwitchStore: Send + Sync {
    /// All currently active kill switches (global + tenant-visible).
    fn active(&self) -> Vec<ActiveKillSwitch>;
}

/// Is a task for (agency_id, client_id, brand_id) paused right now?
pub fn is_paused(
    store: &dyn KillSwitchStore,
    agency_id: Uuid,
    client_id: Option<Uuid>,
    brand_id: Option<Uuid>,
) -> Option<String> {
    for ks in store.active() {
        let hit = match ks.scope {
            KillScope::Global => true,
            KillScope::Agency => ks.target_id == Some(agency_id),
            KillScope::Client => ks.target_id.is_some() && ks.target_id == client_id,
            KillScope::Brand => ks.target_id.is_some() && ks.target_id == brand_id,
        };
        if hit {
            return Some(ks.reason);
        }
    }
    None
}

/// In-memory store for tests; PG store reads kill_switches WHERE active.
#[derive(Default)]
pub struct MemoryKillSwitchStore {
    inner: Mutex<Vec<ActiveKillSwitch>>,
}

impl MemoryKillSwitchStore {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn add(&self, ks: ActiveKillSwitch) {
        self.inner.lock().expect("lock").push(ks);
    }
    pub fn clear(&self) {
        self.inner.lock().expect("lock").clear();
    }
}

impl KillSwitchStore for MemoryKillSwitchStore {
    fn active(&self) -> Vec<ActiveKillSwitch> {
        self.inner.lock().expect("lock").clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brand_kill_switch_pauses_only_that_brand() {
        let store = MemoryKillSwitchStore::new();
        let brand_a = Uuid::new_v4();
        let brand_b = Uuid::new_v4();
        store.add(ActiveKillSwitch {
            scope: KillScope::Brand,
            target_id: Some(brand_a),
            reason: "品牌风险事件".into(),
        });
        assert!(is_paused(&store, Uuid::new_v4(), None, Some(brand_a)).is_some());
        assert!(is_paused(&store, Uuid::new_v4(), None, Some(brand_b)).is_none());
    }

    #[test]
    fn global_kill_switch_pauses_everything() {
        let store = MemoryKillSwitchStore::new();
        store.add(ActiveKillSwitch {
            scope: KillScope::Global,
            target_id: None,
            reason: "全局熔断".into(),
        });
        assert!(
            is_paused(
                &store,
                Uuid::new_v4(),
                Some(Uuid::new_v4()),
                Some(Uuid::new_v4())
            )
            .is_some()
        );
        store.clear();
        assert!(is_paused(&store, Uuid::new_v4(), None, None).is_none());
    }

    #[test]
    fn agency_scope_matches_agency() {
        let store = MemoryKillSwitchStore::new();
        let agency = Uuid::new_v4();
        store.add(ActiveKillSwitch {
            scope: KillScope::Agency,
            target_id: Some(agency),
            reason: "租户违规".into(),
        });
        assert!(is_paused(&store, agency, None, Some(Uuid::new_v4())).is_some());
        assert!(is_paused(&store, Uuid::new_v4(), None, None).is_none());
    }
}
