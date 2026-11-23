//! Operating cycle orchestration (DE §5.2). The cycle is resumable: every step
//! is persisted through CycleStore with an idempotency key before it runs.
//!
//! Loop: Observe → Understand → Decide → Plan → Create → Review → Execute →
//! Measure → Learn → Adapt (DE core operating loop).

use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sincy_intel::adapter::EmployeeContext;
use sincy_intel::model::Opportunity;
use uuid::Uuid;

use crate::agents::AgentKind;
use crate::domain::{Employee, Identity, Mission, Strategy};
use crate::handlers::{SkillRunCtx, run_skill};
use crate::memory::{MemoryItem, MemoryType};
use crate::skills::{Skill, pick_content_skill, skills_for_agent};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CycleStep {
    Observe,
    Understand,
    Decide,
    Plan,
    Create,
    Review,
    Execute,
    Measure,
    Learn,
    Adapt,
}

impl CycleStep {
    pub fn as_str(&self) -> &'static str {
        match self {
            CycleStep::Observe => "observe",
            CycleStep::Understand => "understand",
            CycleStep::Decide => "decide",
            CycleStep::Plan => "plan",
            CycleStep::Create => "create",
            CycleStep::Review => "review",
            CycleStep::Execute => "execute",
            CycleStep::Measure => "measure",
            CycleStep::Learn => "learn",
            CycleStep::Adapt => "adapt",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishReceipt {
    pub task_id: Uuid,
    pub route: String,
    pub status: String,
    pub package_id: Uuid,
    pub approval_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CycleOutcome {
    pub cycle_id: Uuid,
    pub state: String,
    pub selected_opportunity: Option<String>,
    pub package_id: Option<Uuid>,
    pub task_id: Option<Uuid>,
    pub task_state: Option<String>,
    pub summary: Value,
}

/// Persistence port for cycle/task/memory writes (cloud-api implements over PG).
#[async_trait]
pub trait CycleStore: Send + Sync {
    async fn begin_cycle(
        &self,
        employee_id: Uuid,
        trigger: &str,
        idem_key: &str,
    ) -> Result<Uuid, String>;
    async fn finish_cycle(
        &self,
        cycle_id: Uuid,
        state: &str,
        summary: &Value,
    ) -> Result<(), String>;
    async fn begin_task(
        &self,
        cycle_id: Uuid,
        agent: AgentKind,
        step: CycleStep,
        idem_key: &str,
    ) -> Result<Uuid, String>;
    async fn finish_task(&self, task_id: Uuid, state: &str, output: &Value) -> Result<(), String>;
    async fn add_memory(&self, item: &MemoryItem) -> Result<(), String>;
    async fn add_insight(
        &self,
        employee_id: Uuid,
        cycle_id: Uuid,
        content: &str,
        evidence: &Value,
        confidence: f64,
    ) -> Result<(), String>;
    async fn recent_topics(&self, employee_id: Uuid, limit: usize) -> Result<Vec<String>, String>;
    async fn mark_opportunity(&self, topic: &str, status: &str) -> Result<(), String>;
}

/// World Intelligence port (L7 adapter, WI §20).
#[async_trait]
pub trait IntelPort: Send + Sync {
    async fn opportunities(
        &self,
        ctx: &EmployeeContext,
        limit: usize,
    ) -> Result<Vec<Opportunity>, String>;
    /// 全文检索 intel_items (skill handler 用: social_search / topic_research / fact_checking)。
    /// 默认实现报不支持, 由云端 PG 实现提供真实数据。
    async fn search_items(&self, _query: &str, _limit: usize) -> Result<Vec<Value>, String> {
        Err("search_items not supported by this IntelPort".into())
    }
    /// 趋势聚合 (skill handler 用: trend_discovery), 复用 /intel/trends 的口径。
    async fn trending(&self, _limit: usize) -> Result<Vec<Value>, String> {
        Err("trending not supported by this IntelPort".into())
    }
}

/// 运行指标端口 (skill handler 用: analytics 域的 performance_analysis 等)。
/// 默认实现报不支持, 由云端 PG 实现提供真实计数。
#[async_trait]
pub trait StatsPort: Send + Sync {
    async fn metrics(&self) -> Result<Value, String> {
        Err("metrics not supported by this StatsPort".into())
    }
}

/// Content Factory / LLM port (DE §11: employee decides what/why, factory how).
#[async_trait]
pub trait LlmPort: Send + Sync {
    async fn generate(&self, prompt: &str, max_tokens: u32) -> Result<String, String>;
}

/// Publishing execution port (DE §12: separate service; employee only submits
/// packages and receives structured outcomes).
#[async_trait]
pub trait PublishPort: Send + Sync {
    /// Save generated text as a publishing package (approval per governance).
    async fn save_package(
        &self,
        employee: &Employee,
        topic: &str,
        text: &str,
    ) -> Result<(Uuid, String), String>;
    /// Approve a pending package (only when employee.auto_publish governance allows).
    async fn approve_package(&self, package_id: Uuid) -> Result<(), String>;
    /// Submit authorized publish; returns task receipt.
    async fn publish(
        &self,
        employee: &Employee,
        package_id: Uuid,
    ) -> Result<PublishReceipt, String>;
    /// Verify execution outcome by task state (DE §12: never trust the click).
    async fn task_state(&self, task_id: Uuid) -> Result<String, String>;
}

#[async_trait]
impl CycleStore for Box<dyn CycleStore> {
    async fn begin_cycle(&self, e: Uuid, t: &str, k: &str) -> Result<Uuid, String> {
        (**self).begin_cycle(e, t, k).await
    }
    async fn finish_cycle(&self, c: Uuid, s: &str, sum: &Value) -> Result<(), String> {
        (**self).finish_cycle(c, s, sum).await
    }
    async fn begin_task(
        &self,
        c: Uuid,
        a: AgentKind,
        s: CycleStep,
        k: &str,
    ) -> Result<Uuid, String> {
        (**self).begin_task(c, a, s, k).await
    }
    async fn finish_task(&self, t: Uuid, s: &str, o: &Value) -> Result<(), String> {
        (**self).finish_task(t, s, o).await
    }
    async fn add_memory(&self, m: &MemoryItem) -> Result<(), String> {
        (**self).add_memory(m).await
    }
    async fn add_insight(
        &self,
        e: Uuid,
        c: Uuid,
        ct: &str,
        ev: &Value,
        cf: f64,
    ) -> Result<(), String> {
        (**self).add_insight(e, c, ct, ev, cf).await
    }
    async fn recent_topics(&self, e: Uuid, l: usize) -> Result<Vec<String>, String> {
        (**self).recent_topics(e, l).await
    }
    async fn mark_opportunity(&self, t: &str, s: &str) -> Result<(), String> {
        (**self).mark_opportunity(t, s).await
    }
}

#[async_trait]
impl IntelPort for Box<dyn IntelPort> {
    async fn opportunities(
        &self,
        ctx: &EmployeeContext,
        limit: usize,
    ) -> Result<Vec<Opportunity>, String> {
        (**self).opportunities(ctx, limit).await
    }
    async fn search_items(&self, q: &str, limit: usize) -> Result<Vec<Value>, String> {
        (**self).search_items(q, limit).await
    }
    async fn trending(&self, limit: usize) -> Result<Vec<Value>, String> {
        (**self).trending(limit).await
    }
}

#[async_trait]
impl StatsPort for Box<dyn StatsPort> {
    async fn metrics(&self) -> Result<Value, String> {
        (**self).metrics().await
    }
}

pub struct CycleDeps {
    pub store: Box<dyn CycleStore>,
    pub intel: Box<dyn IntelPort>,
    pub llm: Box<dyn LlmPort>,
    pub publish: Box<dyn PublishPort>,
    pub stats: Box<dyn StatsPort>,
}

/// Minimum opportunity score to act on in a cycle.
pub const MIN_OPPORTUNITY_SCORE: f64 = 0.30;
/// Platform text limit enforced at Review (critic) step.
pub fn platform_text_limit(channel: &str) -> usize {
    match channel {
        "x" => 280,
        _ => 40_000,
    }
}

/// X counts most CJK chars as weight 2 toward the 280 limit; ASCII counts 1.
/// The critic enforces the *weighted* length so Chinese drafts are not over-long.
pub fn weighted_len(s: &str) -> usize {
    s.chars()
        .map(|c| {
            if (c as u32) >= 0x1100
                && ((c as u32) <= 0x115F
                    || (0x2E80..=0xA4CF).contains(&(c as u32))
                    || (0xAC00..=0xD7A3).contains(&(c as u32))
                    || (0xF900..=0xFAFF).contains(&(c as u32))
                    || (0xFE30..=0xFE4F).contains(&(c as u32))
                    || (0xFF00..=0xFF60).contains(&(c as u32))
                    || (0xFFE0..=0xFFE6).contains(&(c as u32))
                    || (0x20000..=0x2FFFD).contains(&(c as u32))
                    || (0x30000..=0x3FFFD).contains(&(c as u32)))
            {
                2
            } else {
                1
            }
        })
        .sum()
}

/// Run one operating cycle for an employee. Idempotent per (cycle, step) key.
#[allow(clippy::too_many_arguments)] // 领域编排入口, 参数为完整上下文
pub async fn run_cycle(
    deps: &CycleDeps,
    employee: &Employee,
    identity: &Identity,
    mission: &Mission,
    strategy: &Strategy,
    // 员工绑定且启用的 skills; 各 agent 按域挑选 (DE: agent 动态调用 skill)
    skills: &[Skill],
    trigger: &str,
    idem_key: &str,
) -> Result<CycleOutcome, String> {
    let cycle_id = deps
        .store
        .begin_cycle(employee.id, trigger, idem_key)
        .await?;

    // --- Observe: load context (identity/mission/strategy passed in; memory via store)
    let recent = deps
        .store
        .recent_topics(employee.id, 20)
        .await
        .unwrap_or_default();
    let ctx = EmployeeContext {
        employee_id: employee.id.to_string(),
        mission: mission.statement.clone(),
        audience: identity.audience.clone(),
        pillars: strategy.pillars.clone(),
        channel: employee.channel.clone(),
        recent_topics: recent.clone(),
        max_cost_usd_per_cycle: None,
    };
    // skill handler 运行时上下文: persona 供 llm 类 handler 组 prompt
    let persona = json!({
        "name": identity.name, "role": identity.role,
        "mission": mission.statement, "audience": identity.audience,
        "voice": identity.voice, "pillars": strategy.pillars,
        "language": identity.language,
    });
    let hctx = SkillRunCtx {
        deps,
        employee,
        persona: persona.clone(),
        no_go: Vec::new(),
    };
    step(
        &deps.store,
        cycle_id,
        AgentKind::Executive,
        CycleStep::Observe,
        idem_key,
        json!({"config_version": employee.config_version}),
    )
    .await?;

    // --- Understand: query World Intelligence through the adapter
    let opps = deps.intel.opportunities(&ctx, 10).await?;
    let research_skills: Vec<&str> = skills_for_agent(skills, &AgentKind::ResearchStrategy)
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    step(
        &deps.store,
        cycle_id,
        AgentKind::ResearchStrategy,
        CycleStep::Understand,
        idem_key,
        json!({"opportunities": opps.len(), "skills": research_skills}),
    )
    .await?;

    // --- Decide: pick top opportunity above threshold
    let selected = opps
        .iter()
        .filter(|o| o.score >= MIN_OPPORTUNITY_SCORE)
        .max_by(|a, b| {
            a.score
                .partial_cmp(&b.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .cloned();
    let Some(opp) = selected else {
        let summary =
            json!({"reason": "no opportunity above threshold", "opportunities": opps.len()});
        deps.store
            .finish_cycle(cycle_id, "SUCCEEDED", &summary)
            .await?;
        return Ok(CycleOutcome {
            cycle_id,
            state: "SUCCEEDED".into(),
            selected_opportunity: None,
            package_id: None,
            task_id: None,
            task_state: None,
            summary,
        });
    };
    deps.store
        .mark_opportunity(&opp.topic, "selected")
        .await
        .ok();
    step(
        &deps.store,
        cycle_id,
        AgentKind::Executive,
        CycleStep::Decide,
        idem_key,
        json!({"topic": opp.topic, "score": opp.score}),
    )
    .await?;

    // --- Plan: angle + channel from strategy (+ strategy-domain skills)
    // 同时真正执行 research 域 handler: topic_research 拉真实情报证据喂给创作
    let research_run = run_skill(
        "topic_research",
        &json!({ "topic": opp.topic, "limit": 10 }),
        &hctx,
    )
    .await
    .ok();
    let evidence_items: Vec<Value> = research_run
        .as_ref()
        .and_then(|r| r.output.get("items").cloned())
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    let handler_note = research_run.as_ref().map(|r| {
        json!({
            "skill": r.skill, "duration_ms": r.duration_ms,
            "count": r.output.get("count").cloned().unwrap_or(json!(0)),
        })
    });
    let strategy_skills: Vec<&str> = skills_for_agent(skills, &AgentKind::ResearchStrategy)
        .iter()
        .filter(|s| s.domain == "strategy")
        .map(|s| s.name.as_str())
        .collect();
    let plan = json!({
        "topic": opp.topic,
        "angle": "explanation",
        "channel": employee.channel,
        "format": strategy.formats.first().cloned().unwrap_or_else(|| "text".into()),
        "evidence": opp.evidence,
        "evidence_items": evidence_items.iter().take(5).cloned().collect::<Vec<_>>(),
        "handler": handler_note,
        "skills": strategy_skills,
    });
    step(
        &deps.store,
        cycle_id,
        AgentKind::ResearchStrategy,
        CycleStep::Plan,
        idem_key,
        plan.clone(),
    )
    .await?;

    // --- Create: LLM draft via Content Factory port
    let prompt = format!(
        "You are {name} ({role}). Mission: {mission}. Audience: {audience:?}. Voice: {voice}. Pillars: {pillars:?}. Language: {language}.\n\
         Write ONE {channel} post (max {limit} weighted chars; CJK counts double) entirely in {language}, about: {topic}. Evidence: {evidence:?}.\n\
         Do NOT repeat these recent topics: {recent:?}.",
        name = identity.name,
        role = identity.role,
        mission = mission.statement,
        audience = identity.audience,
        voice = identity.voice,
        pillars = strategy.pillars,
        language = identity.language,
        channel = employee.channel,
        limit = platform_text_limit(&employee.channel),
        topic = opp.topic,
        evidence = opp.evidence,
        recent = ctx.recent_topics,
    );
    // Content agent 按渠道挑选内容生产 skill, 把 skill 的契约写进 prompt
    let content_skill = pick_content_skill(skills, &employee.channel);
    let prompt = match content_skill {
        Some(s) => format!(
            "{prompt}\nSkill: {} v{} — {}. Follow this skill's input/output contract.",
            s.name, s.version, s.description
        ),
        None => prompt,
    };
    // 优先走真正的 skill handler (social_post_generation → llm), 失败回退内联 prompt
    let create_input = json!({
        "topic": opp.topic,
        "evidence": opp.evidence,
        "evidence_items": evidence_items.iter().take(5).cloned().collect::<Vec<_>>(),
        "recent": recent,
    });
    let create_run = run_skill("social_post_generation", &create_input, &hctx)
        .await
        .ok();
    let draft = match &create_run {
        Some(r) => r
            .output
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string(),
        None => deps.llm.generate(&prompt, 512).await?,
    };
    if draft.is_empty() {
        return Err("content handler and fallback both produced empty draft".into());
    }
    step(
        &deps.store,
        cycle_id,
        AgentKind::Content,
        CycleStep::Create,
        idem_key,
        json!({
            "draft_len": draft.chars().count(),
            "skill": content_skill.map(|s| json!({"name": s.name, "version": s.version, "tools": s.tools})),
            "handler": create_run.as_ref().map(|r| json!({
                "skill": r.skill, "duration_ms": r.duration_ms,
            })),
        }),
    )
    .await?;

    // --- Review: critic gate (length, duplication, boundaries)
    let limit = platform_text_limit(&employee.channel);
    let draft_weight = weighted_len(&draft);
    let quality_skills: Vec<&str> = skills_for_agent(skills, &AgentKind::CriticCompliance)
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    let mut verdict = json!({"pass": true, "checks": {}, "skills": quality_skills});
    if draft_weight > limit {
        verdict["pass"] = json!(false);
        verdict["checks"]["length"] =
            json!({"ok": false, "limit": limit, "weighted_len": draft_weight});
    }
    if ctx.recent_topics.iter().any(|r| r == &opp.topic) {
        verdict["pass"] = json!(false);
        verdict["checks"]["duplicate_topic"] = json!({"ok": false});
    }
    // 真执行 quality 域 handler: content_quality_review (llm 审查) + platform_compliance (规则)
    let review_run = run_skill("content_quality_review", &json!({"text": draft}), &hctx)
        .await
        .ok();
    let compliance_run = run_skill("platform_compliance", &json!({"text": draft}), &hctx)
        .await
        .ok();
    if let Some(c) = &compliance_run
        && c.output.get("pass").and_then(|v| v.as_bool()) == Some(false)
    {
        verdict["pass"] = json!(false);
        verdict["checks"]["platform_compliance"] = c.output.clone();
    }
    verdict["handlers"] = json!([
        review_run
            .as_ref()
            .map(|r| json!({"skill": r.skill, "duration_ms": r.duration_ms, "output": r.output})),
        compliance_run
            .as_ref()
            .map(|r| json!({"skill": r.skill, "duration_ms": r.duration_ms, "output": r.output})),
    ]);
    step(
        &deps.store,
        cycle_id,
        AgentKind::CriticCompliance,
        CycleStep::Review,
        idem_key,
        verdict.clone(),
    )
    .await?;
    if verdict["pass"] == json!(false) {
        let summary = json!({"reason": "critic rejected draft", "verdict": verdict});
        deps.store
            .finish_cycle(cycle_id, "SUCCEEDED", &summary)
            .await?;
        return Ok(CycleOutcome {
            cycle_id,
            state: "SUCCEEDED".into(),
            selected_opportunity: Some(opp.topic.clone()),
            package_id: None,
            task_id: None,
            task_state: None,
            summary,
        });
    }

    // --- Execute: save package, then publish only if governance allows
    let (package_id, approval) = deps
        .publish
        .save_package(employee, &opp.topic, &draft)
        .await?;
    let mut receipt = None;
    let mut task_id = None;
    let mut task_state = None;
    if approval == "approved" || employee.auto_publish {
        if approval != "approved" {
            deps.publish.approve_package(package_id).await?;
        }
        // content_publishing handler = PublishPort 真派发 (设备任务入队)
        let r = deps.publish.publish(employee, package_id).await?;
        task_id = Some(r.task_id);
        // --- Measure: verify outcome via task state, never trust the submit
        task_state = Some(
            deps.publish
                .task_state(r.task_id)
                .await
                .unwrap_or_else(|_| "UNKNOWN".into()),
        );
        // analytics 域 handler: performance_analysis 真查运行指标
        let perf_run = run_skill("performance_analysis", &json!({}), &hctx)
            .await
            .ok();
        let step_out = json!({"task_id": r.task_id, "route": r.route, "status": r.status,
                              "handler": "content_publishing"});
        receipt = Some(r);
        step(
            &deps.store,
            cycle_id,
            AgentKind::Executive,
            CycleStep::Execute,
            idem_key,
            step_out,
        )
        .await?;
        step(
            &deps.store,
            cycle_id,
            AgentKind::AnalyticsLearning,
            CycleStep::Measure,
            idem_key,
            json!({"task_state": task_state,
                   "handler": perf_run.as_ref().map(|r| json!({"skill": r.skill, "duration_ms": r.duration_ms, "metrics": r.output.get("metrics").cloned()}))}),
        )
        .await?;
    } else {
        // package pending human approval (FR-024): cycle parks at WAITING_APPROVAL
        step(
            &deps.store,
            cycle_id,
            AgentKind::CriticCompliance,
            CycleStep::Execute,
            idem_key,
            json!({"package_id": package_id, "state": "WAITING_APPROVAL"}),
        )
        .await?;
    }

    // --- Learn: structured memory + insight (evidence-linked)
    let mem = MemoryItem {
        id: Uuid::new_v4(),
        employee_id: employee.id,
        r#type: MemoryType::Episodic,
        content: format!(
            "cycle {} on topic '{}': package={}, task={:?}",
            cycle_id, opp.topic, package_id, task_id
        ),
        evidence: vec![
            json!({"opportunity": opp.rationale}),
            json!({"draft_len": draft.chars().count()}),
        ],
        confidence: opp.score,
        created_at: Utc::now(),
        last_validated: None,
    };
    deps.store.add_memory(&mem).await.ok();
    // learning 域 handler: insight_extraction 真调用 llm 从周期摘要提炼洞察, 失败回退模板句
    let insight_run = run_skill(
        "insight_extraction",
        &json!({"cycle_summary": {
            "topic": opp.topic, "score": opp.score, "package_id": package_id,
            "task_id": task_id, "task_state": task_state, "draft_len": draft.chars().count(),
        }}),
        &hctx,
    )
    .await
    .ok();
    let insight_text = insight_run
        .as_ref()
        .and_then(|r| r.output.get("raw").and_then(|v| v.as_str()))
        .map(|s| s.chars().take(300).collect::<String>())
        .unwrap_or_else(|| {
            format!(
                "Opportunity '{}' scored {:.2}; {}",
                opp.topic,
                opp.score,
                if task_id.is_some() {
                    "published"
                } else {
                    "parked for approval"
                }
            )
        });
    deps.store
        .add_insight(
            employee.id,
            cycle_id,
            &insight_text,
            &json!({"evidence": opp.evidence,
                    "handler": insight_run.as_ref().map(|r| json!({"skill": r.skill, "duration_ms": r.duration_ms}))}),
            opp.score,
        )
        .await
        .ok();
    let learning_skills: Vec<&str> = skills_for_agent(skills, &AgentKind::AnalyticsLearning)
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    step(
        &deps.store,
        cycle_id,
        AgentKind::AnalyticsLearning,
        CycleStep::Learn,
        idem_key,
        json!({"memory_id": mem.id, "skills": learning_skills}),
    )
    .await?;

    // --- Adapt: propose tactical adjustment (recommendation only; strategy
    // changes need approval per DE §7)
    let adapt = json!({
        "recommendation": format!("increase weight of pillar related to '{}'", opp.topic),
        "requires_approval": true,
    });
    step(
        &deps.store,
        cycle_id,
        AgentKind::Executive,
        CycleStep::Adapt,
        idem_key,
        adapt.clone(),
    )
    .await?;

    let summary = json!({
        "topic": opp.topic,
        "opportunity_score": opp.score,
        "package_id": package_id,
        "task_id": task_id,
        "task_state": task_state,
        "receipt": receipt,
        "adapt": adapt,
    });
    deps.store
        .finish_cycle(cycle_id, "SUCCEEDED", &summary)
        .await?;
    Ok(CycleOutcome {
        cycle_id,
        state: "SUCCEEDED".into(),
        selected_opportunity: Some(opp.topic),
        package_id: Some(package_id),
        task_id,
        task_state,
        summary,
    })
}

async fn step(
    store: &dyn CycleStore,
    cycle_id: Uuid,
    agent: AgentKind,
    s: CycleStep,
    idem_key: &str,
    output: Value,
) -> Result<(), String> {
    let key = format!("{idem_key}:{}:{}", agent.as_str(), s.as_str());
    let t = store.begin_task(cycle_id, agent, s, &key).await?;
    store.finish_task(t, "SUCCEEDED", &output).await
}
