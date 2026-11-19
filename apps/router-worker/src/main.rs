//! router-worker entrypoint (WP-01 slice).
//! WP-05 implements the routing algorithm (TDD 7.2): candidate paths from the
//! capability matrix, tenant/account overrides, precondition checks, priority
//! pick, execution_routes audit row, and no-side-effect-only degradation.

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

    tracing::info!(
        env = %cfg.env,
        protocol_schema = sincy_protocol::PROTOCOL_SCHEMA_VERSION,
        "router-worker started (WP-01 skeleton; routing algorithm in WP-05)"
    );
    tokio::signal::ctrl_c()
        .await
        .expect("install ctrl-c handler");
    tracing::info!("router-worker shutting down");
}
