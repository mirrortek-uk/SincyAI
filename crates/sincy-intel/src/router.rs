//! Provider router (WI §8): score candidates on freshness/reliability/cost/
//! rate-limit/health and produce an ordered fallback chain (WI §8 fallback).

use crate::registry::{Provider, ProviderRegistry};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouterRequest {
    pub capability: String,
    pub platform: Option<String>,
    pub query_type: Option<String>,
    pub max_cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteDecision {
    /// ordered fallback chain; empty = no provider available
    pub chain: Vec<String>,
    pub reason: String,
}

/// Provider score (WI §8): health dominates, then reliability, then cost and
/// rate-limit headroom. Deterministic tie-break by id for stable tests.
fn score(p: &Provider, max_cost_usd: Option<f64>) -> f64 {
    let cost_per_call = p.cost_per_1k / 1000.0;
    if let Some(max) = max_cost_usd
        && cost_per_call > max
    {
        return -1.0; // over budget: excluded
    }
    let headroom = if p.rate_budget_max > 0 {
        p.rate_budget_left as f64 / p.rate_budget_max as f64
    } else {
        1.0
    };
    p.health.score() * 0.45
        + p.reliability * 0.30
        + headroom * 0.15
        + (1.0 - (cost_per_call / (cost_per_call + 0.01)).min(1.0)) * 0.10
}

pub struct Router<'a> {
    pub registry: &'a ProviderRegistry,
}

impl<'a> Router<'a> {
    pub fn new(registry: &'a ProviderRegistry) -> Self {
        Self { registry }
    }

    /// Route a request to an ordered fallback chain of provider ids.
    pub fn route(&self, req: &RouterRequest) -> RouteDecision {
        let mut cands = self
            .registry
            .candidates(&req.capability, req.platform.as_deref());
        if cands.is_empty() {
            return RouteDecision {
                chain: vec![],
                reason: format!("no enabled provider for capability {}", req.capability),
            };
        }
        let mut scored: Vec<(f64, &Provider)> = cands
            .drain(..)
            .map(|p| (score(p, req.max_cost_usd), p))
            .filter(|(s, _)| *s >= 0.0)
            .collect();
        scored.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.1.id.cmp(&b.1.id))
        });
        let chain: Vec<String> = scored.iter().map(|(_, p)| p.id.clone()).collect();
        RouteDecision {
            chain,
            reason: format!("{} candidate(s) for {}", scored.len(), req.capability),
        }
    }
}

/// Convenience free function.
pub fn route(registry: &ProviderRegistry, req: &RouterRequest) -> RouteDecision {
    Router::new(registry).route(req)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Health, Provider};

    fn prov(id: &str, health: Health, rel: f64, cost: f64, left: i64, max: i64) -> Provider {
        Provider {
            id: id.into(),
            source_id: "s".into(),
            name: id.into(),
            capabilities: vec!["social_search".into()],
            platforms: vec!["x".into()],
            cost_per_1k: cost * 1000.0,
            rate_budget_left: left,
            rate_budget_max: max,
            health,
            reliability: rel,
            enabled: true,
        }
    }

    #[test]
    fn healthy_cheap_beats_degraded_expensive() {
        let mut reg = ProviderRegistry::new();
        reg.register(prov("degraded", Health::Degraded, 0.99, 0.0, 100, 100));
        reg.register(prov("healthy", Health::Healthy, 0.9, 0.001, 100, 100));
        reg.register(prov("down", Health::Down, 1.0, 0.0, 100, 100));
        let d = route(
            &reg,
            &RouterRequest {
                capability: "social_search".into(),
                platform: Some("x".into()),
                query_type: None,
                max_cost_usd: None,
            },
        );
        assert_eq!(d.chain, vec!["healthy".to_string(), "degraded".to_string()]);
    }

    #[test]
    fn over_budget_and_exhausted_are_excluded() {
        let mut reg = ProviderRegistry::new();
        reg.register(prov("pricey", Health::Healthy, 0.9, 5.0, 100, 100));
        reg.register(prov("exhausted", Health::Healthy, 0.9, 0.0, 0, 100));
        let d = route(
            &reg,
            &RouterRequest {
                capability: "social_search".into(),
                platform: None,
                query_type: None,
                max_cost_usd: Some(0.01),
            },
        );
        assert!(d.chain.is_empty());
    }
}
