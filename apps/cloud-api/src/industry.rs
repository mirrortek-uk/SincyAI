//! Sincy-04 行业情报与客户相关性 API (设计书 §13):
//!   GET  /industries                      行业树
//!   POST /industries                      Admin 新增自定义行业
//!   GET  /brands/{id}/intel-profile       读取客户情报画像
//!   PUT  /brands/{id}/intel-profile       保存客户情报画像
//!   GET  /brands/{id}/intel-feed          按相关性重排的情报 feed (Relevance Engine)
//! 所有 brand 路由都走 AuthCtx + RLS (agency_id 从 token 推导)。

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::Response,
    routing::get,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::Row;

use crate::api::app_error_response;
use crate::auth::AuthCtx;
use crate::state::AppState;

fn dberr(e: sqlx::Error) -> Response {
    app_error_response(sincy_common::AppError::retryable("db", e.to_string()))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/industries", get(list_industries).post(create_industry))
        .route(
            "/brands/{id}/intel-profile",
            get(get_profile).put(put_profile),
        )
        .route("/brands/{id}/intel-feed", get(intel_feed))
}

/// GET /industries — 行业树 (DB 种子 + 用户自定义), 前端画像编辑器与行业过滤器的数据源
async fn list_industries(State(state): State<AppState>) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let rows: Vec<sqlx::postgres::PgRow> = sqlx::query(
        "SELECT id, name, parent_id, level, keywords, description FROM industry_taxonomy ORDER BY level, id",
    )
    .fetch_all(pool)
    .await
    .map_err(dberr)?;
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<String, _>("id"),
                "name": r.get::<String, _>("name"),
                "parent_id": r.get::<Option<String>, _>("parent_id"),
                "level": r.get::<i32, _>("level"),
                "keywords": r.get::<Value, _>("keywords"),
                "description": r.get::<String, _>("description"),
            })
        })
        .collect();
    Ok(Json(json!({ "items": items })))
}

#[derive(Debug, Deserialize)]
struct NewIndustry {
    id: String,
    name: String,
    #[serde(default)]
    parent_id: Option<String>,
    #[serde(default = "default_level")]
    level: i32,
    #[serde(default)]
    keywords: Vec<String>,
    #[serde(default)]
    description: String,
}

fn default_level() -> i32 {
    3
}

/// POST /industries — 新增自定义行业 (level=3), 供客户画像使用
async fn create_industry(
    State(state): State<AppState>,
    axum::extract::Extension(_auth): axum::extract::Extension<AuthCtx>,
    Json(body): Json<NewIndustry>,
) -> Result<(StatusCode, Json<Value>), Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let id = body.id.trim().to_lowercase().replace(' ', "_");
    if id.is_empty() || id.len() > 64 {
        return Err(app_error_response(sincy_common::AppError::non_retryable(
            "bad_industry",
            "id 需为 1-64 字符 slug",
        )));
    }
    sqlx::query(
        "INSERT INTO industry_taxonomy (id, name, parent_id, level, keywords, description)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name, keywords = EXCLUDED.keywords",
    )
    .bind(&id)
    .bind(&body.name)
    .bind(&body.parent_id)
    .bind(body.level)
    .bind(serde_json::to_value(&body.keywords).unwrap_or(json!([])))
    .bind(&body.description)
    .execute(pool)
    .await
    .map_err(dberr)?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

/// 校验 brand 属于当前 agency (RLS 之外再显式校验, 防御性编程)
async fn brand_belongs_to_agency(
    pool: &sqlx::PgPool,
    auth: &AuthCtx,
    brand_id: &str,
) -> Result<uuid::Uuid, Response> {
    let bid = uuid::Uuid::parse_str(brand_id).map_err(|_| {
        app_error_response(sincy_common::AppError::non_retryable(
            "bad_id",
            "bad brand id",
        ))
    })?;
    let ok: Option<bool> =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM brands WHERE id = $1 AND agency_id = $2)")
            .bind(bid)
            .bind(auth.agency_id.as_uuid())
            .fetch_one(pool)
            .await
            .map_err(dberr)?;
    if ok != Some(true) {
        return Err(app_error_response(sincy_common::AppError::non_retryable(
            "not_found",
            "brand 不存在或不属于当前机构",
        )));
    }
    Ok(bid)
}

/// GET /brands/{id}/intel-profile
async fn get_profile(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<AuthCtx>,
    Path(id): Path<String>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let bid = brand_belongs_to_agency(pool, &auth, &id).await?;
    let profile: Value =
        sqlx::query_scalar("SELECT profile FROM brand_intel_profiles WHERE brand_id = $1")
            .bind(bid)
            .fetch_optional(pool)
            .await
            .map_err(dberr)?
            .unwrap_or(json!({}));
    Ok(Json(json!({ "brand_id": bid, "profile": profile })))
}

/// PUT /brands/{id}/intel-profile — 保存 Customer Intelligence Profile (Sincy-04 §9)
async fn put_profile(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<AuthCtx>,
    Path(id): Path<String>,
    Json(profile): Json<Value>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let bid = brand_belongs_to_agency(pool, &auth, &id).await?;
    // 校验能解析成 CustomerProfile
    let parsed = sincy_intel::industry::CustomerProfile::from_json(&profile);
    let v = serde_json::to_value(&parsed).map_err(|e| {
        app_error_response(sincy_common::AppError::non_retryable(
            "bad_profile",
            e.to_string(),
        ))
    })?;
    sqlx::query(
        "INSERT INTO brand_intel_profiles (brand_id, profile, updated_at)
         VALUES ($1, $2, now())
         ON CONFLICT (brand_id) DO UPDATE SET profile = EXCLUDED.profile, updated_at = now()",
    )
    .bind(bid)
    .bind(&v)
    .execute(pool)
    .await
    .map_err(dberr)?;
    Ok(Json(json!({ "brand_id": bid, "profile": v })))
}

#[derive(Debug, Deserialize)]
struct FeedQuery {
    limit: Option<i64>,
    #[serde(default)]
    min_score: Option<f64>,
}

/// GET /brands/{id}/intel-feed?limit=50&min_score=40 — 按客户画像相关性重排的情报 feed。
/// v1 规则评分 (Sincy-04 §10.2): Industry 30% + Topic 25% + Competitor 20% + Audience 15% + Geography 10%
async fn intel_feed(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<AuthCtx>,
    Path(id): Path<String>,
    Query(q): Query<FeedQuery>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let bid = brand_belongs_to_agency(pool, &auth, &id).await?;
    let profile: Value =
        sqlx::query_scalar("SELECT profile FROM brand_intel_profiles WHERE brand_id = $1")
            .bind(bid)
            .fetch_optional(pool)
            .await
            .map_err(dberr)?
            .unwrap_or(json!({}));
    let cp = sincy_intel::industry::CustomerProfile::from_json(&profile);
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let rows: Vec<sqlx::postgres::PgRow> = sqlx::query(
        "SELECT id, source_id, kind, dedup_key, payload, collected_at, industries
           FROM intel_items ORDER BY collected_at DESC LIMIT $1",
    )
    .bind(limit * 4) // 多取一些再在内存里按相关性重排
    .fetch_all(pool)
    .await
    .map_err(dberr)?;
    let mut scored: Vec<(f64, Value)> = Vec::new();
    for r in rows {
        let payload: Value = r.get("payload");
        let industries: Value = r.get("industries");
        let inds: Vec<String> = serde_json::from_value(industries.clone()).unwrap_or_default();
        let text = format!(
            "{} {}",
            payload.get("title").and_then(|v| v.as_str()).unwrap_or(""),
            payload
                .get("summary")
                .and_then(|v| v.as_str())
                .unwrap_or("")
        );
        let score = sincy_intel::industry::relevance_score(&inds, &text, &cp);
        if q.min_score.map(|m| score >= m).unwrap_or(true) {
            scored.push((
                score,
                json!({
                    "id": r.get::<uuid::Uuid, _>("id"),
                    "source_id": r.get::<String, _>("source_id"),
                    "kind": r.get::<String, _>("kind"),
                    "dedup_key": r.get::<String, _>("dedup_key"),
                    "payload": payload,
                    "collected_at": r.get::<chrono::DateTime<chrono::Utc>, _>("collected_at"),
                    "industries": industries,
                    "relevance_score": (score * 10.0).round() / 10.0,
                }),
            ));
        }
    }
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let items: Vec<Value> = scored
        .iter()
        .take(limit as usize)
        .map(|(_, v)| v.clone())
        .collect();
    // per-brand 相关性回填: 实体层不再存恒 0 的 relevance_score, 按品牌落关联表 (migration 0041)
    {
        let mut tx = pool.begin().await.map_err(dberr)?;
        sqlx::query(&format!(
            "SET LOCAL app.current_agency_id = '{}'",
            auth.agency_id.as_uuid()
        ))
        .execute(&mut *tx)
        .await
        .map_err(dberr)?;
        for (score, v) in scored.iter().take(limit as usize) {
            let dk = v.get("dedup_key").and_then(|x| x.as_str()).unwrap_or("");
            if dk.is_empty() {
                continue;
            }
            sqlx::query(
                "INSERT INTO intel_entity_brand_relevance (dedup_key, brand_id, agency_id, score, updated_at)
                 VALUES ($1, $2, $3, $4, now())
                 ON CONFLICT (dedup_key, brand_id) DO UPDATE SET score = EXCLUDED.score, updated_at = now()",
            )
            .bind(dk)
            .bind(bid)
            .bind(auth.agency_id.as_uuid())
            .bind(*score)
            .execute(&mut *tx)
            .await
            .map_err(dberr)?;
        }
        tx.commit().await.map_err(dberr)?;
    }
    Ok(Json(json!({
        "brand_id": bid,
        "profile_industries": cp.industries(),
        "count": items.len(),
        "items": items,
    })))
}
