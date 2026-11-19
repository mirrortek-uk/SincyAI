//! cloud-scheduler entrypoint (WP-01 slice).
//! WP-09 turns this into the task orchestration loop: queue, assign, lease,
//! ACK, progress, result, timeout recovery with idempotency + Outbox.

use sincy_common::ErrorKind;

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
    tracing_subscriber::fmt()
        .with_env_filter(&cfg.log.filter)
        .init();

    tracing::info!(env = %cfg.env, nats = %cfg.worker.nats_url, "cloud-scheduler started (WP-01 skeleton)");
    tracing::debug!(kind = ?ErrorKind::NoSideEffect, "error taxonomy available via sincy-common");

    // WP-09: subscribe JetStream, drive task state machine.
    tokio::signal::ctrl_c()
        .await
        .expect("install ctrl-c handler");
    tracing::info!("cloud-scheduler shutting down");
}
