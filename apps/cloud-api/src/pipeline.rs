//! Intelligence Pipeline 云端侧 (Sincy-05 落地):
//! - 持久化四层存储 (raw_records / entities / trend_series+ranking_snapshots / signals)
//! - collector 与 pulse 的产出都先写 RawRecord, 再经 PipelineEngine 按 record_type 路由处理
//! - 读 API: /intel/raw-records /intel/entities /intel/signals /intel/pipelines
//!
//! 现有 intel_items 保持不变 (legacy), 双写并行。

use axum::{
    Json, Router,
    extract::{Query, State},
    response::Response,
    routing::get,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::Row;
use std::sync::OnceLock;

use sincy_intel::pipeline::{
    PipelineEngine, ProcessingContext, RawRecord, RecordType, default_engine,
};

use crate::api::app_error_response;
use crate::state::AppState;

static ENGINE: OnceLock<PipelineEngine> = OnceLock::new();

pub fn engine() -> &'static PipelineEngine {
    ENGINE.get_or_init(default_engine)
}

/// 旧 feed kind → RecordType 映射 (形状已有: 不改管道)
pub fn record_type_for_feed_kind(kind: &str) -> RecordType {
    match kind {
        "news" | "blog" => RecordType::Article,
        "trend" | "hot" => RecordType::Trend,
        "rank" => RecordType::Ranking,
        "social_content" => RecordType::SocialPost,
        "event" => RecordType::Event,
        "research" => RecordType::ResearchPaper,
        "video" => RecordType::Video,
        "product" => RecordType::Product,
        _ => RecordType::Article,
    }
}

/// pulse entity_type → RecordType
pub fn record_type_for_pulse(entity_type: &str) -> RecordType {
    match entity_type {
        "social_post" => RecordType::SocialPost,
        "social_account" => RecordType::SocialAccount,
        "ranking_snapshot" => RecordType::Ranking,
        "trend" => RecordType::Trend,
        "account_health" => RecordType::Metric,
        "news" => RecordType::Article,
        _ => RecordType::Unknown,
    }
}

/// 把一条 RawRecord 入库并跑 pipeline (幂等: dedup_key 冲突则跳过处理)。返回是否处理。
pub async fn ingest_record(pool: &sqlx::PgPool, record: &RawRecord) -> Result<bool, String> {
    let inserted = sqlx::query_scalar::<_, i64>(
        "INSERT INTO intelligence_raw_records
           (provider_id, source_id, dataset_id, record_type, external_id, url, dedup_key, payload, metadata, fetched_at, observed_at)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
         ON CONFLICT (dedup_key) DO NOTHING RETURNING 1::bigint",
    )
    .bind(&record.provider_id)
    .bind(&record.source_id)
    .bind(&record.dataset_id)
    .bind(record.record_type.as_str())
    .bind(&record.external_id)
    .bind(&record.url)
    .bind(&record.dedup_key)
    .bind(&record.payload)
    .bind(serde_json::to_value(&record.metadata).unwrap_or(json!({})))
    .bind(record.fetched_at)
    .bind(record.observed_at)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    if inserted.is_none() {
        return Ok(false); // 幂等: 已处理过
    }

    // 构建处理上下文: 历史 trend 值 + 历史 rank
    let mut ctx = ProcessingContext::default();
    let hist_rows = sqlx::query(
        "SELECT series_key, value FROM trend_series WHERE series_key LIKE $1 ORDER BY observed_at DESC",
    )
    .bind(format!("{}:%", record.dataset_id))
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    for r in hist_rows {
        let k: String = r.get("series_key");
        let v: f64 = r.get("value");
        ctx.history.entry(k).or_insert(json!(v));
    }
    let rank_rows = sqlx::query(
        "SELECT dataset_id, external_id, title, rank FROM ranking_snapshots WHERE dataset_id = $1 ORDER BY observed_at DESC LIMIT 500",
    )
    .bind(&record.dataset_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    for r in rank_rows {
        let key = format!(
            "{}:{}:rank",
            r.get::<String, _>("dataset_id"),
            r.get::<Option<String>, _>("external_id")
                .unwrap_or_else(|| r.get::<String, _>("title"))
        );
        let v: Option<f64> = r.get("rank");
        if let Some(v) = v {
            ctx.history.entry(key).or_insert(json!(v));
        }
    }

    let result = match engine().ingest(record, &ctx).await {
        Ok(r) => r,
        Err(e) => {
            let _ = sqlx::query(
                "UPDATE intelligence_raw_records SET processing_status='failed' WHERE dedup_key = $1",
            )
            .bind(&record.dedup_key)
            .execute(pool)
            .await;
            return Err(e);
        }
    };

    // 持久化实体 (upsert)
    for e in &result.entities {
        sqlx::query(
            "INSERT INTO intelligence_entities
               (entity_id, entity_type, source_id, dataset_id, external_id, url, title, payload,
                industry_tags, topic_tags, entities, geography, language, provenance,
                confidence, quality_score, relevance_score, observed_at, updated_at)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18, now())
             ON CONFLICT (entity_id) DO UPDATE SET
               title = EXCLUDED.title, payload = EXCLUDED.payload,
               industry_tags = EXCLUDED.industry_tags, topic_tags = EXCLUDED.topic_tags,
               language = EXCLUDED.language, quality_score = EXCLUDED.quality_score,
               relevance_score = EXCLUDED.relevance_score, confidence = EXCLUDED.confidence,
               updated_at = now()",
        )
        .bind(&e.entity_id)
        .bind(&e.entity_type)
        .bind(&e.source_id)
        .bind(&e.dataset_id)
        .bind(&e.external_id)
        .bind(&e.url)
        .bind(&e.title)
        .bind(&e.payload)
        .bind(serde_json::to_value(&e.industry_tags).unwrap_or(json!([])))
        .bind(serde_json::to_value(&e.topic_tags).unwrap_or(json!([])))
        .bind(serde_json::to_value(&e.entities).unwrap_or(json!([])))
        .bind(serde_json::to_value(&e.geography).unwrap_or(json!([])))
        .bind(&e.language)
        .bind(&e.provenance)
        .bind(e.confidence)
        .bind(e.quality_score)
        .bind(e.relevance_score)
        .bind(e.observed_at)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    }
    // 持久化信号
    for s in &result.signals {
        let ind = s
            .payload
            .get("industry")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        sqlx::query(
            "INSERT INTO intelligence_signals
               (signal_id, signal_type, entity_id, source_id, industry_id, topic_id, strength, confidence, importance, observed_at, payload)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
             ON CONFLICT (signal_id) DO NOTHING",
        )
        .bind(&s.signal_id)
        .bind(s.signal_type.as_str())
        .bind(&s.entity_id)
        .bind(&s.source_id)
        .bind(ind)
        .bind(&s.topic_id)
        .bind(s.strength)
        .bind(s.confidence)
        .bind(s.importance)
        .bind(s.observed_at)
        .bind(&s.payload)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    }
    // side effects: trend_series / ranking_snapshots
    for se in &result.side_effects {
        match se.get("kind").and_then(|v| v.as_str()) {
            Some("trend_series_upsert") => {
                sqlx::query(
                    "INSERT INTO trend_series (series_key, value, source_id, observed_at) VALUES ($1,$2,$3,$4)",
                )
                .bind(se["key"].as_str().unwrap_or(""))
                .bind(se["value"].as_f64().unwrap_or(0.0))
                .bind(se["source_id"].as_str().unwrap_or(&record.source_id))
                .bind(chrono::Utc::now())
                .execute(pool)
                .await
                .map_err(|e| e.to_string())?;
            }
            Some("ranking_snapshot") => {
                sqlx::query(
                    "INSERT INTO ranking_snapshots (dataset_id, external_id, title, rank, metrics, observed_at) VALUES ($1,$2,$3,$4,$5,$6)",
                )
                .bind(se["dataset_id"].as_str().unwrap_or(""))
                .bind(se.get("external_id").and_then(|v| v.as_str()))
                .bind(se["title"].as_str().unwrap_or(""))
                .bind(se.get("rank").and_then(|v| v.as_f64()))
                .bind(&se["metrics"])
                .bind(chrono::Utc::now())
                .execute(pool)
                .await
                .map_err(|e| e.to_string())?;
            }
            _ => {}
        }
    }
    sqlx::query(
        "UPDATE intelligence_raw_records SET processing_status='processed' WHERE dedup_key = $1",
    )
    .bind(&record.dedup_key)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(true)
}

// ---------- 读 API ----------

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/intel/raw-records", get(list_raw_records))
        .route("/intel/entities", get(list_entities))
        .route("/intel/signals", get(list_signals))
        .route("/intel/pipelines", get(list_pipelines))
}

fn dberr(e: sqlx::Error) -> Response {
    app_error_response(sincy_common::AppError::retryable("db", e.to_string()))
}

#[derive(Debug, Deserialize)]
struct PageQuery {
    limit: Option<i64>,
    #[serde(rename = "type")]
    type_: Option<String>,
}

async fn list_raw_records(
    State(state): State<AppState>,
    Query(q): Query<PageQuery>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let rows: Vec<sqlx::postgres::PgRow> = match &q.type_ {
        Some(t) => sqlx::query(
            "SELECT id, provider_id, source_id, dataset_id, record_type, external_id, url, dedup_key, payload, fetched_at, processing_status FROM intelligence_raw_records WHERE record_type = $1 ORDER BY fetched_at DESC LIMIT $2",
        )
        .bind(t)
        .bind(limit)
        .fetch_all(pool)
        .await
        .map_err(dberr)?,
        None => sqlx::query(
            "SELECT id, provider_id, source_id, dataset_id, record_type, external_id, url, dedup_key, payload, fetched_at, processing_status FROM intelligence_raw_records ORDER BY fetched_at DESC LIMIT $1",
        )
        .bind(limit)
        .fetch_all(pool)
        .await
        .map_err(dberr)?,
    };
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<uuid::Uuid, _>("id"),
                "provider_id": r.get::<String, _>("provider_id"),
                "source_id": r.get::<String, _>("source_id"),
                "dataset_id": r.get::<String, _>("dataset_id"),
                "record_type": r.get::<String, _>("record_type"),
                "external_id": r.get::<Option<String>, _>("external_id"),
                "url": r.get::<Option<String>, _>("url"),
                "dedup_key": r.get::<String, _>("dedup_key"),
                "payload": r.get::<Value, _>("payload"),
                "fetched_at": r.get::<chrono::DateTime<chrono::Utc>, _>("fetched_at"),
                "processing_status": r.get::<String, _>("processing_status"),
            })
        })
        .collect();
    Ok(Json(json!({ "items": items })))
}

async fn list_entities(
    State(state): State<AppState>,
    Query(q): Query<PageQuery>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let rows: Vec<sqlx::postgres::PgRow> = match &q.type_ {
        Some(t) => sqlx::query(
            "SELECT entity_id, entity_type, source_id, dataset_id, url, title, industry_tags, topic_tags, language, quality_score, relevance_score, confidence, updated_at FROM intelligence_entities WHERE entity_type = $1 ORDER BY updated_at DESC LIMIT $2",
        )
        .bind(t)
        .bind(limit)
        .fetch_all(pool)
        .await
        .map_err(dberr)?,
        None => sqlx::query(
            "SELECT entity_id, entity_type, source_id, dataset_id, url, title, industry_tags, topic_tags, language, quality_score, relevance_score, confidence, updated_at FROM intelligence_entities ORDER BY updated_at DESC LIMIT $1",
        )
        .bind(limit)
        .fetch_all(pool)
        .await
        .map_err(dberr)?,
    };
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "entity_id": r.get::<String, _>("entity_id"),
                "entity_type": r.get::<String, _>("entity_type"),
                "source_id": r.get::<String, _>("source_id"),
                "dataset_id": r.get::<String, _>("dataset_id"),
                "url": r.get::<Option<String>, _>("url"),
                "title": r.get::<String, _>("title"),
                "industry_tags": r.get::<Value, _>("industry_tags"),
                "topic_tags": r.get::<Value, _>("topic_tags"),
                "language": r.get::<String, _>("language"),
                "quality_score": r.get::<f64, _>("quality_score"),
                "relevance_score": r.get::<f64, _>("relevance_score"),
                "confidence": r.get::<f64, _>("confidence"),
                "updated_at": r.get::<chrono::DateTime<chrono::Utc>, _>("updated_at"),
            })
        })
        .collect();
    Ok(Json(json!({ "items": items })))
}

#[derive(Debug, Deserialize)]
struct SignalQuery {
    limit: Option<i64>,
    #[serde(rename = "type")]
    type_: Option<String>,
}

async fn list_signals(
    State(state): State<AppState>,
    Query(q): Query<SignalQuery>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let rows: Vec<sqlx::postgres::PgRow> = match &q.type_ {
        Some(t) => sqlx::query(
            "SELECT signal_id, signal_type, entity_id, source_id, industry_id, strength, confidence, importance, observed_at, payload FROM intelligence_signals WHERE signal_type = $1 ORDER BY importance DESC LIMIT $2",
        )
        .bind(t)
        .bind(limit)
        .fetch_all(pool)
        .await
        .map_err(dberr)?,
        None => sqlx::query(
            "SELECT signal_id, signal_type, entity_id, source_id, industry_id, strength, confidence, importance, observed_at, payload FROM intelligence_signals ORDER BY importance DESC, observed_at DESC LIMIT $1",
        )
        .bind(limit)
        .fetch_all(pool)
        .await
        .map_err(dberr)?,
    };
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "signal_id": r.get::<String, _>("signal_id"),
                "signal_type": r.get::<String, _>("signal_type"),
                "entity_id": r.get::<Option<String>, _>("entity_id"),
                "source_id": r.get::<String, _>("source_id"),
                "industry_id": r.get::<Option<String>, _>("industry_id"),
                "strength": r.get::<f64, _>("strength"),
                "confidence": r.get::<f64, _>("confidence"),
                "importance": r.get::<f64, _>("importance"),
                "observed_at": r.get::<chrono::DateTime<chrono::Utc>, _>("observed_at"),
                "payload": r.get::<Value, _>("payload"),
            })
        })
        .collect();
    Ok(Json(json!({ "items": items })))
}

async fn list_pipelines(State(_state): State<AppState>) -> Result<Json<Value>, Response> {
    let eng = engine();
    let pipelines: Vec<Value> = eng
        .pipelines
        .list()
        .iter()
        .map(|p| {
            json!({
                "id": p.id,
                "version": p.version,
                "input_record_type": p.input_record_type.as_str(),
                "processors": p.processor_ids,
                "output_entity_types": p.output_entity_types,
                "enabled": p.enabled,
                "bus_subject": PipelineEngine::subject_for(p.input_record_type),
            })
        })
        .collect();
    let processors: Vec<Value> = eng
        .processors
        .list()
        .into_iter()
        .map(|(id, ver, rts)| {
            json!({
                "processor_id": id,
                "version": ver,
                "input_types": rts.iter().map(|r| r.as_str()).collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(Json(
        json!({ "pipelines": pipelines, "processors": processors }),
    ))
}
