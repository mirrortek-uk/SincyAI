//! cloud-api entrypoint.
//!
//! Serves /api/v1 REST (TDD 12.1) with unified JSON errors, request_id and
//! structured tracing. Runs migrations on startup when a database is
//! configured. The Agent WSS gateway (/api/v1/agent/connect) lands in WP-08.

use axum::Router;
use cloud_api::AppState;
use std::net::SocketAddr;

#[tokio::main]
async fn main() {
    let env = std::env::var("SINCY_ENV").unwrap_or_else(|_| "local".to_string());
    let cfg = match sincy_config::load(&env) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("config error: {e}");
            std::process::exit(2);
        }
    };

    sincy_tracing_init(&cfg.log.filter, cfg.log.json);

    // database is optional in local mode (healthz/version still work)
    let pool = if cfg.database.url.is_empty() {
        tracing::warn!("SINCY_DATABASE_URL not set: running without database");
        None
    } else {
        match sincy_database::connect(&cfg.database.url, cfg.database.max_connections).await {
            Ok(db) => {
                tracing::info!("database connected, migrations applied");
                Some(db.pool().clone())
            }
            Err(e) => {
                tracing::error!(error = %e, "database connect/migrate failed");
                std::process::exit(3);
            }
        }
    };

    let state = AppState {
        pool,
        env: cfg.env.clone(),
    };

    let app = Router::new()
        .nest("/api/v1", cloud_api::routes())
        .with_state(state)
        .fallback(cloud_api::api::not_found);

    let addr = SocketAddr::from(([0, 0, 0, 0], cfg.http.port));
    tracing::info!(%addr, env = %cfg.env, "cloud-api listening");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|e| {
            tracing::error!(error = %e, "failed to bind");
            std::process::exit(1);
        });
    axum::serve(listener, app).await.unwrap_or_else(|e| {
        tracing::error!(error = %e, "server error");
        std::process::exit(1);
    });
}

fn sincy_tracing_init(filter: &str, json: bool) {
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    if json {
        builder.json().init();
    } else {
        builder.init();
    }
}
