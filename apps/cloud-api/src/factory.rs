//! 内容工厂 API (全链路架构 工序0/3/9 的云端入口):
//! - POST /factory/briefs  三种触发入口 → 工序0 Brief (断点①: 机会驱动在此接通)
//! - GET  /factory/adapters 工序3 平台适配器清单 (每平台一个, 可插拔)
//! - POST /factory/learn   工序9 指标回流 → 学习结论 (断点②)

use axum::{
    Json, Router,
    extract::State,
    response::Response,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};

use sincy_content::{
    Brief, MetricEvent, TriggerSource, default_adapters, learn,
};

use crate::api::app_error_response;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/factory/briefs", post(create_brief))
        .route("/factory/adapters", get(list_adapters))
        .route("/factory/learn", post(run_learn))
}

#[derive(Debug, Deserialize)]
struct BriefReq {
    trigger: TriggerSource,
    brand_id: String,
    /// ①机会驱动
    opportunity_id: Option<String>,
    signal_id: Option<String>,
    /// ③人工驱动
    topic: Option<String>,
    goal: Option<String>,
    platforms: Vec<String>,
    angle: Option<String>,
    audience: Option<String>,
    cta: Option<String>,
}

async fn create_brief(
    State(_state): State<AppState>,
    Json(req): Json<BriefReq>,
) -> Result<Json<Value>, Response> {
    let brief = match req.trigger {
        TriggerSource::OpportunityDriven => {
            let opp = req
                .opportunity_id
                .clone()
                .ok_or_else(|| app_error_response(sincy_common::AppError::non_retryable("bad_request", "opportunity_id required")))?;
            let mut b = Brief::from_opportunity(
                &req.brand_id,
                &opp,
                req.signal_id.as_deref(),
                req.goal.as_deref().unwrap_or(""),
                req.platforms.clone(),
                req.angle.as_deref().unwrap_or(""),
            );
            b.audience = req.audience.clone().unwrap_or_default();
            b.cta = req.cta.clone().unwrap_or_default();
            b
        }
        TriggerSource::CalendarDriven => {
            let mut b = Brief::manual(&req.brand_id, req.topic.as_deref().unwrap_or(""), req.platforms.clone());
            b.trigger = TriggerSource::CalendarDriven;
            b.goal = req.goal.clone().unwrap_or_default();
            b.audience = req.audience.clone().unwrap_or_default();
            b.cta = req.cta.clone().unwrap_or_default();
            b
        }
        TriggerSource::Manual => {
            let mut b = Brief::manual(&req.brand_id, req.topic.as_deref().unwrap_or(""), req.platforms.clone());
            b.goal = req.goal.clone().unwrap_or_default();
            b.audience = req.audience.clone().unwrap_or_default();
            b.cta = req.cta.clone().unwrap_or_default();
            b
        }
    };
    Ok(Json(json!(brief)))
}

async fn list_adapters(State(_state): State<AppState>) -> Result<Json<Value>, Response> {
    let items: Vec<Value> = default_adapters()
        .iter()
        .map(|a| {
            let s = a.spec();
            json!({
                "platform": s.platform,
                "max_chars": s.max_chars,
                "thread_limit": s.thread_limit,
                "tone": s.tone,
                "cta_style": s.cta_style,
                "hashtag_style": s.hashtag_style,
                "localization": s.localization,
                "ai_label_required": s.ai_label_required,
                "media_specs": s.media_specs,
            })
        })
        .collect();
    Ok(Json(json!({ "adapters": items, "total": items.len() })))
}

#[derive(Debug, Deserialize)]
struct LearnReq {
    events: Vec<MetricEvent>,
}

async fn run_learn(
    State(_state): State<AppState>,
    Json(req): Json<LearnReq>,
) -> Result<Json<Value>, Response> {
    let learnings = learn(&req.events);
    Ok(Json(json!({
        "learnings": learnings,
        "count": learnings.len(),
        "note": "回写策略/品牌记忆 (断点②): learnings 可写入 employee_experiments / 品牌记忆",
    })))
}
