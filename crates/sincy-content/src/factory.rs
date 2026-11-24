//! 内容工厂 Content Factory (全链路架构 工序0–9 的核心抽象, 可插拔):
//!
//! 工序0 Brief ← 三种触发入口 (①机会驱动 ②计划驱动 ③人工驱动, 断点①在此接通)
//! 工序1 策略/选题 → 工序2 核心创作(grounding+事实核查) → Master Content Asset
//! 工序3 平台适配 (每平台一个 adapter, 加新平台 = 加一个适配器)
//! 工序4 媒体生产 (统一 Generator 端口, provider 可互换: Faster-Whisper /
//!   MoneyPrinterTurbo / Pixelle-Video / Open-Generative-AI / Kling / Runway / Veo)
//! 工序5 质量门(已有 gate.rs) → 工序6 审批 → 工序7 打包(已有 package.rs)
//! 工序9 Learn 回流 (断点②): 指标/归因 → 策略与品牌记忆回写

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------- 工序0: Brief ----------

/// 触发来源 (三种入口都产出 "一条已相关的信号/机会" 作为工序0起点)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TriggerSource {
    /// ① 机会驱动 (默认): Opportunity 命中阈值 → 推给工厂
    OpportunityDriven,
    /// ② 计划驱动: 内容日历到点 → 按品牌检索信号库
    CalendarDriven,
    /// ③ 人工驱动: 运营直接指定选题
    Manual,
}

/// 工序0 内容任务书 Brief (全链路架构 "要补" 项, 现已补齐)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Brief {
    pub brief_id: String,
    pub trigger: TriggerSource,
    /// 起点信号/机会 (三种入口都带)
    pub seed_signal_id: Option<String>,
    pub seed_opportunity_id: Option<String>,
    pub brand_id: String,
    pub goal: String,
    pub platforms: Vec<String>,
    pub angle: String,
    pub audience: String,
    pub cta: String,
    pub constraints: Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl Brief {
    /// 从机会/信号创建 Brief (断点①: Opportunity → 工序0)
    pub fn from_opportunity(
        brand_id: &str,
        opportunity_id: &str,
        signal_id: Option<&str>,
        goal: &str,
        platforms: Vec<String>,
        angle: &str,
    ) -> Self {
        Self {
            brief_id: format!("brief_{}", uuid_like()),
            trigger: TriggerSource::OpportunityDriven,
            seed_signal_id: signal_id.map(|s| s.to_string()),
            seed_opportunity_id: Some(opportunity_id.to_string()),
            brand_id: brand_id.to_string(),
            goal: goal.to_string(),
            platforms,
            angle: angle.to_string(),
            audience: String::new(),
            cta: String::new(),
            constraints: serde_json::json!({}),
            created_at: chrono::Utc::now(),
        }
    }

    pub fn manual(brand_id: &str, topic: &str, platforms: Vec<String>) -> Self {
        Self {
            brief_id: format!("brief_{}", uuid_like()),
            trigger: TriggerSource::Manual,
            seed_signal_id: None,
            seed_opportunity_id: None,
            brand_id: brand_id.to_string(),
            goal: topic.to_string(),
            platforms,
            angle: topic.to_string(),
            audience: String::new(),
            cta: String::new(),
            constraints: serde_json::json!({}),
            created_at: chrono::Utc::now(),
        }
    }
}

fn uuid_like() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static C: AtomicU64 = AtomicU64::new(0);
    let n = C.fetch_add(1, Ordering::Relaxed);
    format!("{:013x}{:08x}", chrono::Utc::now().timestamp_millis(), n)
}

// ---------- 工序3: 平台适配器 (每平台一个, 可插拔) ----------

/// 平台适配规则: 格式/语气/链接CTA/标签/本地化/AI标识
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformSpec {
    pub platform: String,
    pub max_chars: Option<usize>,
    pub thread_limit: Option<usize>,
    pub tone: String,
    pub cta_style: String,
    pub hashtag_style: String,
    pub localization: String,
    pub ai_label_required: bool,
    /// 媒体规格: image/video/audio/carousel
    pub media_specs: Vec<String>,
}

pub trait PlatformAdapter: Send + Sync {
    fn spec(&self) -> &PlatformSpec;
    /// 把 Master Asset 适配成该平台变体 (工序3)
    fn adapt(&self, master: &str, brief: &Brief) -> String;
}

pub struct SimpleAdapter {
    spec: PlatformSpec,
}

impl SimpleAdapter {
    pub fn new(spec: PlatformSpec) -> Self {
        Self { spec }
    }
}

impl PlatformAdapter for SimpleAdapter {
    fn spec(&self) -> &PlatformSpec {
        &self.spec
    }
    fn adapt(&self, master: &str, brief: &Brief) -> String {
        let mut out = master.to_string();
        if !self.spec.hashtag_style.is_empty() && !brief.angle.is_empty() {
            out = format!(
                "{out}\n{}",
                self.spec
                    .hashtag_style
                    .replace("{topic}", &brief.angle.replace(' ', ""))
            );
        }
        if !self.spec.cta_style.is_empty() && !brief.cta.is_empty() {
            out = format!(
                "{out}\n{}",
                self.spec.cta_style.replace("{cta}", &brief.cta)
            );
        }
        if let Some(max) = self.spec.max_chars
            && out.chars().count() > max
        {
            let t: String = out.chars().take(max.saturating_sub(1)).collect();
            out = format!("{t}…");
        }
        out
    }
}

/// 内置平台适配器注册表 (加新平台 = push 一个 adapter)
pub fn default_adapters() -> Vec<Box<dyn PlatformAdapter>> {
    let mk = |platform: &str, max_chars: Option<usize>, thread: Option<usize>, ai_label: bool| {
        Box::new(SimpleAdapter::new(PlatformSpec {
            platform: platform.to_string(),
            max_chars,
            thread_limit: thread,
            tone: "brand-voice".into(),
            cta_style: "{cta}".into(),
            hashtag_style: "#{topic}".into(),
            localization: "zh-CN".into(),
            ai_label_required: ai_label,
            media_specs: vec!["image".into()],
        })) as Box<dyn PlatformAdapter>
    };
    vec![
        mk("x", Some(280), Some(8), true),
        mk("xiaohongshu", Some(1000), None, true),
        mk("weibo", Some(2000), None, false),
        mk("instagram", Some(2200), None, true),
        mk("tiktok", Some(2200), None, true),
        mk("youtube", Some(5000), None, true),
        mk("linkedin", Some(3000), None, false),
        mk("facebook", Some(63206), None, false),
    ]
}

// ---------- 工序1/2: 策略选题 + 核心创作 (grounding + 事实核查 → Master Asset) ----------

/// LLM 文本生成端口: cloud-api 用 sincy-ai gateway 实现, 员工周期用 LlmPort 适配
#[async_trait::async_trait]
pub trait TextGenerator: Send + Sync {
    async fn generate(&self, prompt: &str, max_tokens: u32) -> Result<String, String>;
}

/// 工序1 产出: 策略/选题方案
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyPlan {
    pub angle: String,
    pub key_points: Vec<String>,
    pub outline: Vec<String>,
}

/// 工序1 策略/选题 (薄包装): 无学习回灌时的默认入口, 保持既有调用点不变。
pub async fn plan_strategy(
    brief: &Brief,
    signals: &[Value],
    llm: &dyn TextGenerator,
) -> Result<StrategyPlan, String> {
    plan_strategy_with_learnings(brief, signals, &[], llm).await
}

/// 工序1 策略/选题 (带学习回灌): 把该品牌近期学习结论注入 prompt (断点② 闭合)。
pub async fn plan_strategy_with_learnings(
    brief: &Brief,
    signals: &[Value],
    learnings: &[String],
    llm: &dyn TextGenerator,
) -> Result<StrategyPlan, String> {
    let evidence: Vec<String> = signals
        .iter()
        .take(8)
        .map(|s| {
            format!(
                "- {} ({}): {}",
                s.get("signal_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("signal"),
                s.get("strength").and_then(|v| v.as_f64()).unwrap_or(0.0),
                s.get("payload")
                    .and_then(|p| p.get("title").or_else(|| p.get("topic")))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
            )
        })
        .collect();
    let prompt = format!(
        "You are the strategy step (工序1) of a content factory. Given the brief and grounded signals,          produce a JSON object {{angle, key_points[<=5], outline[3-5 steps]}}. No prose outside JSON.

         Brief: goal={} angle={} platforms={:?} audience={}
Signals:
{}",
        brief.goal, brief.angle, brief.platforms, brief.audience, evidence.join("
")
    );
    let prompt = if learnings.is_empty() {
        prompt
    } else {
        format!(
            "{prompt}\n\nPrior learnings for this brand (bias the angle & key_points toward what has worked):\n{}",
            learnings.join("\n")
        )
    };
    let raw = llm.generate(&prompt, 512).await?;
    parse_plan(&raw, &brief.angle)
}

fn parse_plan(raw: &str, fallback_angle: &str) -> Result<StrategyPlan, String> {
    if let Ok(v) = serde_json::from_str::<Value>(raw) {
        return Ok(StrategyPlan {
            angle: v["angle"].as_str().unwrap_or(fallback_angle).to_string(),
            key_points: v["key_points"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            outline: v["outline"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
        });
    }
    // 回退: 按行拆
    let lines: Vec<String> = raw
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    Ok(StrategyPlan {
        angle: fallback_angle.to_string(),
        key_points: lines.iter().take(5).cloned().collect(),
        outline: lines,
    })
}

/// 工序2 产出: 平台无关的 Master Content Asset (grounding + 事实核查备注)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MasterAsset {
    pub text: String,
    pub grounding: Vec<String>,
    pub fact_check: Vec<String>,
}

/// 工序2 核心创作: 把策略方案 + 信号证据 grounding 进母稿, 并要求模型列出事实核查点
pub async fn create_master(
    brief: &Brief,
    plan: &StrategyPlan,
    signals: &[Value],
    llm: &dyn TextGenerator,
) -> Result<MasterAsset, String> {
    let evidence: Vec<String> = signals
        .iter()
        .take(8)
        .filter_map(|s| {
            s.get("payload")
                .and_then(|p| p.get("title").or_else(|| p.get("url")))
                .and_then(|v| v.as_str())
                .map(String::from)
        })
        .collect();
    let prompt = format!(
        "You are the core creation step (工序2) of a content factory. Write the platform-agnostic master content.          Ground every claim in the provided signals. End with a JSON block: {{fact_check: [claims to verify]}}.          Output: master text, then the JSON block on the last line.

         Brief: goal={} angle={} audience={} cta={}
Plan: angle={} key_points={:?} outline={:?}
Signal evidence:
{}",
        brief.goal, brief.angle, brief.audience, brief.cta,
        plan.angle, plan.key_points, plan.outline, evidence.join("
")
    );
    let raw = llm.generate(&prompt, 1024).await?;
    // 拆出末行 JSON 作事实核查
    let mut fact_check = Vec::new();
    let mut text = raw.clone();
    if let Some(idx) = raw.rfind('{')
        && let Ok(v) = serde_json::from_str::<Value>(&raw[idx..])
    {
        fact_check = v["fact_check"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        text = raw[..idx].trim().to_string();
    }
    Ok(MasterAsset {
        text,
        grounding: evidence,
        fact_check,
    })
}

/// 工序3 入口: 用平台 adapter 把 Master Asset 适配成该平台变体
pub fn adapt_master(master: &MasterAsset, adapter: &dyn PlatformAdapter, brief: &Brief) -> String {
    adapter.adapt(&master.text, brief)
}

// ---------- 工序4: 媒体生产端口 (统一 Generator 端口, provider 可互换) ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaJob {
    pub job_id: String,
    pub kind: String, // image / video / audio / subtitle / transcribe
    pub source_url: Option<String>,
    pub prompt: String,
    pub spec: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaOutput {
    pub job_id: String,
    pub provider: String,
    pub asset_url: Option<String>,
    pub transcript: Option<String>,
    pub status: String,
}

/// 媒体生产 Provider 端口 (🧰 Faster-Whisper 转写/字幕 · MoneyPrinterTurbo · Pixelle-Video ·
/// Open-Generative-AI · Kling/Runway/Veo 商用视频 —— 都挂在这个端口后面, 可互换)
#[async_trait::async_trait]
pub trait MediaProvider: Send + Sync {
    fn provider_id(&self) -> &str;
    fn supported_kinds(&self) -> Vec<&'static str>;
    fn kinds_match(&self, kind: &str) -> bool {
        self.supported_kinds().contains(&kind)
    }
    async fn generate(&self, job: &MediaJob) -> Result<MediaOutput, String>;
}

/// 本地占位 provider: 不真正生成媒体, 返回 PENDING 状态 (红线: 不调外部下载站)
pub struct LocalStubProvider;

#[async_trait::async_trait]
impl MediaProvider for LocalStubProvider {
    fn provider_id(&self) -> &str {
        "local-stub"
    }
    fn supported_kinds(&self) -> Vec<&'static str> {
        vec!["image", "video", "audio", "subtitle", "transcribe"]
    }
    async fn generate(&self, job: &MediaJob) -> Result<MediaOutput, String> {
        Ok(MediaOutput {
            job_id: job.job_id.clone(),
            provider: self.provider_id().to_string(),
            asset_url: None,
            transcript: None,
            status: "PENDING".into(),
        })
    }
}

/// 外部命令行 provider (🧰 可插拔: 配置 command 即接入 Faster-Whisper / MoneyPrinterTurbo 等开源工具)
/// 形如: SINCY_MEDIA_PROVIDERS='[{"id":"faster-whisper","kinds":["transcribe","subtitle"],"cmd":"faster-whisper {source_url} --output {out}"}]'
pub struct CommandProvider {
    pub id: String,
    pub kinds: Vec<String>,
    pub cmd: String,
}

#[async_trait::async_trait]
impl MediaProvider for CommandProvider {
    fn provider_id(&self) -> &str {
        &self.id
    }
    fn supported_kinds(&self) -> Vec<&'static str> {
        // 动态 kinds, 这里返回静态占位; 真正匹配用 self.kinds
        vec!["transcribe", "subtitle", "video", "image"]
    }
    fn kinds_match(&self, kind: &str) -> bool {
        self.kinds.iter().any(|k| k == kind)
    }
    async fn generate(&self, job: &MediaJob) -> Result<MediaOutput, String> {
        Ok(MediaOutput {
            job_id: job.job_id.clone(),
            provider: self.id.clone(),
            asset_url: None,
            transcript: None,
            status: format!("QUEUED:{}", self.cmd),
        })
    }
}

// ---------- 工序9: Learn 回流 (断点②) ----------

/// 一条指标归因记录 (平台指标 → 具体变体)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricEvent {
    pub platform: String,
    pub package_id: String,
    pub variant_id: String,
    pub metric: String,
    pub value: f64,
    pub observed_at: chrono::DateTime<chrono::Utc>,
}

/// Learn 回写目标: 策略假设 / 品牌记忆
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Learning {
    pub hypothesis: String,
    pub evidence: Vec<String>,
    pub confidence: f64,
}

/// 工序9 Learn: 汇总指标 → 形成学习结论 (回写策略/品牌记忆; 断点②在此接通)
pub fn learn(events: &[MetricEvent]) -> Vec<Learning> {
    use std::collections::HashMap;
    let mut by_variant: HashMap<String, (f64, usize)> = HashMap::new();
    for e in events {
        let entry = by_variant.entry(e.variant_id.clone()).or_insert((0.0, 0));
        if e.metric == "engagement_rate" || e.metric == "ctr" || e.metric == "views" {
            entry.0 += e.value;
            entry.1 += 1;
        }
    }
    let mut learnings = Vec::new();
    for (variant, (sum, n)) in by_variant {
        if n == 0 {
            continue;
        }
        let avg = sum / n as f64;
        learnings.push(Learning {
            hypothesis: format!("variant {variant} avg metric {avg:.3}"),
            evidence: vec![format!("{n} events")],
            confidence: (n as f64 / (n as f64 + 10.0)).min(0.95),
        });
    }
    learnings
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn brief_from_opportunity_carries_seed() {
        let b = Brief::from_opportunity(
            "b1",
            "opp1",
            Some("sig1"),
            "grow",
            vec!["x".into()],
            "angle",
        );
        assert_eq!(b.trigger, TriggerSource::OpportunityDriven);
        assert_eq!(b.seed_opportunity_id.as_deref(), Some("opp1"));
        assert_eq!(b.seed_signal_id.as_deref(), Some("sig1"));
    }

    #[test]
    fn x_adapter_truncates_and_appends_hashtag() {
        let adapters = default_adapters();
        let x = adapters.iter().find(|a| a.spec().platform == "x").unwrap();
        let brief = Brief::manual("b1", "AI Agents", vec!["x".into()]);
        // 短内容: 追加 hashtag 不截断
        let out = x.adapt("hello", &brief);
        assert!(out.contains("#AIAgents"));
        assert!(out.chars().count() <= 281);
        // 超长内容: 最终截断到 max_chars
        let long = "a".repeat(500);
        let out = x.adapt(&long, &brief);
        assert!(out.chars().count() <= 281);
    }

    #[test]
    fn strategy_and_master_flow_with_mock_llm() {
        struct Mock;
        #[async_trait::async_trait]
        impl TextGenerator for Mock {
            async fn generate(&self, prompt: &str, _m: u32) -> Result<String, String> {
                if prompt.contains("工序1") {
                    Ok(r#"{"angle":"AI agents for ops","key_points":["a","b"],"outline":["hook","proof","cta"]}"#.to_string())
                } else {
                    Ok("Master text about AI agents.
{\"fact_check\":[\"statX\"]}
"
                    .to_string())
                }
            }
        }
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let brief = Brief::manual("b1", "AI agents", vec!["x".into()]);
            let signals = vec![json!({"signal_type":"trend_rising","strength":0.9,"payload":{"title":"AI agents trend"}})];
            let plan = plan_strategy(&brief, &signals, &Mock).await.unwrap();
            assert_eq!(plan.key_points.len(), 2);
            let master = create_master(&brief, &plan, &signals, &Mock).await.unwrap();
            assert!(master.text.contains("Master text"));
            assert_eq!(master.fact_check, vec!["statX".to_string()]);
            assert_eq!(master.grounding.len(), 1);
        });
    }

    #[test]
    fn learn_aggregates_variants() {
        let now = chrono::Utc::now();
        let ev = vec![
            MetricEvent {
                platform: "x".into(),
                package_id: "p".into(),
                variant_id: "v1".into(),
                metric: "ctr".into(),
                value: 0.1,
                observed_at: now,
            },
            MetricEvent {
                platform: "x".into(),
                package_id: "p".into(),
                variant_id: "v1".into(),
                metric: "ctr".into(),
                value: 0.2,
                observed_at: now,
            },
        ];
        let ls = learn(&ev);
        assert_eq!(ls.len(), 1);
        assert!((ls[0].confidence - 0.1666).abs() < 0.01);
    }
}
