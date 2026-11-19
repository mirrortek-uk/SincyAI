//! agent-worker entrypoint (WP-01 slice).
//! WP-04 fills this with the Agent Runtime: planner, tools behind Policy Gate,
//! content versioning, quality/policy gates and AI labeling.

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

    tracing::info!(env = %cfg.env, "agent-worker started (WP-01 skeleton)");
    tokio::signal::ctrl_c()
        .await
        .expect("install ctrl-c handler");
    tracing::info!("agent-worker shutting down");
}
