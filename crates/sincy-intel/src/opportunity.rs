//! Opportunity engine (WI §19): score trends/topics against an employee's
//! content pillars and produce ranked, evidence-linked opportunities.

use crate::model::{Opportunity, Trend};
use serde_json::json;

/// Score an opportunity. Inputs:
/// - `pillar_match`: 0..1 how well the topic matches the employee's content pillars
/// - `trend`: the trend signal (volume/velocity)
/// - `competition`: 0..1 how saturated the topic already is on the channel
/// - `freshness_hours`: age of the signal
///
/// Score is a weighted sum in 0..1, with decay for stale signals.
pub fn score_opportunity(
    pillar_match: f64,
    trend: &Trend,
    competition: f64,
    freshness_hours: f64,
) -> f64 {
    let relevance = pillar_match.clamp(0.0, 1.0);
    let momentum = (trend.velocity / (trend.velocity + 1.0)).clamp(0.0, 1.0); // saturating
    let volume_norm = (trend.volume as f64 / (trend.volume as f64 + 1000.0)).clamp(0.0, 1.0);
    let scarcity = (1.0 - competition.clamp(0.0, 1.0)).clamp(0.0, 1.0);
    let freshness = (-freshness_hours / 24.0).exp().clamp(0.0, 1.0); // halves daily-ish
    (0.35 * relevance + 0.25 * momentum + 0.15 * volume_norm + 0.15 * scarcity + 0.10 * freshness)
        .clamp(0.0, 1.0)
}

/// Rank a set of candidate opportunities, highest score first, stable by topic.
pub fn rank_opportunities(mut opps: Vec<Opportunity>) -> Vec<Opportunity> {
    opps.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.topic.cmp(&b.topic))
    });
    opps
}

/// Build an Opportunity from a trend + context.
pub fn opportunity_from_trend(
    trend: &Trend,
    pillar_match: f64,
    competition: f64,
    freshness_hours: f64,
    evidence: Vec<String>,
) -> Opportunity {
    let score = score_opportunity(pillar_match, trend, competition, freshness_hours);
    Opportunity {
        topic: trend.topic.clone(),
        score,
        rationale: json!({
            "pillar_match": pillar_match,
            "velocity": trend.velocity,
            "volume": trend.volume,
            "competition": competition,
            "freshness_hours": freshness_hours,
            "platforms": trend.platforms,
        }),
        evidence,
        status: "new".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trend(topic: &str, volume: i64, velocity: f64) -> Trend {
        Trend {
            topic: topic.into(),
            platforms: vec!["x".into()],
            volume,
            velocity,
            sentiment: 0.6,
            window_hours: 24,
        }
    }

    #[test]
    fn fresher_and_better_matched_scores_higher() {
        let t = trend("ai agents", 5000, 3.0);
        let fresh = score_opportunity(0.9, &t, 0.1, 1.0);
        let stale = score_opportunity(0.9, &t, 0.1, 96.0);
        let offpillar = score_opportunity(0.1, &t, 0.1, 1.0);
        assert!(fresh > stale);
        assert!(fresh > offpillar);
    }

    #[test]
    fn ranking_is_stable_and_descending() {
        let opps = vec![
            opportunity_from_trend(&trend("a", 100, 0.5), 0.3, 0.5, 10.0, vec![]),
            opportunity_from_trend(&trend("b", 9000, 5.0), 0.9, 0.1, 1.0, vec![]),
            opportunity_from_trend(&trend("c", 500, 1.0), 0.6, 0.3, 5.0, vec![]),
        ];
        let ranked = rank_opportunities(opps);
        assert_eq!(ranked[0].topic, "b");
        assert!(ranked[0].score >= ranked[1].score && ranked[1].score >= ranked[2].score);
    }
}
