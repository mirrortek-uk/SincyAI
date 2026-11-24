//! Digital Employee intelligence adapter (WI §20): converts global
//! intelligence into employee-specific context (research brief). The adapter
//! never changes strategy and never makes the final content decision.

use crate::model::Opportunity;
use serde::{Deserialize, Serialize};

/// Employee context the adapter needs (from identity/mission/strategy).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmployeeContext {
    pub employee_id: String,
    pub mission: String,
    pub audience: Vec<String>,
    /// content pillars from the active strategy
    pub pillars: Vec<String>,
    pub channel: String,
    /// recent topics already posted (anti-template, DE §10)
    pub recent_topics: Vec<String>,
    pub max_cost_usd_per_cycle: Option<f64>,
    /// 领域画像 focus (identity.focus, WS1); 相关性路由据此现算 per-员工 relevance
    #[serde(default)]
    pub focus: Option<serde_json::Value>,
}

/// Research brief contract (DE §25.7).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmployeeBrief {
    pub employee_id: String,
    pub question: String,
    pub scope: String,
    pub opportunities: Vec<Opportunity>,
    pub evidence: Vec<String>,
    pub confidence: f64,
    pub suggested_action: String,
    pub limitations: Vec<String>,
}

/// Pillar match: fraction of pillar keywords present in the topic, plus a
/// small bonus if the topic is not in recent_topics (anti-repetition).
pub fn pillar_match(topic: &str, pillars: &[String], recent_topics: &[String]) -> f64 {
    if pillars.is_empty() {
        return 0.5;
    }
    let t = topic.to_lowercase();
    let hits = pillars
        .iter()
        .filter(|p| {
            let p = p.to_lowercase();
            !p.is_empty() && t.contains(&p)
        })
        .count();
    let mut m = hits as f64 / pillars.len() as f64;
    if !recent_topics.iter().any(|r| r.to_lowercase() == t) {
        m = (m + 0.1).min(1.0);
    }
    m
}

/// Adapt ranked opportunities into an employee research brief.
pub fn adapt_for_employee(
    ctx: &EmployeeContext,
    opportunities: Vec<Opportunity>,
    question: &str,
) -> EmployeeBrief {
    let confidence = if opportunities.is_empty() {
        0.0
    } else {
        opportunities.iter().map(|o| o.score).sum::<f64>() / opportunities.len() as f64
    };
    let evidence: Vec<String> = opportunities
        .iter()
        .flat_map(|o| o.evidence.clone())
        .collect();
    let suggested_action = match opportunities.first() {
        Some(o) => format!(
            "create content on '{}' (score {:.2}) for channel {}",
            o.topic, o.score, ctx.channel
        ),
        None => "no actionable opportunity; continue monitoring".to_string(),
    };
    EmployeeBrief {
        employee_id: ctx.employee_id.clone(),
        question: question.to_string(),
        scope: format!("channel={} pillars={:?}", ctx.channel, ctx.pillars),
        opportunities,
        evidence,
        confidence,
        suggested_action,
        limitations: vec![
            "signals are point-in-time; verify before publishing".into(),
            "adapter does not decide strategy".into(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn brief_ranks_and_suggests() {
        let ctx = EmployeeContext {
            employee_id: "e1".into(),
            mission: "build AI authority".into(),
            audience: vec!["developers".into()],
            pillars: vec!["ai agents".into(), "llm ops".into()],
            channel: "x".into(),
            recent_topics: vec!["ai agents".into()],
            max_cost_usd_per_cycle: None,
            focus: None,
        };
        let opps = vec![
            Opportunity {
                topic: "ai agents".into(),
                score: 0.8,
                rationale: json!({}),
                evidence: vec!["u1".into()],
                status: "new".into(),
            },
            Opportunity {
                topic: "llm ops".into(),
                score: 0.6,
                rationale: json!({}),
                evidence: vec!["u2".into()],
                status: "new".into(),
            },
        ];
        let brief = adapt_for_employee(&ctx, opps, "what to post?");
        assert_eq!(brief.opportunities.len(), 2);
        assert!(brief.suggested_action.contains("ai agents"));
        assert_eq!(brief.evidence, vec!["u1", "u2"]);
        // repeated topic gets no freshness bonus
        assert!(
            pillar_match("ai agents", &ctx.pillars, &ctx.recent_topics)
                < pillar_match("llm ops", &ctx.pillars, &ctx.recent_topics)
        );
    }
}
