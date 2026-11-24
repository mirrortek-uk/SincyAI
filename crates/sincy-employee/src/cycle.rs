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
    /// 落实验假说 (learning 闭环: Analytics → Hypothesis → Experiment)。
    /// 默认不支持, 由云端 PG 实现写 employee_experiments。
    async fn add_experiment(
        &self,
        _employee_id: Uuid,
        _hypothesis: &str,
        _variable: &str,
    ) -> Result<Uuid, String> {
        Err("add_experiment not supported by this CycleStore".into())
    }
    /// 平台审核规则 (platform_review_rules)。默认返回内置默认值, 由云端 PG 实现读表。
    async fn review_rule(&self, platform: &str) -> Result<PlatformReviewRule, String> {
        Ok(PlatformReviewRule::default_for(platform))
    }
    /// 账号级语气覆盖 (employee_accounts.voice_profile, 0047)。默认 None, 由云端 PG 实现读表。
    async fn account_voice_profile(
        &self,
        _employee_id: Uuid,
        _platform: &str,
    ) -> Result<Option<Value>, String> {
        Ok(None)
    }
    /// 把 thread 拆分结果写回发布包。默认不支持, 由云端 PG 实现写 publishing_packages.thread_chunks。
    async fn set_thread_chunks(&self, _package_id: Uuid, _chunks: &[String]) -> Result<(), String> {
        Ok(())
    }
    /// 落实验假说 + 结构化调整 (learning 闭环 a): adjustment = {"angle","pillar_boost"},
    /// 下周期由 take_pending_adjustment 原子取用。默认不支持, 由云端 PG 实现。
    async fn add_experiment_adjustment(
        &self,
        _employee_id: Uuid,
        _hypothesis: &str,
        _variable: &str,
        _adjustment: &Value,
    ) -> Result<Uuid, String> {
        Err("add_experiment_adjustment not supported by this CycleStore".into())
    }
    /// 原子取用最近一条未应用的调整 (SELECT → 标 applied_at → 返回)。
    /// 默认返回 None, 由云端 PG 实现。
    async fn take_pending_adjustment(&self, _employee_id: Uuid) -> Result<Option<Value>, String> {
        Ok(None)
    }
    /// 发布后登记延迟测量 (b): due_at 到期后由后台 drain 读真实终态。默认空实现。
    async fn enqueue_measurement(&self, _m: &MeasurementRequest) -> Result<(), String> {
        Ok(())
    }
    /// drain 完成一条测量: 写回 metrics + insight。默认空实现。
    async fn complete_measurement(
        &self,
        _id: Uuid,
        _metrics: &Value,
        _insight: &str,
    ) -> Result<(), String> {
        Ok(())
    }
}

/// 延迟测量登记项: 发布成功后 enqueue, due_at 后由后台 drain 消费
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MeasurementRequest {
    pub cycle_id: Uuid,
    pub employee_id: Uuid,
    pub agency_id: Option<Uuid>,
    pub package_id: Option<Uuid>,
    pub task_id: Option<Uuid>,
    pub due_at: chrono::DateTime<Utc>,
}

/// 平台审核门规则: 加权字数上限 + 超限行为 (reject=打回 / thread=拆串行帖)。
/// 默认值内置在代码里, 生产环境由 platform_review_rules 表覆盖(设置页可改)。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlatformReviewRule {
    pub platform: String,
    pub weighted_limit: usize,
    pub on_over_limit: String,
}

impl PlatformReviewRule {
    pub fn default_for(platform: &str) -> Self {
        if platform == "x" {
            Self {
                platform: platform.into(),
                weighted_limit: 280,
                on_over_limit: "thread".into(),
            }
        } else {
            Self {
                platform: platform.into(),
                weighted_limit: 40_000,
                on_over_limit: "reject".into(),
            }
        }
    }
}

/// 按加权长度拆分 thread: 优先段落边界, 再句子边界, 最后硬切。
/// 每块加权长度都不超过 limit (CJK 字符权重 2, 见 weighted_len)。
pub fn split_thread(text: &str, limit: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0usize;
    fn push_chunk(cur: &mut String, cur_w: &mut usize, out: &mut Vec<String>) {
        if !cur.trim().is_empty() {
            out.push(std::mem::take(cur));
        }
        cur.clear();
        *cur_w = 0;
    }
    // 先按段落切
    for para in text.split("\n\n") {
        let para_w = weighted_len(para);
        if para_w <= limit {
            if cur_w + para_w + 2 > limit && !cur.is_empty() {
                push_chunk(&mut cur, &mut cur_w, &mut out);
            }
            if !cur.is_empty() {
                cur.push_str("\n\n");
                cur_w += 2;
            }
            cur.push_str(para);
            cur_w += para_w;
        } else {
            // 段落本身超限: 按句子切
            if !cur.is_empty() {
                push_chunk(&mut cur, &mut cur_w, &mut out);
            }
            for sent in split_sentences(para) {
                let sent_w = weighted_len(&sent);
                if sent_w > limit {
                    // 句子也超限: 硬切
                    let mut rest = sent.as_str();
                    while weighted_len(rest) > limit {
                        let (head, tail) = take_weighted(rest, limit);
                        out.push(head.to_string());
                        rest = tail;
                    }
                    if !rest.trim().is_empty() {
                        out.push(rest.to_string());
                    }
                } else if cur_w + sent_w > limit && !cur.is_empty() {
                    push_chunk(&mut cur, &mut cur_w, &mut out);
                    cur.push_str(&sent);
                    cur_w = sent_w;
                } else {
                    cur.push_str(&sent);
                    cur_w += sent_w;
                }
            }
        }
    }
    push_chunk(&mut cur, &mut cur_w, &mut out);
    if out.is_empty() && !text.trim().is_empty() {
        out.push(text.to_string());
    }
    out
}

/// 按中英文句末标点切句。
fn split_sentences(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in s.chars() {
        cur.push(c);
        if matches!(c, '。' | '！' | '？' | '.' | '!' | '?') {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// 从头取不超过 limit 加权长度的前缀, 返回 (前缀, 剩余)。
fn take_weighted(s: &str, limit: usize) -> (&str, &str) {
    let mut w = 0usize;
    for (i, c) in s.char_indices() {
        let cw = if (c as u32) >= 0x1100 { 2 } else { 1 };
        if w + cw > limit {
            return (&s[..i], &s[i..]);
        }
        w += cw;
    }
    (s, "")
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
    /// 粉丝增量 (patch-A): 读 account_metric_samples 按窗口现算 {followers_now, delta, delta_pct}。
    /// 默认不支持, 由云端 PG 实现; mock 不受影响。
    async fn account_growth(&self, _employee_id: Uuid, _window_days: i64) -> Result<Value, String> {
        Err("account_growth not supported by this StatsPort".into())
    }
}

/// Content Factory / LLM port (DE §11: employee decides what/why, factory how).
#[async_trait]
pub trait LlmPort: Send + Sync {
    async fn generate(&self, prompt: &str, max_tokens: u32) -> Result<String, String>;
}

/// 把 LlmPort 适配成内容工厂的 TextGenerator 端口 (工序1/2 走同一 LLM)
struct LlmTextGen<'a>(&'a dyn LlmPort);

#[async_trait::async_trait]
impl sincy_content::factory::TextGenerator for LlmTextGen<'_> {
    async fn generate(&self, prompt: &str, max_tokens: u32) -> Result<String, String> {
        self.0.generate(prompt, max_tokens).await
    }
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
    async fn add_experiment(&self, e: Uuid, h: &str, v: &str) -> Result<Uuid, String> {
        (**self).add_experiment(e, h, v).await
    }
    async fn review_rule(&self, platform: &str) -> Result<PlatformReviewRule, String> {
        (**self).review_rule(platform).await
    }
    async fn set_thread_chunks(&self, p: Uuid, c: &[String]) -> Result<(), String> {
        (**self).set_thread_chunks(p, c).await
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
    async fn account_growth(&self, employee_id: Uuid, window_days: i64) -> Result<Value, String> {
        (**self).account_growth(employee_id, window_days).await
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
    let mut ctx = EmployeeContext {
        employee_id: employee.id.to_string(),
        mission: mission.statement.clone(),
        audience: identity.audience.clone(),
        pillars: strategy.pillars.clone(),
        channel: employee.channel.clone(),
        recent_topics: recent.clone(),
        max_cost_usd_per_cycle: None,
    };
    // 学习闭环 (a): 周期开头原子取用上一周期留下的结构化调整, 真正改掉 pillars / angle
    let pending_adjustment = deps
        .store
        .take_pending_adjustment(employee.id)
        .await
        .ok()
        .flatten();
    let mut applied_angle: Option<String> = None;
    if let Some(adj) = &pending_adjustment {
        if let Some(angle) = adj.get("angle").and_then(|v| v.as_str()) {
            applied_angle = Some(angle.to_string());
        }
        if let Some(boost) = adj.get("pillar_boost").and_then(|v| v.as_object()) {
            // 加权相关支柱前置: 命中 boost 的支柱按 boost 值降序排到最前
            let mut pillars = ctx.pillars.clone();
            pillars.sort_by(|a, b| {
                let wa = boost.get(a).and_then(|v| v.as_f64()).unwrap_or(0.0);
                let wb = boost.get(b).and_then(|v| v.as_f64()).unwrap_or(0.0);
                wb.partial_cmp(&wa).unwrap_or(std::cmp::Ordering::Equal)
            });
            ctx.pillars = pillars;
        }
    }
    // Voice Profile: 有效语气 = merge_opt(员工基线 identity.voice_profile, 账号覆盖 employee_accounts.voice_profile)
    let acct_voice = deps
        .store
        .account_voice_profile(employee.id, &employee.channel)
        .await
        .ok()
        .flatten()
        .unwrap_or(Value::Null);
    let voice_eff = sincy_content::voice::effective_voice(&identity.voice_profile, &acct_voice);
    let voice_block = sincy_content::voice::compile_voice_prompt(&voice_eff, 3);
    // skill handler 运行时上下文: persona 供 llm 类 handler 组 prompt
    let persona = json!({
        "name": identity.name, "role": identity.role,
        "mission": mission.statement, "audience": identity.audience,
        "voice": identity.voice, "pillars": strategy.pillars,
        "language": identity.language, "voice_block": voice_block,
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
    // patch-B: target_followers KPI — 读近 7 天 Δ粉丝, 落后目标则 plan 标记 behind,
    // Create prompt 据此偏涨粉角度 (hot_take/listicle), drain 侧再用 Δ粉丝当 reward 闭环
    let follower_kpi = match strategy.target_followers {
        Some(target) => {
            let g = deps
                .stats
                .account_growth(employee.id, 7)
                .await
                .unwrap_or_else(|_| json!({}));
            let now = g["followers_now"].as_i64();
            let delta = g["delta"].as_i64();
            json!({
                "target_followers": target,
                "followers_now": now,
                "delta_7d": delta,
                "behind": now.map(|n| n < target).unwrap_or(false),
            })
        }
        None => Value::Null,
    };
    let plan = json!({
        "topic": opp.topic,
        "angle": applied_angle.clone().unwrap_or_else(|| "explanation".into()),
        "applied_adjustment": pending_adjustment.clone(),
        "follower_kpi": follower_kpi,
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

    // 平台审核规则: 优先读 platform_review_rules 表(设置页可改), 回退内置默认值
    let rule = deps
        .store
        .review_rule(&employee.channel)
        .await
        .unwrap_or_else(|_| PlatformReviewRule::default_for(&employee.channel));

    // --- Create: LLM draft via Content Factory port
    let voice_line = if voice_block.is_empty() {
        String::new()
    } else {
        format!("\n{voice_block}")
    };
    let prompt = format!(
        "You are {name} ({role}). Mission: {mission}. Audience: {audience:?}. Voice: {voice}.{voice_line} Pillars: {pillars:?}. Language: {language}.\n\
         Write ONE {channel} post (max {limit} weighted chars; CJK counts double) entirely in {language}, about: {topic}. Evidence: {evidence:?}.\n\
         Do NOT repeat these recent topics: {recent:?}.{kpi_line}",
        name = identity.name,
        role = identity.role,
        mission = mission.statement,
        audience = identity.audience,
        voice = identity.voice,
        voice_line = voice_line,
        pillars = strategy.pillars,
        language = identity.language,
        channel = employee.channel,
        limit = rule.weighted_limit,
        topic = opp.topic,
        evidence = opp.evidence,
        recent = ctx.recent_topics,
        kpi_line = match &plan["follower_kpi"] {
            Value::Null => String::new(),
            k => format!("
Follower KPI: {k}. If behind target, bias toward growth-friendly angle (hot_take / listicle / strong hook)."),
        },
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
    // 内容工厂主路径: 工序0 Brief → 工序1 策略 → 工序2 母稿(grounding+事实核查)
    let factory_draft = {
        let brief = sincy_content::factory::Brief::from_opportunity(
            &employee.brand_id.map(|b| b.to_string()).unwrap_or_default(),
            "",
            None,
            &mission.statement,
            vec![employee.channel.clone()],
            &opp.topic,
        );
        let signals: Vec<serde_json::Value> = evidence_items.iter().take(8).cloned().collect();
        let llm_gen = LlmTextGen(deps.llm.as_ref());
        async move {
            let plan = sincy_content::factory::plan_strategy(&brief, &signals, &llm_gen)
                .await
                .ok()?;
            let master = sincy_content::factory::create_master_with_voice(
                &brief,
                &plan,
                &signals,
                &llm_gen,
                &voice_block,
            )
            .await
            .ok()?;
            Some(master.text)
        }
        .await
        .filter(|t| !t.is_empty())
    };
    let fallback_draft = match &create_run {
        Some(r) => r
            .output
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string(),
        None => deps.llm.generate(&prompt, 512).await.unwrap_or_default(),
    };
    let draft = factory_draft.unwrap_or(fallback_draft);
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
    // 审核门来自可配置规则: 超限时 reject=打回 / thread=拆成串行帖放行
    let limit = rule.weighted_limit;
    let draft_weight = weighted_len(&draft);
    let mut thread_chunks: Vec<String> = Vec::new();
    let quality_skills: Vec<&str> = skills_for_agent(skills, &AgentKind::CriticCompliance)
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    let mut verdict = json!({"pass": true, "checks": {}, "skills": quality_skills});
    if draft_weight > limit {
        if rule.on_over_limit == "thread" {
            thread_chunks = split_thread(&draft, limit);
            verdict["checks"]["length"] = json!({
                "ok": true, "limit": limit, "weighted_len": draft_weight,
                "action": "thread", "chunks": thread_chunks.len(),
            });
        } else {
            verdict["pass"] = json!(false);
            verdict["checks"]["length"] =
                json!({"ok": false, "limit": limit, "weighted_len": draft_weight});
        }
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
    // 质量门真拦截 (c): 解析 content_quality_review 的 LLM 判定 {score, pass},
    // pass=false 或 score<60 直接判负; 解析失败 fail-open (不误拦)
    if let Some(r) = &review_run
        && let Some((score, pass)) = parse_llm_review(&r.output)
    {
        verdict["checks"]["llm_review"] = json!({"score": score, "pass": pass});
        if !pass || score < 60.0 {
            verdict["pass"] = json!(false);
        }
    }
    // Voice 硬约束: 命中语气档案的 avoid_words / banned_moves 直接判负
    let vv = sincy_content::voice::voice_violations(&voice_eff, &draft);
    if !vv.is_empty() {
        verdict["pass"] = json!(false);
        verdict["checks"]["voice"] = json!({
            "ok": false,
            "violations": vv.iter().map(|v| json!({"code": v.code, "message": v.message})).collect::<Vec<_>>(),
        });
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
    if !thread_chunks.is_empty() {
        deps.store
            .set_thread_chunks(package_id, &thread_chunks)
            .await
            .ok();
    }
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
        // 延迟测量 (b): 发布后登记, 默认 6h 后到期 (SINCY_MEASURE_DELAY_SECS 可调),
        // 由后台 drain 读 device_tasks 真实终态回写指标与数据驱动 adjustment
        let delay_secs: i64 = std::env::var("SINCY_MEASURE_DELAY_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(6 * 3600);
        let _ = deps
            .store
            .enqueue_measurement(&MeasurementRequest {
                cycle_id,
                employee_id: employee.id,
                agency_id: None,
                package_id: Some(package_id),
                task_id: Some(r.task_id),
                due_at: Utc::now() + chrono::Duration::seconds(delay_secs),
            })
            .await;
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

    // --- Adapt: learning 闭环 — 把本周期结果 + 运行指标喂给 llm 生成可检验假说,
    // 落 employee_experiments 表 (Analytics ≠ Learning, 要产出 hypothesis + experiment)
    let metrics = ctx_deps_stats(deps).await;
    let hyp_run = run_skill(
        "strategy_optimization",
        &json!({
            "cycle_summary": {
                "topic": opp.topic, "score": opp.score, "task_state": task_state,
                "draft_len": draft.chars().count(),
            },
            "metrics": metrics,
        }),
        &hctx,
    )
    .await
    .ok();
    let hypothesis = hyp_run
        .as_ref()
        .and_then(|r| r.output.get("raw").and_then(|v| v.as_str()))
        .map(|s| s.chars().take(400).collect::<String>())
        .unwrap_or_else(|| format!("increase weight of pillar related to '{}'", opp.topic));
    // 结构化调整: 行为真的随上周期结果变 (angle 不再写死 explanation);
    // 延迟测量回采真实指标后会再写一条数据驱动的调整覆盖它
    let adjustment = json!({
        "angle": if task_state.as_deref() == Some("FAILED") || task_state.as_deref() == Some("FAILED_FINAL") {
            "how_to"
        } else if draft.chars().count() > 200 {
            "hot_take"
        } else {
            "explanation"
        },
        "pillar_boost": { opp.topic.clone(): 0.2 },
        "source": "in-cycle adapt",
    });
    let experiment_id = deps
        .store
        .add_experiment_adjustment(employee.id, &hypothesis, "hook_structure", &adjustment)
        .await
        .ok();
    let adapt = json!({
        "recommendation": hypothesis,
        "hypothesis": hypothesis,
        "experiment_id": experiment_id,
        "adjustment": adjustment,
        "handler": hyp_run.as_ref().map(|r| json!({"skill": r.skill, "duration_ms": r.duration_ms})),
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

/// 解析 content_quality_review handler 输出里的 LLM 判定 {score, pass}。
/// 找不到或解析失败返回 None (调用方 fail-open)。
pub fn parse_llm_review(output: &Value) -> Option<(f64, bool)> {
    let cand = output
        .get("llm_review")
        .or_else(|| output.get("review"))
        .or_else(|| output.get("raw"))
        .or_else(|| output.get("text"))?;
    let v = match cand {
        Value::String(s) => serde_json::from_str::<Value>(s).ok()?,
        other => other.clone(),
    };
    let score = v.get("score").and_then(|x| x.as_f64())?;
    let pass = v.get("pass").and_then(|x| x.as_bool())?;
    Some((score, pass))
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

/// 拉运行指标 (StatsPort), 失败返回空对象 —— 学习闭环不因指标缺失中断
async fn ctx_deps_stats(deps: &CycleDeps) -> Value {
    deps.stats.metrics().await.unwrap_or_else(|_| json!({}))
}

#[cfg(test)]
mod thread_tests {
    use super::*;

    #[test]
    fn split_short_text_stays_single() {
        let chunks = split_thread("hello world", 280);
        assert_eq!(chunks, vec!["hello world".to_string()]);
    }

    #[test]
    fn split_respects_weighted_limit_cjk_counts_double() {
        // 200 个中文字符 = 加权 400 > 280, 必须被拆开
        let text = "汉".repeat(200);
        let chunks = split_thread(&text, 280);
        assert!(chunks.len() >= 2);
        for c in &chunks {
            assert!(
                weighted_len(c) <= 280,
                "chunk weight {} > 280",
                weighted_len(c)
            );
        }
        assert_eq!(chunks.concat(), text);
    }

    #[test]
    fn split_prefers_paragraph_boundary() {
        let text = format!("{}\n\n{}", "a".repeat(200), "b".repeat(200));
        let chunks = split_thread(&text, 280);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].starts_with('a'));
        assert!(chunks[1].starts_with('b'));
    }

    #[test]
    fn default_rule_x_is_thread_280() {
        let r = PlatformReviewRule::default_for("x");
        assert_eq!(r.weighted_limit, 280);
        assert_eq!(r.on_over_limit, "thread");
        let r2 = PlatformReviewRule::default_for("youtube");
        assert_eq!(r2.on_over_limit, "reject");
    }

    #[test]
    fn llm_review_parsed_and_missing() {
        use serde_json::json;
        // 直接是对象
        let out = json!({"llm_review": {"score": 82.0, "pass": true}});
        assert_eq!(parse_llm_review(&out), Some((82.0, true)));
        // raw 里包 JSON 字符串
        let out = json!({"raw": r#"{"score": 41.5, "pass": false}"#});
        assert_eq!(parse_llm_review(&out), Some((41.5, false)));
        // 解析失败 → None (fail-open)
        let out = json!({"raw": "not json at all"});
        assert_eq!(parse_llm_review(&out), None);
        let out = json!({"text": "ok"});
        assert_eq!(parse_llm_review(&out), None);
    }
}
