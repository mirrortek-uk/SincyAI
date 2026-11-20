//! cloud-api library surface (so integration tests can drive the router).

pub mod api;
pub mod state;

pub use api::routes;
pub use state::AppState;

#[doc(hidden)]
pub mod state_for_test {
    pub use crate::state::AppState;
    pub fn test_state(pool: Option<sqlx::PgPool>) -> AppState {
        AppState {
            pool,
            env: "test".into(),
        }
    }
}

#[doc(hidden)]
pub fn routes_for_test() -> axum::Router<AppState> {
    routes()
}
