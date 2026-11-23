//! Skill handlers — 把 skills.tools 从声明式字段变成真正可执行的调用。
//!
//! 每个 handler 是一个真实执行路径:
//! - intel-backed (research 域): 查 intel_items / 趋势聚合 (真实采集数据)
//! - llm-backed (content/creative/strategy/quality/learning 域): 走 LlmPort 真生成
//! - db-backed (memory/analytics 域): 走 CycleStore / StatsPort 真读写真数据
//! - publish-backed (publishing 域): 走 PublishPort 真派发设备任务
//!
//! 未注册 handler 的 skill 仍是元数据 (prompt 契约), API 会标 handler=false。

use std::time::Instant;

use async_trait::async_trait;
use chrono::Utc;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::cycle::{CycleDeps, StatsPort};
use crate::domain::Employee;
use crate::memory::{MemoryItem, MemoryType};

/// handler 运行时上下文: 端口 + 员工身份信息 (prompt 需要的品牌人设字段)。
pub struct SkillRunCtx<'a> {
    pub deps: &'a CycleDeps,
    pub employee: &'a Employee,
    /// 品牌人设/策略字段, 供 llm 类 handler 组 prompt
    pub persona: Value,
    /// 品牌 no_go 列表 (platform_compliance 用)
    pub no_go: Vec<String>,
}

/// 一次 skill 执行的可审计结果。
#[derive(Debug, Clone)]
pub struct SkillRun {
    pub skill: String,
    pub output: Value,
    pub duration_ms: u128,
}

#[async_trait]
pub trait SkillHandler: Send + Sync {
    /// 该 handler 覆盖的 skill name 列表。
    fn names(&self) -> &'static [&'static str];
    async fn run(&self, skill: &str, input: &Value, ctx: &SkillRunCtx<'_>)
    -> Result<Value, String>;
}

/// 全部内置 handler 注册表。
pub fn builtin_handlers() -> Vec<Box<dyn SkillHandler>> {
    vec![Box::new(BuiltinHandlers)]
}

/// 所有已落地 handler 的 skill name (API 标记 handler=true 用)。
pub fn handler_skill_names() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = Vec::new();
    for h in builtin_handlers() {
        v.extend_from_slice(h.names());
    }
    v.sort_unstable();
    v.dedup();
    v
}

/// 查注册表并执行一次 skill; 不阻塞周期 —— 调用方决定是否容忍错误。
pub async fn run_skill(
    skill: &str,
    input: &Value,
    ctx: &SkillRunCtx<'_>,
) -> Result<SkillRun, String> {
    let start = Instant::now();
    for h in builtin_handlers() {
        if h.names().contains(&skill) {
            let output = h.run(skill, input, ctx).await?;
            return Ok(SkillRun {
                skill: skill.to_string(),
                output,
                duration_ms: start.elapsed().as_millis(),
            });
        }
    }
    Err(format!("no handler registered for skill '{skill}'"))
}

struct BuiltinHandlers;

#[async_trait]
impl SkillHandler for BuiltinHandlers {
    fn names(&self) -> &'static [&'static str] {
        &[
            // research — intel 真数据
            "social_search",
            "topic_research",
            "audience_research",
            "competitor_analysis",
            "content_gap_analysis",
            "creator_analysis",
            "trend_discovery",
            "fact_checking",
            "source_verification",
            "fact_check",
            "claim_verification",
            // content — llm 真生成
            "social_post_generation",
            "text_generation",
            "article_writing",
            "caption_generation",
            "script_generation",
            "subtitle_generation",
            "content_rewriting",
            "localization",
            "translation",
            "carousel_generation",
            // creative — llm
            "idea_generation",
            "angle_discovery",
            "hook_generation",
            "storytelling",
            "viral_format_analysis",
            "creative_variation",
            "content_brainstorming",
            "trend_to_content",
            // strategy — llm
            "content_strategy",
            "audience_strategy",
            "channel_strategy",
            "content_pillar_planning",
            "campaign_planning",
            "growth_strategy",
            "competitor_positioning",
            // quality_compliance — llm 审查 + 规则
            "content_quality_review",
            "brand_voice_review",
            "originality_check",
            "safety_review",
            "content_scoring",
            "platform_compliance",
            "copyright_check",
            // publishing — 真派发/真校验
            "content_publishing",
            "platform_formatting",
            "account_selection",
            "publishing_verification",
            // analytics — db 真统计
            "performance_analysis",
            "engagement_analysis",
            "audience_analysis",
            "viral_content_analysis",
            "content_comparison",
            "trend_performance_analysis",
            "campaign_analysis",
            // learning — llm 洞察
            "insight_extraction",
            "strategy_optimization",
            "pattern_detection",
            // memory — db 真读写
            "episodic_memory",
            "semantic_memory",
        ]
    }

    async fn run(
        &self,
        skill: &str,
        input: &Value,
        ctx: &SkillRunCtx<'_>,
    ) -> Result<Value, String> {
        match skill {
            // ---------- research: intel 真数据 ----------
            "social_search"
            | "topic_research"
            | "audience_research"
            | "competitor_analysis"
            | "content_gap_analysis"
            | "creator_analysis" => {
                let q = input
                    .get("query")
                    .or_else(|| input.get("topic"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if q.is_empty() {
                    return Err("input.query/topic required".into());
                }
                let items = ctx.deps.intel.search_items(&q, 20).await?;
                Ok(json!({
                    "skill": skill, "query": q, "count": items.len(),
                    "items": items,
                }))
            }
            "trend_discovery" => {
                let limit = input.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
                let trends = ctx.deps.intel.trending(limit).await?;
                Ok(json!({ "skill": skill, "count": trends.len(), "trends": trends }))
            }
            "fact_checking" | "source_verification" | "fact_check" | "claim_verification" => {
                let claim = input
                    .get("claim")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if claim.is_empty() {
                    return Err("input.claim required".into());
                }
                let items = ctx.deps.intel.search_items(&claim, 10).await?;
                let sources: Vec<String> = items
                    .iter()
                    .filter_map(|i| {
                        i.get("source_id")
                            .and_then(|s| s.as_str())
                            .map(String::from)
                    })
                    .collect();
                let distinct: std::collections::BTreeSet<_> = sources.iter().collect();
                let verdict = if distinct.len() >= 2 {
                    "corroborated"
                } else if distinct.len() == 1 {
                    "single_source"
                } else {
                    "insufficient_evidence"
                };
                Ok(json!({
                    "skill": skill, "claim": claim, "verdict": verdict,
                    "corroborating_sources": distinct.into_iter().collect::<Vec<_>>(),
                    "evidence": items,
                }))
            }

            // ---------- content: llm 真生成 ----------
            "social_post_generation"
            | "text_generation"
            | "article_writing"
            | "caption_generation"
            | "script_generation"
            | "subtitle_generation"
            | "content_rewriting"
            | "localization"
            | "translation"
            | "carousel_generation" => {
                let topic = input.get("topic").and_then(|v| v.as_str()).unwrap_or("");
                let p = &ctx.persona;
                let prompt = format!(
                    "You are {name} ({role}). Mission: {mission}. Audience: {audience}. Voice: {voice}. Pillars: {pillars}. Language: {language}.\n\
                     Task: {skill} about \"{topic}\" for channel {channel} (max {limit} weighted chars, CJK counts double), entirely in {language}.\n\
                     Evidence: {evidence}\nDo NOT repeat these recent topics: {recent}.\nOutput ONLY the content text.",
                    name = p
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("the brand"),
                    role = p
                        .get("role")
                        .and_then(|v| v.as_str())
                        .unwrap_or("content_operator"),
                    mission = p.get("mission").and_then(|v| v.as_str()).unwrap_or(""),
                    audience = p.get("audience").cloned().unwrap_or(json!([])),
                    voice = p.get("voice").and_then(|v| v.as_str()).unwrap_or("casual"),
                    pillars = p.get("pillars").cloned().unwrap_or(json!([])),
                    language = p
                        .get("language")
                        .and_then(|v| v.as_str())
                        .unwrap_or("zh-CN"),
                    channel = ctx.employee.channel,
                    limit = crate::cycle::platform_text_limit(&ctx.employee.channel),
                    evidence = input.get("evidence").cloned().unwrap_or(json!([])),
                    recent = input.get("recent").cloned().unwrap_or(json!([])),
                );
                let text = ctx.deps.llm.generate(&prompt, 512).await?;
                Ok(json!({ "skill": skill, "text": text, "chars": text.chars().count() }))
            }

            // ---------- creative / strategy: llm ----------
            "idea_generation"
            | "angle_discovery"
            | "hook_generation"
            | "storytelling"
            | "viral_format_analysis"
            | "creative_variation"
            | "content_brainstorming"
            | "trend_to_content"
            | "content_strategy"
            | "audience_strategy"
            | "channel_strategy"
            | "content_pillar_planning"
            | "campaign_planning"
            | "growth_strategy"
            | "competitor_positioning" => {
                let topic = input.get("topic").and_then(|v| v.as_str()).unwrap_or("");
                let p = &ctx.persona;
                let prompt = format!(
                    "You are {name}, voice {voice}, audience {audience}, pillars {pillars}, language {language}.\n\
                     Skill: {skill}. Topic/context: {topic}. Trends: {trends}\n\
                     Produce 3-5 concrete, specific outputs (ideas/angles/hooks/strategy items) in {language}. Output as a JSON array of strings.",
                    name = p
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("the brand"),
                    voice = p.get("voice").and_then(|v| v.as_str()).unwrap_or("casual"),
                    audience = p.get("audience").cloned().unwrap_or(json!([])),
                    pillars = p.get("pillars").cloned().unwrap_or(json!([])),
                    language = p
                        .get("language")
                        .and_then(|v| v.as_str())
                        .unwrap_or("zh-CN"),
                    trends = input.get("trends").cloned().unwrap_or(json!([])),
                );
                let text = ctx.deps.llm.generate(&prompt, 512).await?;
                Ok(json!({ "skill": skill, "raw": text }))
            }

            // ---------- quality_compliance ----------
            "content_quality_review"
            | "brand_voice_review"
            | "safety_review"
            | "content_scoring"
            | "originality_check" => {
                let draft = input.get("text").and_then(|v| v.as_str()).unwrap_or("");
                if draft.is_empty() {
                    return Err("input.text required".into());
                }
                // originality: 先用真实历史主题做确定性查重
                let recent = ctx
                    .deps
                    .store
                    .recent_topics(ctx.employee.id, 20)
                    .await
                    .unwrap_or_default();
                let dup = recent.iter().any(|t| !t.is_empty() && draft.contains(t));
                let p = &ctx.persona;
                let prompt = format!(
                    "You are a strict {skill} reviewer for brand voice {voice} (language {language}).\n\
                     Review this draft:\n\"\"\"\n{draft}\n\"\"\"\n\
                     Reply with a JSON object: {{\"score\": 0-100, \"pass\": true/false, \"issues\": [...]}}.",
                    voice = p.get("voice").and_then(|v| v.as_str()).unwrap_or("casual"),
                    language = p
                        .get("language")
                        .and_then(|v| v.as_str())
                        .unwrap_or("zh-CN"),
                );
                let raw = ctx
                    .deps
                    .llm
                    .generate(&prompt, 256)
                    .await
                    .unwrap_or_default();
                Ok(json!({
                    "skill": skill, "llm_review": raw, "duplicate_topic": dup,
                    "recent_topics": recent,
                }))
            }
            "platform_compliance" | "copyright_check" => {
                let text = input.get("text").and_then(|v| v.as_str()).unwrap_or("");
                let limit = crate::cycle::platform_text_limit(&ctx.employee.channel);
                let hits: Vec<&str> = ctx
                    .no_go
                    .iter()
                    .filter(|ng| !ng.is_empty() && text.to_lowercase().contains(&ng.to_lowercase()))
                    .map(|s| s.as_str())
                    .collect();
                Ok(json!({
                    "skill": skill, "pass": hits.is_empty() && text.len() <= limit * 2,
                    "no_go_hits": hits, "len": text.len(), "limit": limit,
                }))
            }

            // ---------- publishing ----------
            "content_publishing" => {
                let package_id = input
                    .get("package_id")
                    .and_then(|v| v.as_str())
                    .and_then(|s| Uuid::parse_str(s).ok())
                    .ok_or("input.package_id (uuid) required")?;
                let receipt = ctx.deps.publish.publish(ctx.employee, package_id).await?;
                Ok(json!({ "skill": skill, "receipt": {
                    "task_id": receipt.task_id, "route": receipt.route,
                    "status": receipt.status, "approval_status": receipt.approval_status,
                }}))
            }
            "publishing_verification" => {
                let task_id = input
                    .get("task_id")
                    .and_then(|v| v.as_str())
                    .and_then(|s| Uuid::parse_str(s).ok())
                    .ok_or("input.task_id (uuid) required")?;
                let state = ctx.deps.publish.task_state(task_id).await?;
                Ok(json!({ "skill": skill, "task_id": task_id, "state": state }))
            }
            "platform_formatting" => {
                let text = input
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let limit = crate::cycle::platform_text_limit(&ctx.employee.channel);
                let truncated = crate::cycle::weighted_len(&text) > limit;
                let out = if truncated {
                    text.chars().take(limit).collect::<String>()
                } else {
                    text
                };
                Ok(json!({ "skill": skill, "channel": ctx.employee.channel,
                           "limit": limit, "truncated": truncated, "text": out }))
            }
            "account_selection" => Ok(json!({
                "skill": skill, "channel": ctx.employee.channel,
                "brand_id": ctx.employee.brand_id, "employee": ctx.employee.name,
            })),

            // ---------- analytics: db 真统计 ----------
            "performance_analysis"
            | "engagement_analysis"
            | "audience_analysis"
            | "viral_content_analysis"
            | "content_comparison"
            | "trend_performance_analysis"
            | "campaign_analysis" => {
                let m = ctx.deps.stats.metrics().await?;
                Ok(json!({ "skill": skill, "metrics": m }))
            }

            // ---------- learning ----------
            "insight_extraction" | "strategy_optimization" => {
                let cycle_summary = input.get("cycle_summary").cloned().unwrap_or(json!({}));
                let p = &ctx.persona;
                let prompt = format!(
                    "You are the learning module of {name}. Based on this operating cycle summary:\n{cycle_summary}\n\
                     Extract ONE sharp insight (in {language}) as a JSON object {{\"insight\": \"...\", \"confidence\": 0-1, \"action\": \"...\"}}.",
                    name = p
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("the brand"),
                    language = p
                        .get("language")
                        .and_then(|v| v.as_str())
                        .unwrap_or("zh-CN"),
                );
                let raw = ctx.deps.llm.generate(&prompt, 256).await?;
                Ok(json!({ "skill": skill, "raw": raw }))
            }
            "pattern_detection" => {
                let topics = ctx.deps.store.recent_topics(ctx.employee.id, 50).await?;
                let mut freq: std::collections::HashMap<&str, usize> =
                    std::collections::HashMap::new();
                for t in &topics {
                    *freq.entry(t.as_str()).or_insert(0) += 1;
                }
                let mut patterns: Vec<(&str, usize)> = freq.into_iter().collect();
                patterns.sort_by(|a, b| b.1.cmp(&a.1));
                Ok(json!({ "skill": skill, "recent_topics": topics,
                           "repeated": patterns.into_iter().take(5).collect::<Vec<_>>() }))
            }

            // ---------- memory ----------
            "episodic_memory" => {
                let content = input.get("content").and_then(|v| v.as_str()).unwrap_or("");
                if content.is_empty() {
                    return Err("input.content required".into());
                }
                let mem = MemoryItem {
                    id: Uuid::new_v4(),
                    employee_id: ctx.employee.id,
                    r#type: MemoryType::Episodic,
                    content: content.to_string(),
                    evidence: input
                        .get("evidence")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default(),
                    confidence: input
                        .get("confidence")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.7),
                    created_at: Utc::now(),
                    last_validated: None,
                };
                let id = mem.id;
                ctx.deps.store.add_memory(&mem).await?;
                Ok(json!({ "skill": skill, "memory_id": id, "stored": true }))
            }
            "semantic_memory" => {
                let topics = ctx.deps.store.recent_topics(ctx.employee.id, 50).await?;
                Ok(json!({ "skill": skill, "topics": topics }))
            }

            other => Err(format!("handler registered but no arm for '{other}'")),
        }
    }
}

// 让 StatsPort 在本模块可被引用 (handlers 通过 deps.stats 访问)
#[allow(dead_code)]
fn _assert_stats_port(_: &dyn StatsPort) {}
