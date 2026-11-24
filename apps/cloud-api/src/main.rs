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

    // World Intelligence 采集 (WI §9): 唯一抓取方是 manifest 驱动的 Connector
    // 调度器; Sincy Pulse 的 aggregator provider 走自己的 manifest 调度。
    // Disable with SINCY_INTEL_COLLECT_ENABLED=false.
    if let Some(pool) = pool.clone()
        && std::env::var("SINCY_INTEL_COLLECT_ENABLED")
            .map(|v| v != "false")
            .unwrap_or(true)
    {
        // Sincy Pulse: 种子内置 provider + manifest 驱动调度
        cloud_api::pulse::seed_providers(&pool).await;
        tokio::spawn(cloud_api::pulse::run_scheduler(pool.clone()));
        // Connector 调度器 (Sincy-05): manifest 驱动抓取, 唯一采集入口, 产出 RawRecord 并镜像 intel_items
        tokio::spawn(cloud_api::pipeline::connectors::run_scheduler(pool.clone()));
        // 内容工厂后台 runner: 消费 draft Brief (断点①自动行真正被消费)
        tokio::spawn(cloud_api::factory::draft_runner_loop(pool.clone()));
        // 粉丝数测量回流 (patch-A): 周期派发 read_followers + harvest 设备回执写样本
        tokio::spawn(cloud_api::account_metrics::account_metrics_loop(
            pool.clone(),
        ));
        // 互动子循环: read_mentions 抓提及 → LLM 回复 → reply_comment 设备任务
        tokio::spawn(cloud_api::interaction::interaction_loop(
            pool.clone(),
            state.allow_mock_adapters,
        ));
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
        // 延迟测量后台 drain (与员工调度并行): 到期读 device_tasks 真实终态回写
        if let Some(pool) = pool.clone() {
            tokio::spawn(cloud_api::employees::measurement_drain_loop(pool));
        }
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
