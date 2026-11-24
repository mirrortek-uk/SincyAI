//! cloud-api library surface (so integration tests can drive the router).

pub mod account_metrics;
pub mod api;
pub mod architecture;
pub mod auth;
pub mod bootstrap;
pub mod employees;
pub mod factory;
pub mod feed_import;
pub mod focus;
pub mod gateway;
pub mod industry;
pub mod intel;
pub mod intel_collector;
pub mod interaction;
pub mod kpi;
pub mod metrics;
pub mod pipeline;
pub mod pulse;
pub mod skills;
pub mod source_status;
pub mod state;
pub mod voice;

pub use api::routes;
pub use state::AppState;

#[doc(hidden)]
pub mod state_for_test {
    pub use crate::state::AppState;
    pub fn test_state(pool: Option<sqlx::PgPool>) -> AppState {
        let mut st = AppState::new(pool, "test".into());
        // tests run against mock adapters by explicit flag (never in prod)
        st.allow_mock_adapters = true;
        st
    }
}

#[doc(hidden)]
pub fn routes_for_test() -> axum::Router<AppState> {
    routes()
}
