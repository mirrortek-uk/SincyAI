//! WP-15 observability: in-process Prometheus-style counters + DB gauges on
//! GET /api/v1/metrics. Dashboards: routing, quota, adapter health, devices.
//!
//! Counters live in a process-global registry (standard Prometheus practice);
//! gauges are read from the DB per scrape.

use sqlx::PgPool;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
pub struct Metrics {
    pub http_requests_total: AtomicU64,
    pub tasks_published_total: AtomicU64,
    pub route_api_total: AtomicU64,
    pub route_device_total: AtomicU64,
    pub route_assisted_total: AtomicU64,
    pub route_degraded_total: AtomicU64,
    pub quota_reserved_total: AtomicU64,
    pub quota_exceeded_total: AtomicU64,
    pub kill_switch_toggles_total: AtomicU64,
    pub kill_switch_active: AtomicU64,
    pub agent_connections_total: AtomicU64,
    pub agent_disconnects_total: AtomicU64,
    pub agent_tasks_assigned_total: AtomicU64,
    pub agent_task_results_total: AtomicU64,
    pub agent_tasks_requeued_total: AtomicU64,
    pub ui_drift_events_total: AtomicU64,
    pub challenges_total: AtomicU64,
    pub budget_blocked_total: AtomicU64,
}

impl Metrics {
    pub fn inc(c: &AtomicU64) {
        c.fetch_add(1, Ordering::Relaxed);
    }
    fn get(c: &AtomicU64) -> u64 {
        c.load(Ordering::Relaxed)
    }
}

pub fn metrics() -> &'static Metrics {
    static M: OnceLock<Metrics> = OnceLock::new();
    M.get_or_init(Metrics::default)
}

/// Render the /metrics body (Prometheus text exposition format).
pub async fn render(pool: Option<&PgPool>) -> String {
    let m = metrics();
    let mut out = String::new();
    let mut counter = |name: &str, help: &str, c: &AtomicU64| {
        out.push_str(&format!(
            "# HELP {name} {help}\n# TYPE {name} counter\n{name} {}\n",
            Metrics::get(c)
        ));
    };
    counter(
        "sincy_http_requests_total",
        "HTTP requests served",
        &m.http_requests_total,
    );
    counter(
        "sincy_tasks_published_total",
        "tasks accepted via /tasks/publish",
        &m.tasks_published_total,
    );
    counter(
        "sincy_route_api_total",
        "routes resolved to API",
        &m.route_api_total,
    );
    counter(
        "sincy_route_device_total",
        "routes resolved to device",
        &m.route_device_total,
    );
    counter(
        "sincy_route_assisted_total",
        "routes degraded to assisted",
        &m.route_assisted_total,
    );
    counter(
        "sincy_route_degraded_total",
        "routes degraded (no side effect)",
        &m.route_degraded_total,
    );
    counter(
        "sincy_quota_reserved_total",
        "quota reservations",
        &m.quota_reserved_total,
    );
    counter(
        "sincy_quota_exceeded_total",
        "quota exceeded events",
        &m.quota_exceeded_total,
    );
    counter(
        "sincy_kill_switch_toggles_total",
        "kill switch changes",
        &m.kill_switch_toggles_total,
    );
    counter(
        "sincy_agent_connections_total",
        "agent websocket connections",
        &m.agent_connections_total,
    );
    counter(
        "sincy_agent_disconnects_total",
        "agent websocket disconnects",
        &m.agent_disconnects_total,
    );
    counter(
        "sincy_agent_tasks_assigned_total",
        "device tasks dispatched",
        &m.agent_tasks_assigned_total,
    );
    counter(
        "sincy_agent_task_results_total",
        "device task results received",
        &m.agent_task_results_total,
    );
    counter(
        "sincy_agent_tasks_requeued_total",
        "tasks requeued after device loss",
        &m.agent_tasks_requeued_total,
    );
    counter(
        "sincy_ui_drift_events_total",
        "UI drift events reported",
        &m.ui_drift_events_total,
    );
    counter(
        "sincy_challenges_total",
        "challenge screens (never auto-solved)",
        &m.challenges_total,
    );
    counter(
        "sincy_budget_blocked_total",
        "generation tasks blocked by budget",
        &m.budget_blocked_total,
    );
    out.push_str(&format!(
        "# HELP sincy_kill_switch_active 1 when any kill switch is active\n# TYPE sincy_kill_switch_active gauge\nsincy_kill_switch_active {}\n",
        Metrics::get(&m.kill_switch_active)
    ));

    if let Some(pool) = pool {
        // gauges via SECURITY DEFINER function (migration 0011): platform-wide
        // counts only, tenant RLS bypassed inside the function
        if let Ok(row) = sqlx::query("SELECT sincy_ops_gauges() AS g")
            .fetch_one(pool)
            .await
        {
            let g: serde_json::Value = row.get("g");
            out.push_str("# HELP sincy_device_tasks device tasks by state\n# TYPE sincy_device_tasks gauge\n");
            if let Some(series) = g["device_tasks"].as_array() {
                for item in series {
                    out.push_str(&format!(
                        "sincy_device_tasks{{state=\"{}\"}} {}\n",
                        item["state"].as_str().unwrap_or("?"),
                        item["n"].as_i64().unwrap_or(0)
                    ));
                }
            }
            let gauge = |name: &str, help: &str, v: &serde_json::Value| {
                format!(
                    "# HELP {name} {help}\n# TYPE {name} gauge\n{name} {}\n",
                    v.as_i64().unwrap_or(0)
                )
            };
            out.push_str(&gauge(
                "sincy_devices_online",
                "devices with heartbeat in last 120s",
                &g["devices_online"],
            ));
            out.push_str(&gauge(
                "sincy_adapters_degraded",
                "adapters currently degraded by drift",
                &g["adapters_degraded"],
            ));
            out.push_str(&gauge(
                "sincy_ui_drift_last_hour",
                "drift events in the last hour",
                &g["ui_drift_last_hour"],
            ));
        }
    }
    out
}

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sqlx::Row;

pub async fn metrics_handler(State(state): State<crate::state::AppState>) -> Response {
    Metrics::inc(&metrics().http_requests_total);
    let body = render(state.pool.as_ref()).await;
    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        body,
    )
        .into_response()
}
