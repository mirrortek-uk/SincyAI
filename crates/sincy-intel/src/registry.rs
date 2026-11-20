//! Source + provider registries (WI §6 source registry, §7 connector metadata,
//! §26 deployment model). In-memory core; cloud-api persists to intel_sources /
//! intel_providers tables.

use crate::model::{AccessMethod, Source};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provider {
    pub id: String,
    pub source_id: String,
    pub name: String,
    pub capabilities: Vec<String>,
    pub platforms: Vec<String>,
    /// cost per 1k requests in USD (0 = free)
    pub cost_per_1k: f64,
    /// remaining rate-limit budget for the current window
    pub rate_budget_left: i64,
    pub rate_budget_max: i64,
    pub health: Health,
    pub reliability: f64, // 0..1 historical success rate
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Health {
    Healthy,
    Degraded,
    Down,
    Unknown,
}

impl Health {
    pub fn score(&self) -> f64 {
        match self {
            Health::Healthy => 1.0,
            Health::Degraded => 0.4,
            Health::Down => 0.0,
            Health::Unknown => 0.5,
        }
    }
}

#[derive(Debug, Default)]
pub struct SourceRegistry {
    sources: Vec<Source>,
}

impl SourceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, source: Source) {
        self.sources.retain(|s| s.id != source.id);
        self.sources.push(source);
    }

    pub fn get(&self, id: &str) -> Option<&Source> {
        self.sources.iter().find(|s| s.id == id)
    }

    pub fn all(&self) -> &[Source] {
        &self.sources
    }

    /// sources that expose a capability (WI §25.4 capability catalog)
    pub fn with_capability(&self, capability: &str) -> Vec<&Source> {
        self.sources
            .iter()
            .filter(|s| s.enabled && s.capabilities.iter().any(|c| c == capability))
            .collect()
    }
}

#[derive(Debug, Default)]
pub struct ProviderRegistry {
    providers: Vec<Provider>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, p: Provider) {
        self.providers.retain(|x| x.id != p.id);
        self.providers.push(p);
    }

    pub fn record_health(&mut self, provider_id: &str, health: Health) {
        if let Some(p) = self.providers.iter_mut().find(|p| p.id == provider_id) {
            p.health = health;
        }
    }

    pub fn consume_budget(&mut self, provider_id: &str, cost: i64) {
        if let Some(p) = self.providers.iter_mut().find(|p| p.id == provider_id) {
            p.rate_budget_left = (p.rate_budget_left - cost).max(0);
        }
    }

    /// candidate providers for a capability+platform, enabled and not down
    pub fn candidates(&self, capability: &str, platform: Option<&str>) -> Vec<&Provider> {
        self.providers
            .iter()
            .filter(|p| {
                p.enabled
                    && p.health != Health::Down
                    && p.capabilities.iter().any(|c| c == capability)
                    && p.rate_budget_left > 0
                    && platform
                        .map(|pl| p.platforms.is_empty() || p.platforms.iter().any(|x| x == pl))
                        .unwrap_or(true)
            })
            .collect()
    }

    pub fn all(&self) -> &[Provider] {
        &self.providers
    }
}

/// Seed the registry with the WI guide's evaluated integrations as *registered
/// sources* (metadata only — actual connector processes live in the WI
/// connector layer, never in the employee domain; DE §25.9).
pub fn seed_sources(reg: &mut SourceRegistry) {
    let mk = |id: &str, name: &str, cat: &str, caps: &[&str], platforms: &[&str]| Source {
        id: id.into(),
        name: name.into(),
        category: cat.into(),
        access_method: AccessMethod::ExternalService,
        capabilities: caps.iter().map(|s| s.to_string()).collect(),
        platforms: platforms.iter().map(|s| s.to_string()).collect(),
        enabled: true,
    };
    reg.register(mk(
        "rsshub",
        "RSSHub",
        "rss",
        &["rss_research", "news_research"],
        &[],
    ));
    reg.register(mk("searxng", "SearXNG", "search", &["web_search"], &[]));
    reg.register(mk(
        "trendradar",
        "TrendRadar",
        "trend",
        &["trend_search"],
        &["weibo", "zhihu", "douyin", "xiaohongshu", "bilibili"],
    ));
    reg.register(mk(
        "hotrank-hub",
        "HotRank Hub",
        "rank",
        &["trend_search", "creator_research"],
        &["weibo", "zhihu", "bilibili", "douyin"],
    ));
    reg.register(mk(
        "reach-mcp",
        "reach-mcp",
        "social",
        &["social_search", "x_research", "reddit_research"],
        &["x", "reddit", "youtube", "bilibili"],
    ));
    reg.register(mk(
        "tikhub",
        "TikHub",
        "social",
        &[
            "social_search",
            "douyin_research",
            "xiaohongshu_research",
            "creator_research",
        ],
        &["douyin", "xiaohongshu", "tiktok"],
    ));
    reg.register(mk(
        "firecrawl",
        "Firecrawl",
        "web",
        &["web_extraction"],
        &[],
    ));
}
