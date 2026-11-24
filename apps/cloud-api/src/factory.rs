//! 内容工厂 API (全链路架构 工序0/3/9 的云端入口, 有状态、落库):
//! - POST /factory/briefs        三种触发入口 → 工序0 Brief 并落库 factory_briefs
//! - GET  /factory/briefs        Brief 列表
//! - POST /factory/briefs/{id}/run  跑工序1策略 → 工序2母稿(grounding+事实核查) → 工序3平台适配
//! - GET  /factory/adapters      工序3 平台适配器清单 (每平台一个, 可插拔)
//! - POST /factory/learn         工序9 指标回流 → 学习结论真写回 factory_learnings + employee_experiments (断点②)
//! - 断点①: 机会引擎刷新后自动为 ≥阈值的机会建 Brief (auto_briefs_from_opportunities)

use axum::{
    Json, Router,
    extract::{Path, State},
    response::Response,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::Row;

use sincy_content::{
    Brief, MasterAsset, MetricEvent, StrategyPlan, TextGenerator, TriggerSource, adapt_master,
    create_master, default_adapters, learn, plan_strategy,
};

use crate::api::app_error_response;
use crate::state::AppState;

fn dberr(e: sqlx::Error) -> Response {
    app_error_response(sincy_common::AppError::retryable("db", e.to_string()))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/factory/briefs", post(create_brief).get(list_briefs))
        .route("/factory/briefs/{id}/run", post(run_brief))
        .route("/factory/adapters", get(list_adapters))
        .route("/factory/learn", post(run_learn))
}

/// 把 LlmPort 适配成工厂的 TextGenerator 端口
struct LlmGen(crate::employees::PgLlmPort);

#[async_trait::async_trait]
impl TextGenerator for LlmGen {
    async fn generate(&self, prompt: &str, max_tokens: u32) -> Result<String, String> {
        sincy_employee::cycle::LlmPort::generate(&self.0, prompt, max_tokens).await
    }
}

fn llm_gen() -> LlmGen {
    let allow_mock = std::env::var("SINCY_ENV").as_deref() != Ok("production");
    LlmGen(crate::employees::PgLlmPort::new(allow_mock))
}

#[derive(Debug, Deserialize)]
struct BriefReq {
    trigger: TriggerSource,
    brand_id: String,
    opportunity_id: Option<String>,
    signal_id: Option<String>,
    topic: Option<String>,
    goal: Option<String>,
    platforms: Vec<String>,
    angle: Option<String>,
    audience: Option<String>,
    cta: Option<String>,
}

async fn create_brief(
    State(state): State<AppState>,
    Json(req): Json<BriefReq>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let agency_id: Option<uuid::Uuid> =
        sqlx::query_scalar("SELECT id FROM agencies ORDER BY created_at LIMIT 1")
            .fetch_optional(pool)
            .await
            .map_err(dberr)?;
    let brief = match req.trigger {
        TriggerSource::OpportunityDriven => {
            let opp = req.opportunity_id.clone().ok_or_else(|| {
                app_error_response(sincy_common::AppError::non_retryable(
                    "bad_request",
                    "opportunity_id required",
                ))
            })?;
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
            let mut b = Brief::manual(
                &req.brand_id,
                req.topic.as_deref().unwrap_or(""),
                req.platforms.clone(),
            );
            b.trigger = TriggerSource::CalendarDriven;
            b.goal = req.goal.clone().unwrap_or_default();
            b.audience = req.audience.clone().unwrap_or_default();
            b.cta = req.cta.clone().unwrap_or_default();
            b
        }
        TriggerSource::Manual => {
            let mut b = Brief::manual(
                &req.brand_id,
                req.topic.as_deref().unwrap_or(""),
                req.platforms.clone(),
            );
            b.goal = req.goal.clone().unwrap_or_default();
            b.audience = req.audience.clone().unwrap_or_default();
            b.cta = req.cta.clone().unwrap_or_default();
            b
        }
    };
    sqlx::query(
        "INSERT INTO factory_briefs
           (brief_id, agency_id, brand_id, trigger, seed_opportunity_id, seed_signal_id,
            goal, platforms, angle, audience, cta, constraints, status)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,'draft')
         ON CONFLICT (brief_id) DO NOTHING",
    )
    .bind(&brief.brief_id)
    .bind(agency_id)
    .bind(&brief.brand_id)
    .bind(match brief.trigger {
        TriggerSource::OpportunityDriven => "opportunity_driven",
        TriggerSource::CalendarDriven => "calendar_driven",
        TriggerSource::Manual => "manual",
    })
    .bind(&brief.seed_opportunity_id)
    .bind(&brief.seed_signal_id)
    .bind(&brief.goal)
    .bind(serde_json::to_value(&brief.platforms).unwrap_or(json!([])))
    .bind(&brief.angle)
    .bind(&brief.audience)
    .bind(&brief.cta)
    .bind(&brief.constraints)
    .execute(pool)
    .await
    .map_err(dberr)?;
    Ok(Json(json!(brief)))
}

async fn list_briefs(State(state): State<AppState>) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let rows = sqlx::query(
        "SELECT brief_id, brand_id, trigger, seed_opportunity_id, seed_signal_id, goal, platforms, angle, status, created_at
         FROM factory_briefs ORDER BY created_at DESC LIMIT 100",
    )
    .fetch_all(pool)
    .await
    .map_err(dberr)?;
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "brief_id": r.get::<String, _>("brief_id"),
                "brand_id": r.get::<String, _>("brand_id"),
                "trigger": r.get::<String, _>("trigger"),
                "seed_opportunity_id": r.get::<Option<String>, _>("seed_opportunity_id"),
                "seed_signal_id": r.get::<Option<String>, _>("seed_signal_id"),
                "goal": r.get::<String, _>("goal"),
                "platforms": r.get::<Value, _>("platforms"),
                "angle": r.get::<String, _>("angle"),
                "status": r.get::<String, _>("status"),
                "created_at": r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
            })
        })
        .collect();
    Ok(Json(json!({ "briefs": items, "total": items.len() })))
}

/// 跑一个 Brief 的工序1→2→3, 结果写回 factory_briefs.result
async fn run_brief(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let row = sqlx::query(
        "SELECT brief_id, brand_id, trigger, seed_opportunity_id, seed_signal_id, goal, platforms, angle, audience, cta, constraints
         FROM factory_briefs WHERE brief_id = $1",
    )
    .bind(&id)
    .fetch_optional(pool)
    .await
    .map_err(dberr)?
    .ok_or_else(|| app_error_response(sincy_common::AppError::non_retryable("not_found", "brief not found")))?;
    let brief = Brief {
        brief_id: row.get("brief_id"),
        trigger: match row.get::<String, _>("trigger").as_str() {
            "opportunity_driven" => TriggerSource::OpportunityDriven,
            "calendar_driven" => TriggerSource::CalendarDriven,
            _ => TriggerSource::Manual,
        },
        seed_opportunity_id: row.get("seed_opportunity_id"),
        seed_signal_id: row.get("seed_signal_id"),
        brand_id: row.get("brand_id"),
        goal: row.get("goal"),
        platforms: serde_json::from_value(row.get("platforms")).unwrap_or_default(),
        angle: row.get("angle"),
        audience: row.get("audience"),
        cta: row.get("cta"),
        constraints: row.get("constraints"),
        created_at: chrono::Utc::now(),
    };
    // 从信号库取证据 (三入口的起点信号优先, 否则取最近高重要度信号)
    let signals: Vec<Value> = if let Some(sid) = &brief.seed_signal_id {
        sqlx::query("SELECT to_jsonb(s.*) FROM intelligence_signals s WHERE signal_id = $1")
            .bind(sid)
            .fetch_all(pool)
            .await
            .map_err(dberr)?
            .iter()
            .map(|r| r.get::<Value, _>(0))
            .collect()
    } else {
        sqlx::query("SELECT to_jsonb(s.*) FROM intelligence_signals s ORDER BY importance DESC, observed_at DESC LIMIT 8")
            .fetch_all(pool)
            .await
            .map_err(dberr)?
            .iter()
            .map(|r| r.get::<Value, _>(0))
            .collect()
    };
    let llm = llm_gen();
    sqlx::query("UPDATE factory_briefs SET status='running', updated_at=now() WHERE brief_id=$1")
        .bind(&id)
        .execute(pool)
        .await
        .map_err(dberr)?;
    let result = async {
        let plan: StrategyPlan = plan_strategy(&brief, &signals, &llm).await?;
        let master: MasterAsset = create_master(&brief, &plan, &signals, &llm).await?;
        let mut variants = Vec::new();
        for a in default_adapters() {
            if brief.platforms.is_empty() || brief.platforms.contains(&a.spec().platform) {
                variants.push(json!({
                    "platform": a.spec().platform,
                    "text": adapt_master(&master, a.as_ref(), &brief),
                }));
            }
        }
        Ok::<Value, String>(json!({
            "plan": plan, "master": master, "variants": variants, "signal_count": signals.len(),
        }))
    }
    .await;
    match result {
        Ok(v) => {
            sqlx::query("UPDATE factory_briefs SET status='done', result=$2, updated_at=now() WHERE brief_id=$1")
                .bind(&id).bind(&v).execute(pool).await.map_err(dberr)?;
            Ok(Json(
                json!({ "brief_id": id, "status": "done", "result": v }),
            ))
        }
        Err(e) => {
            sqlx::query(
                "UPDATE factory_briefs SET status='failed', updated_at=now() WHERE brief_id=$1",
            )
            .bind(&id)
            .execute(pool)
            .await
            .map_err(dberr)?;
            Err(app_error_response(sincy_common::AppError::retryable(
                "factory", e,
            )))
        }
    }
}

/// 断点①: 机会引擎刷新后, 为达到阈值且今天没有 Brief 的机会自动建 Brief
pub async fn auto_briefs_from_opportunities(pool: &sqlx::PgPool, threshold: f64) -> usize {
    let rows = sqlx::query(
        "SELECT o.id::text, o.topic, o.score
         FROM intel_opportunities o
         WHERE o.score >= $1
           AND NOT EXISTS (
             SELECT 1 FROM factory_briefs b
             WHERE b.seed_opportunity_id = o.id::text AND b.created_at > now() - interval '24 hours')
         ORDER BY o.score DESC LIMIT 10",
    )
    .bind(threshold)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let mut n = 0;
    for r in rows {
        let brief = Brief::from_opportunity(
            "",
            &r.get::<String, _>("id"),
            None,
            &format!("opportunity: {}", r.get::<String, _>("topic")),
            vec!["x".to_string()],
            &r.get::<String, _>("topic"),
        );
        let res = sqlx::query(
            "INSERT INTO factory_briefs
               (brief_id, brand_id, trigger, seed_opportunity_id, goal, platforms, angle, status)
             VALUES ($1,$2,'opportunity_driven',$3,$4,$5,$6,'draft')
             ON CONFLICT (brief_id) DO NOTHING",
        )
        .bind(&brief.brief_id)
        .bind(&brief.brand_id)
        .bind(&brief.seed_opportunity_id)
        .bind(&brief.goal)
        .bind(serde_json::to_value(&brief.platforms).unwrap_or(json!([])))
        .bind(&brief.angle)
        .execute(pool)
        .await;
        if matches!(res, Ok(q) if q.rows_affected() > 0) {
            n += 1;
        }
    }
    n
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
    brand_id: Option<String>,
    events: Vec<MetricEvent>,
}

/// 工序9 Learn: 结论真写回 factory_learnings, 并镜像进 employee_experiments (断点②闭合)
async fn run_learn(
    State(state): State<AppState>,
    Json(req): Json<LearnReq>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let learnings = learn(&req.events);
    let employee_id: Option<uuid::Uuid> = match &req.brand_id {
        Some(b) => sqlx::query_scalar(
            "SELECT id FROM digital_employees WHERE brand_id = $1::uuid ORDER BY created_at LIMIT 1",
        )
        .bind(b)
        .fetch_optional(pool)
        .await
        .map_err(dberr)?,
        None => None,
    };
    for l in &learnings {
        sqlx::query(
            "INSERT INTO factory_learnings (brand_id, employee_id, hypothesis, evidence, confidence, source)
             VALUES ($1,$2,$3,$4,$5,'factory_learn')",
        )
        .bind(req.brand_id.as_deref())
        .bind(employee_id)
        .bind(&l.hypothesis)
        .bind(serde_json::to_value(&l.evidence).unwrap_or(json!([])))
        .bind(l.confidence)
        .execute(pool)
        .await
        .map_err(dberr)?;
        if let Some(eid) = employee_id {
            let _ = sqlx::query(
                "INSERT INTO employee_experiments (employee_id, hypothesis, variable, status, result)
                 VALUES ($1,$2,'factory_learn','ANALYZING', $3)",
            )
            .bind(eid)
            .bind(&l.hypothesis)
            .bind(json!({"evidence": l.evidence, "confidence": l.confidence, "source": "factory_learn"}))
            .execute(pool)
            .await;
        }
    }
    Ok(Json(json!({
        "learnings": learnings,
        "count": learnings.len(),
        "written_to": ["factory_learnings", if employee_id.is_some() { "employee_experiments" } else { "" }],
    })))
}
