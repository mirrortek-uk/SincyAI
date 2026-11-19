//! cloud-api entrypoint (WP-01 slice).
//!
//! Serves /api/v1 REST (TDD 12.1) with unified JSON errors, request_id and
//! structured tracing. The Agent WSS gateway (/api/v1/agent/connect) is
//! added in WP-08; routes are declared here so the surface is stable.

mod api;

use axum::Router;
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

    let app = Router::new()
        .nest("/api/v1", api::routes())
        .fallback(api::not_found);

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
