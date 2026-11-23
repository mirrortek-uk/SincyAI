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

    let state = AppState::from_config(pool.clone(), &cfg);

    // 首管理员引导: 仅由环境变量提供, 幂等, 密码只存 Argon2id 哈希
    if let Some(pool) = pool.clone()
        && let Err(e) = cloud_api::bootstrap::bootstrap_admin(&pool, &env).await
    {
        tracing::error!(error = %e, "bootstrap admin failed");
        std::process::exit(4);
    }

    let app = Router::new()
        .nest("/api/v1", cloud_api::routes())
        .with_state(state.clone())
        .fallback(cloud_api::api::not_found);

    // World Intelligence collector (WI §9): background loop over registered
    // read-only public sources. Disable with SINCY_INTEL_COLLECT_ENABLED=false.
    if let Some(pool) = pool.clone()
        && std::env::var("SINCY_INTEL_COLLECT_ENABLED")
            .map(|v| v != "false")
            .unwrap_or(true)
    {
        let cfg = cloud_api::intel_collector::CollectorConfig::from_env();
        tracing::info!(
            interval_secs = cfg.interval_secs,
            feeds = cfg.feeds.len(),
            "intel collector starting"
        );
        tokio::spawn(cloud_api::intel_collector::run_loop(pool.clone(), cfg));
        // Sincy Pulse: 种子内置 provider + manifest 驱动调度
        cloud_api::pulse::seed_providers(&pool).await;
        tokio::spawn(cloud_api::pulse::run_scheduler(pool.clone()));
    }

    // Digital Employee scheduler (DE §cadence): auto-runs operating cycles for
    // RUNNING employees per strategy.cadence_per_week. Disable with
    // SINCY_EMPLOYEE_SCHEDULER_ENABLED=false.
    if std::env::var("SINCY_EMPLOYEE_SCHEDULER_ENABLED")
        .map(|v| v != "false")
        .unwrap_or(true)
    {
        tracing::info!("employee scheduler starting");
        cloud_api::employees::spawn_scheduler(state.clone());
    }

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
