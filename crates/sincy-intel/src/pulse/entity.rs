//! 统一 Intelligence Entity + Observation (Sincy Pulse §5):
//! Raw Provider Data → Unified Intelligence, 而不是 Raw → Database。
//!
//! Entity 保存聚合指标 (估计值 + 范围 + 跨源分 + 置信度),
//! 每条原始观测保留在 Observation 里, 可审计、可回溯。

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// 一条第三方原始观测
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    pub provider_id: String,
    pub dataset_id: String,
    pub metric: String,
    pub value: Value,
}

/// 一个待聚合的实体 (归一化产出)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntelligenceEntity {
    pub entity_type: String,
    pub platform: String,
    pub canonical_key: String,
    pub title: String,
    pub payload: Value,
    pub observations: Vec<Observation>,
}

impl IntelligenceEntity {
    pub fn new(entity_type: &str, platform: &str, canonical_key: &str, title: &str) -> Self {
        Self {
            entity_type: entity_type.into(),
            platform: platform.into(),
            canonical_key: canonical_key.into(),
            title: title.into(),
            payload: json!({}),
            observations: Vec::new(),
        }
    }

    pub fn observe(&mut self, provider_id: &str, dataset_id: &str, metric: &str, value: Value) {
        self.observations.push(Observation {
            provider_id: provider_id.into(),
            dataset_id: dataset_id.into(),
            metric: metric.into(),
            value,
        });
    }
}

/// 聚合结果: 写入 pulse_entities 的 metrics/trend/confidence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregateMetrics {
    /// 每个数值 metric: {estimated, min, max, count}
    pub metrics: Value,
    pub cross_source_score: f64,
    pub provider_count: usize,
    pub confidence: Value,
}

/// 跨源聚合 (设计 §5 例子): 同一实体多条数值观测 → 估计值(均值) + 范围 + 跨源分。
/// provider_reliability: provider_id → reliability (来自 pulse_providers.reliability)。
pub fn aggregate(
    entity_type: &str,
    observations: &[Observation],
    provider_reliability: &dyn Fn(&str) -> f64,
) -> AggregateMetrics {
    use std::collections::BTreeMap;
    let mut numeric: BTreeMap<String, Vec<(f64, f64)>> = BTreeMap::new(); // metric -> [(value, weight)]
    let mut providers: Vec<&str> = Vec::new();
    for o in observations {
        if !providers.contains(&o.provider_id.as_str()) {
            providers.push(&o.provider_id);
        }
        if let Some(n) = o.value.as_f64() {
            numeric
                .entry(o.metric.clone())
                .or_default()
                .push((n, provider_reliability(&o.provider_id)));
        }
    }
    let mut metrics = json!({});
    for (metric, vals) in &numeric {
        let wsum: f64 = vals.iter().map(|(_, w)| w).sum();
        let estimated = if wsum > 0.0 {
            vals.iter().map(|(v, w)| v * w).sum::<f64>() / wsum
        } else {
            vals.iter().map(|(v, _)| v).sum::<f64>() / vals.len() as f64
        };
        let min = vals.iter().map(|(v, _)| *v).fold(f64::INFINITY, f64::min);
        let max = vals
            .iter()
            .map(|(v, _)| *v)
            .fold(f64::NEG_INFINITY, f64::max);
        metrics[metric.clone()] = json!({
            "estimated": estimated,
            "range": {"min": min, "max": max},
            "count": vals.len(),
        });
    }
    let provider_count = providers.len();
    // 跨源分: 参与源越多、可靠性越高, 分越高 (饱和)
    let avg_rel: f64 = if provider_count > 0 {
        providers
            .iter()
            .map(|p| provider_reliability(p))
            .sum::<f64>()
            / provider_count as f64
    } else {
        0.0
    };
    let cross_source_score =
        (100.0 * (1.0 - (1.0 / (1.0 + provider_count as f64))) * (0.5 + 0.5 * avg_rel)).min(100.0);
    // 置信度: 源数量 + 平均可靠性
    let conf_score = ((provider_count as f64 / (provider_count as f64 + 2.0)) * 0.6
        + avg_rel * 0.4)
        .clamp(0.0, 1.0);
    let level = if conf_score >= 0.75 {
        "high"
    } else if conf_score >= 0.45 {
        "medium"
    } else {
        "low"
    };
    let _ = entity_type;
    AggregateMetrics {
        metrics,
        cross_source_score,
        provider_count,
        confidence: json!({"score": conf_score, "level": level}),
    }
}

/// 趋势分类 (Phase 3 的轻量版): 按 velocity 分桶
pub fn classify_trend(velocity_per_h: f64) -> &'static str {
    if velocity_per_h >= 1000.0 {
        "viral"
    } else if velocity_per_h >= 100.0 {
        "rising"
    } else if velocity_per_h >= 10.0 {
        "emerging"
    } else {
        "stable"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(provider: &str, metric: &str, v: f64) -> Observation {
        Observation {
            provider_id: provider.into(),
            dataset_id: "d".into(),
            metric: metric.into(),
            value: json!(v),
        }
    }

    #[test]
    fn merges_multi_source_views_into_range_and_estimate() {
        // 设计 §5 例子: 1.2M / 1.5M → estimated 1.35M, range [1.2M, 1.5M]
        let obs = vec![
            obs("xbangdan", "views", 1_200_000.0),
            obs("xpaiming", "views", 1_500_000.0),
        ];
        let agg = aggregate("social_post", &obs, &|_| 0.8);
        assert_eq!(agg.provider_count, 2);
        assert_eq!(agg.metrics["views"]["range"]["min"], json!(1_200_000.0));
        assert_eq!(agg.metrics["views"]["range"]["max"], json!(1_500_000.0));
        assert!((agg.metrics["views"]["estimated"].as_f64().unwrap() - 1_350_000.0).abs() < 1.0);
        assert!(agg.cross_source_score > 50.0);
        assert_eq!(agg.confidence["level"], "medium");
    }

    #[test]
    fn single_source_has_low_confidence() {
        let obs = vec![obs("one", "views", 100.0)];
        let agg = aggregate("social_post", &obs, &|_| 0.3);
        assert_eq!(agg.provider_count, 1);
        assert_eq!(agg.confidence["level"], "low");
    }

    #[test]
    fn trend_classification_buckets() {
        assert_eq!(classify_trend(5000.0), "viral");
        assert_eq!(classify_trend(500.0), "rising");
        assert_eq!(classify_trend(50.0), "emerging");
        assert_eq!(classify_trend(1.0), "stable");
    }
}
