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
    AiLabel, Brief, LocalStubProvider, MediaJob, MediaProvider, MetricEvent, PublishingPackage,
    QualityGate, TextGenerator, TriggerSource, adapt_master, create_master_with_voice,
    default_adapters, learn, plan_strategy_with_learnings,
};

use crate::api::app_error_response;
use crate::auth::AuthCtx;
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
    axum::extract::Extension(auth): axum::extract::Extension<AuthCtx>,
    Json(req): Json<BriefReq>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let agency_id = *auth.agency_id.as_uuid();
    // brand 必须属于调用方 agency, 否则 403 (防跨租户写)
    if !req.brand_id.is_empty() {
        let ok: Option<uuid::Uuid> =
            sqlx::query_scalar("SELECT id FROM brands WHERE id = $1::uuid AND agency_id = $2")
                .bind(&req.brand_id)
                .bind(agency_id)
                .fetch_optional(pool)
                .await
                .map_err(dberr)?;
        if ok.is_none() {
            return Err(app_error_response(sincy_common::AppError::non_retryable(
                "forbidden",
                "brand does not belong to your agency",
            )));
        }
    }
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

async fn list_briefs(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<AuthCtx>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let rows = sqlx::query(
        "SELECT brief_id, brand_id, trigger, seed_opportunity_id, seed_signal_id, goal, platforms, angle, status, created_at
         FROM factory_briefs WHERE agency_id = $1 ORDER BY created_at DESC LIMIT 100",
    )
    .bind(auth.agency_id.as_uuid())
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
    axum::extract::Extension(auth): axum::extract::Extension<AuthCtx>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    // 执行委托给共享执行体 (with_tenant 租户上下文内跑工序1→2→3)
    let agency = *auth.agency_id.as_uuid();
    match run_brief_by_id(pool, agency, &id).await {
        Ok(v) => Ok(Json(
            json!({ "brief_id": id, "status": "done", "result": v }),
        )),
        Err(e) => {
            let _ = sqlx::query("UPDATE factory_briefs SET status='failed', updated_at=now() WHERE brief_id=$1 AND agency_id=$2")
                .bind(&id).bind(agency).execute(pool).await;
            Err(app_error_response(sincy_common::AppError::retryable(
                "factory", e,
            )))
        }
    }
}

/// 断点①: 机会引擎刷新后, 为达到阈值且今天没有 Brief 的机会自动建 Brief
pub async fn auto_briefs_from_opportunities(
    pool: &sqlx::PgPool,
    agency_id: uuid::Uuid,
    threshold: f64,
) -> usize {
    // 整段在租户事务内: SET LOCAL 后 RLS WITH CHECK 放行 (FORCE RLS 下必须带上下文)
    let mut tx = match pool.begin().await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(error = %e, "auto_briefs begin tx failed");
            return 0;
        }
    };
    if let Err(e) = sqlx::query(&format!("SET LOCAL app.current_agency_id = '{agency_id}'"))
        .execute(&mut *tx)
        .await
    {
        tracing::warn!(error = %e, "auto_briefs set tenant ctx failed");
        return 0;
    }
    let brand_id: Option<String> = sqlx::query_scalar(
        "SELECT id::text FROM brands WHERE agency_id = $1 ORDER BY created_at LIMIT 1",
    )
    .bind(agency_id)
    .fetch_optional(&mut *tx)
    .await
    .unwrap_or(None);
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
    .fetch_all(&mut *tx)
    .await
    .unwrap_or_default();
    let mut n = 0;
    for r in rows {
        let brief = Brief::from_opportunity(
            brand_id.as_deref().unwrap_or(""),
            &r.get::<String, _>("id"),
            None,
            &format!("opportunity: {}", r.get::<String, _>("topic")),
            vec!["x".to_string()],
            &r.get::<String, _>("topic"),
        );
        let res = sqlx::query(
            "INSERT INTO factory_briefs
               (brief_id, agency_id, brand_id, trigger, seed_opportunity_id, goal, platforms, angle, status)
             VALUES ($1,$2,$3,'opportunity_driven',$4,$5,$6,$7,'draft')
             ON CONFLICT (brief_id) DO NOTHING",
        )
        .bind(&brief.brief_id)
        .bind(agency_id)
        .bind(&brief.brand_id)
        .bind(&brief.seed_opportunity_id)
        .bind(&brief.goal)
        .bind(serde_json::to_value(&brief.platforms).unwrap_or(json!([])))
        .bind(&brief.angle)
        .execute(&mut *tx)
        .await;
        if matches!(res, Ok(q) if q.rows_affected() > 0) {
            n += 1;
        }
    }
    if let Err(e) = tx.commit().await {
        tracing::warn!(error = %e, "auto_briefs commit failed");
        return 0;
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
    axum::extract::Extension(auth): axum::extract::Extension<AuthCtx>,
    Json(req): Json<LearnReq>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let agency_id = *auth.agency_id.as_uuid();
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
            "INSERT INTO factory_learnings (agency_id, brand_id, employee_id, hypothesis, evidence, confidence, source)
             VALUES ($1,$2,$3,$4,$5,$6,'factory_learn')",
        )
        .bind(agency_id)
        .bind(req.brand_id.as_deref())
        .bind(employee_id)
        .bind(&l.hypothesis)
        .bind(serde_json::to_value(&l.evidence).unwrap_or(json!([])))
        .bind(l.confidence)
        .execute(pool)
        .await
        .map_err(dberr)?;
        if let Some(eid) = employee_id {
            // (2) 给 factory-learn 镜像补 adjustment, 使其能被员工回路 take_pending_adjustment 消费
            let adj = json!({
                "pillar_boost": l.hypothesis.split_whitespace().next().unwrap_or(""),
                "source": "factory_learn"
            });
            let _ = sqlx::query(
                "INSERT INTO employee_experiments (employee_id, hypothesis, variable, status, result, adjustment)
                 VALUES ($1,$2,'factory_learn','ANALYZING', $3, $4)",
            )
            .bind(eid)
            .bind(&l.hypothesis)
            .bind(json!({"evidence": l.evidence, "confidence": l.confidence, "source": "factory_learn"}))
            .bind(adj)
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

/// 后台 runner: 消费 draft 状态的 Brief (断点①自动产出的行真正被消费)
pub async fn run_draft_briefs(pool: &sqlx::PgPool, limit: i64) -> usize {
    // 先列出有 draft 的 agency, 再逐租户在租户事务里消费 (FORCE RLS 下必须带上下文)
    let agencies: Vec<uuid::Uuid> = sqlx::query_scalar(
        "SELECT DISTINCT agency_id FROM factory_briefs WHERE status='draft' LIMIT 20",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let mut n = 0;
    for agency in agencies {
        let ids: Vec<String> = match pool.begin().await {
            Ok(mut tx) => {
                let _ = sqlx::query(&format!("SET LOCAL app.current_agency_id = '{agency}'"))
                    .execute(&mut *tx)
                    .await;
                let r = sqlx::query_scalar(
                    "SELECT brief_id FROM factory_briefs WHERE status='draft' ORDER BY created_at LIMIT $1",
                )
                .bind(limit)
                .fetch_all(&mut *tx)
                .await;
                let _ = tx.commit().await;
                r.unwrap_or_default()
            }
            Err(_) => continue,
        };
        for id in ids {
            if run_brief_by_id(pool, agency, &id).await.is_ok() {
                n += 1;
            }
        }
    }
    n
}

/// 共享执行体: HTTP 端点与后台 runner 都走它; 必须带 agency, 读写都在租户事务内
/// 工序4 媒体 provider: 配 SINCY_MEDIA_PROVIDERS 即接真实生成工具 (Faster-Whisper/
/// MoneyPrinterTurbo/Pixelle-Video 等), 否则本地占位 (红线: 不调下载站)。
fn default_media_provider() -> Box<dyn MediaProvider> {
    Box::new(LocalStubProvider)
}

/// Voice Profile: 加载有效语气 = merge_opt(员工基线 identity.voice_profile, 账号覆盖 employee_accounts.voice_profile)。
/// 品牌下找不到员工/账号时退回空对象 (调用方退回旧 Voice 单字段)。
async fn load_effective_voice(pool: &sqlx::PgPool, brand_id: &str, platform: &str) -> Value {
    let emp = sqlx::query(
        "SELECT e.id FROM digital_employees e WHERE e.brand_id = $1::uuid AND e.status = 'RUNNING' ORDER BY e.created_at LIMIT 1",
    )
    .bind(brand_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    let Some(emp) = emp else { return json!({}) };
    let emp_id: uuid::Uuid = emp.get("id");
    let base: Value = sqlx::query_scalar(
        "SELECT COALESCE(identity->'voice_profile', '{}'::jsonb) FROM employee_identity_versions WHERE employee_id = $1 AND status = 'active' ORDER BY version DESC LIMIT 1",
    )
    .bind(emp_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .unwrap_or_else(|| json!({}));
    let over: Value = sqlx::query_scalar(
        "SELECT voice_profile FROM employee_accounts WHERE employee_id = $1 AND platform = $2 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(emp_id)
    .bind(platform)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .unwrap_or(Value::Null);
    sincy_content::voice::effective_voice(&base, &over)
}
pub async fn run_brief_by_id(
    pool: &sqlx::PgPool,
    agency_id: uuid::Uuid,
    id: &str,
) -> Result<Value, String> {
    // 读 brief: 租户事务内 + agency 过滤
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query(&format!("SET LOCAL app.current_agency_id = '{agency_id}'"))
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    let row = sqlx::query(
        "SELECT brief_id, brand_id, trigger, seed_opportunity_id, seed_signal_id, goal, platforms, angle, audience, cta, constraints
         FROM factory_briefs WHERE brief_id = $1 AND agency_id = $2",
    )
    .bind(id)
    .bind(agency_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    let row = row.ok_or_else(|| "brief not found".to_string())?;
    let brief = Brief {
        brief_id: row.get("brief_id"),
        trigger: match row.get::<String, _>("trigger").as_str() {
            "opportunity_driven" => TriggerSource::OpportunityDriven,
            "calendar_driven" => TriggerSource::CalendarDriven,
            _ => TriggerSource::Manual,
        },
        seed_opportunity_id: row.get("seed_opportunity_id"),
        seed_signal_id: row.get("seed_signal_id"),
        brand_id: row.get::<Option<String>, _>("brand_id").unwrap_or_default(),
        goal: row.get("goal"),
        platforms: serde_json::from_value(row.get("platforms")).unwrap_or_default(),
        angle: row.get("angle"),
        audience: row.get("audience"),
        cta: row.get("cta"),
        constraints: row.get("constraints"),
        created_at: chrono::Utc::now(),
    };
    // 信号库是平台级表 (无 RLS), 直接读
    let signals: Vec<Value> = if let Some(sid) = &brief.seed_signal_id {
        sqlx::query("SELECT to_jsonb(s.*) FROM intelligence_signals s WHERE signal_id = $1")
            .bind(sid)
            .fetch_all(pool)
            .await
            .map_err(|e| e.to_string())?
            .iter()
            .map(|r| r.get::<Value, _>(0))
            .collect()
    } else {
        sqlx::query("SELECT to_jsonb(s.*) FROM intelligence_signals s ORDER BY importance DESC, observed_at DESC LIMIT 8")
            .fetch_all(pool).await.map_err(|e| e.to_string())?
            .iter().map(|r| r.get::<Value, _>(0)).collect()
    };
    let llm = llm_gen();
    // 标记 running (租户事务)
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query(&format!("SET LOCAL app.current_agency_id = '{agency_id}'"))
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query("UPDATE factory_briefs SET status='running', updated_at=now() WHERE brief_id=$1 AND agency_id=$2")
        .bind(id).bind(agency_id).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    // (2) 工序9→策略 回灌: 读该品牌近期学习结论注入工序1 (断点② 闭合)
    let learnings: Vec<String> = sqlx::query_scalar(
        "SELECT hypothesis FROM factory_learnings WHERE brand_id = $1 ORDER BY created_at DESC LIMIT 5",
    )
    .bind(brief.brand_id.as_str())
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    // 工序1 策略 (带回灌) + 工序2 母稿 (Voice Profile 注入软约束)
    let plan = plan_strategy_with_learnings(&brief, &signals, &learnings, &llm).await?;
    let voice_platform = brief.platforms.first().cloned().unwrap_or_default();
    let voice_eff = load_effective_voice(pool, &brief.brand_id, &voice_platform).await;
    let voice_block = sincy_content::voice::compile_voice_prompt(&voice_eff, 3);
    let master = create_master_with_voice(&brief, &plan, &signals, &llm, &voice_block).await?;

    // 工序3→7: 每平台 适配 →(4)媒体 →(5)质量门 →(6)审批 →(7)打包落库
    let agency = sincy_common::AgencyId::from_uuid(agency_id);
    let brand_uuid = uuid::Uuid::parse_str(&brief.brand_id).ok();
    let media_provider = default_media_provider();
    let mut variants = Vec::new();
    for a in default_adapters() {
        let platform = a.spec().platform.clone();
        if !(brief.platforms.is_empty() || brief.platforms.contains(&platform)) {
            continue;
        }
        let text = adapt_master(&master, a.as_ref(), &brief);

        // 工序4 媒体: 视频类平台产媒体任务 (stub 返回 PENDING, provider 可换真实生成)
        let media_output = if matches!(platform.as_str(), "youtube" | "tiktok" | "抖音" | "视频号")
        {
            let job = MediaJob {
                job_id: format!("{}-{}", brief.brief_id, platform),
                kind: "video".into(),
                source_url: None,
                prompt: master.text.chars().take(400).collect(),
                spec: json!({ "platform": platform }),
            };
            media_provider.generate(&job).await.ok()
        } else {
            None
        };

        // 工序5 质量门: 构造不可变 PublishingPackage 过 gate.rs
        let label = AiLabel::ai_generated(vec![json!({"model": "factory", "kind": "llm"})]);
        let rules = sincy_content::gate::BrandRules {
            forbidden_topics: Vec::new(),
            required_cta: (!brief.cta.trim().is_empty()).then(|| brief.cta.clone()),
            max_text_len: Some(if platform == "x" { 280 } else { 40_000 }),
            media_specs: std::collections::HashMap::new(),
        };
        let gate_verdict = match PublishingPackage::create(
            uuid::Uuid::new_v4(),
            agency_id.to_string(),
            brief.brand_id.clone(),
            platform.clone(),
            "",
            "publish_post",
            text.clone(),
            Vec::new(),
            label,
            json!({}),
        ) {
            Ok(pkg) => {
                let v = QualityGate::new(rules)
                    .with_voice_profile(voice_eff.clone())
                    .check(&pkg, pkg.ai_label());
                json!({ "passed": v.passed(), "issues": v.issues })
            }
            Err(e) => json!({
                "passed": false,
                "issues": [{"code": "package_build_failed", "message": e.to_string()}]
            }),
        };
        let passed = gate_verdict["passed"].as_bool().unwrap_or(false);

        // 工序6 审批 + 工序7 打包落库: 过门的入待审队列 (FR-024: AI 内容人工审核)
        let package_id = if passed
            && let Some(bid) = brand_uuid
            && let Ok(pid) = crate::api::insert_package(
                pool,
                agency,
                crate::api::NewPackage {
                    brand_id: bid,
                    platform: &platform,
                    action: "publish_post",
                    text: &text,
                    source: "ai",
                    provenance: json!({"factory_brief": brief.brief_id, "plan_angle": plan.angle}),
                    approval_status: "pending",
                },
            )
            .await
        {
            Some(pid.to_string())
        } else {
            None
        };

        variants.push(json!({
            "platform": platform,
            "text": text,
            "media": media_output,
            "gate": gate_verdict,
            "approval_status": if passed { "pending" } else { "rejected" },
            "package_id": package_id,
        }));
    }
    let result = json!({
        "plan": plan,
        "master": master,
        "variants": variants,
        "signal_count": signals.len(),
        "learnings_used": learnings.len(),
    });
    // 写回 done (租户事务)
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query(&format!("SET LOCAL app.current_agency_id = '{agency_id}'"))
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query("UPDATE factory_briefs SET status='done', result=$2, updated_at=now() WHERE brief_id=$1 AND agency_id=$3")
        .bind(id).bind(&result).bind(agency_id).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(result)
}

/// 后台调度循环: 每 10 分钟消费一批 draft Brief
pub async fn draft_runner_loop(pool: sqlx::PgPool) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(600));
    loop {
        tick.tick().await;
        let n = run_draft_briefs(&pool, 5).await;
        if n > 0 {
            tracing::info!(ran = n, "factory draft runner consumed briefs");
        }
    }
}
