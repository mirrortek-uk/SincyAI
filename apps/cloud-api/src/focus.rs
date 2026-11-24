//! 员工领域画像 Focus (WS1/WS2/WS3 共用)。
//!
//! 设计见 实施指南「细分领域员工」: identity.focus = { scope, industry, sub_verticals,
//! topics, audience, excluded_topics }。相关性是 per-员工现算的 (原则1),
//! scope=general 放行不过滤 (原则3)。

use serde_json::{Value, json};
use sincy_intel::industry::{CustomerProfile, taxonomy};

#[derive(Debug, Clone, Default)]
pub struct Focus {
    pub scope: String, // "niche" | "general"
    pub industry: String,
    pub sub_verticals: Vec<String>,
    pub topics: Vec<String>,
    pub audience: Vec<String>,
    pub excluded_topics: Vec<String>,
}

/// 全部分类法 industry id (供校验 + 前端下拉)。
pub fn industry_ids() -> Vec<String> {
    taxonomy().iter().map(|d| d.id.clone()).collect()
}

pub fn focus_from_identity(identity: &Value) -> Focus {
    focus_from_value(identity.get("focus").cloned().unwrap_or(Value::Null))
}

pub fn focus_from_value(v: Value) -> Focus {
    let g = |k: &str| -> Vec<String> {
        v.get(k)
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    };
    Focus {
        scope: v
            .get("scope")
            .and_then(|x| x.as_str())
            .unwrap_or("general")
            .to_string(),
        industry: v
            .get("industry")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        sub_verticals: g("sub_verticals"),
        topics: g("topics"),
        audience: g("audience"),
        excluded_topics: g("excluded_topics"),
    }
}

/// 校验: scope=niche 时 industry 必填且必须在分类法内。返回 Err(给 422 用)。
pub fn validate_focus(v: &Value) -> Result<(), String> {
    let f = focus_from_value(v.clone());
    let scope = v.get("scope").and_then(|x| x.as_str()).unwrap_or("general");
    if scope != "niche" && scope != "general" {
        return Err(format!(
            "focus.scope must be 'niche' or 'general', got '{scope}'"
        ));
    }
    if scope == "niche" {
        if f.industry.is_empty() {
            return Err("focus.industry is required when scope=niche".into());
        }
        if !industry_ids().iter().any(|i| i == &f.industry) {
            return Err(format!(
                "focus.industry '{}' not in taxonomy ({})",
                f.industry,
                industry_ids().join(", ")
            ));
        }
    }
    Ok(())
}

impl Focus {
    pub fn is_general(&self) -> bool {
        self.scope != "niche"
    }

    /// 硬过滤关键词集合: industry + sub_verticals (小写)
    pub fn keys(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        if !self.industry.is_empty() {
            v.push(self.industry.to_lowercase());
        }
        for s in &self.sub_verticals {
            v.push(s.to_lowercase());
        }
        v
    }

    /// 标签是否与领域相交 (含分类法父子行业)。tags 为空 = 无信息, 放行。
    pub fn tags_intersect(&self, tags: &[String]) -> bool {
        if self.is_general() {
            return true;
        }
        if tags.is_empty() {
            return true;
        }
        let keys = self.keys();
        let tax = taxonomy();
        tags.iter().any(|t| {
            let tl = t.to_lowercase();
            if keys.contains(&tl) {
                return true;
            }
            // 父行业算命中: tag 的父行业在 keys 里, 或 keys 里 industry 的父 == tag
            if let Some(def) = tax.iter().find(|d| d.id.to_lowercase() == tl)
                && let Some(p) = &def.parent_id
                && keys.contains(&p.to_lowercase())
            {
                return true;
            }
            if let Some(def) = tax
                .iter()
                .find(|d| d.id.to_lowercase() == self.industry.to_lowercase())
                && let Some(p) = &def.parent_id
                && p.to_lowercase() == tl
            {
                return true;
            }
            false
        })
    }

    /// 负向过滤: 文本命中 excluded_topics (大小写不敏感)
    pub fn excluded_hit(&self, text: &str) -> bool {
        let lower = text.to_lowercase();
        self.excluded_topics
            .iter()
            .any(|e| !e.is_empty() && lower.contains(&e.to_lowercase()))
    }

    /// 文本关键词命中 (topics/sub_verticals 任一命中即相关, 用于无 industry_tags 时的软匹配)
    pub fn keyword_hit(&self, text: &str) -> bool {
        if self.is_general() {
            return true;
        }
        let lower = text.to_lowercase();
        self.topics
            .iter()
            .chain(self.sub_verticals.iter())
            .any(|t| !t.is_empty() && lower.contains(&t.to_lowercase()))
    }

    /// 编译成相关性引擎的 CustomerProfile (WS2: 填了它 relevance_score 才有真实输入)
    pub fn customer_profile(&self) -> CustomerProfile {
        CustomerProfile {
            industry: self.industry.clone(),
            sub_industry: self.sub_verticals.join(", "),
            topics: self.topics.clone(),
            keywords: self.sub_verticals.clone(),
            target_audience: self.audience.join(", "),
            ..Default::default()
        }
    }

    /// 用 focus 填满 pillars (topics + sub_verticals), 解决空 pillars 中性 0.5 问题
    pub fn pillars(&self) -> Vec<String> {
        let mut v = self.topics.clone();
        v.extend(self.sub_verticals.iter().cloned());
        if !self.industry.is_empty() {
            v.push(self.industry.clone());
        }
        v
    }

    pub fn to_json(&self) -> Value {
        json!({
            "scope": self.scope,
            "industry": self.industry,
            "sub_verticals": self.sub_verticals,
            "topics": self.topics,
            "audience": self.audience,
            "excluded_topics": self.excluded_topics,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn niche() -> Value {
        json!({
            "scope": "niche",
            "industry": "ai",
            "sub_verticals": ["llm_ops", "vector_db"],
            "topics": ["RAG", "推理优化"],
            "audience": ["AI 工程师"],
            "excluded_topics": ["加密币炒作"],
        })
    }

    #[test]
    fn validate_requires_industry_for_niche() {
        assert!(validate_focus(&niche()).is_ok());
        let bad = json!({"scope": "niche"});
        assert!(validate_focus(&bad).is_err());
        let unknown = json!({"scope": "niche", "industry": "not_a_industry"});
        assert!(validate_focus(&unknown).is_err());
        assert!(validate_focus(&json!({"scope": "general"})).is_ok());
        assert!(validate_focus(&json!({"scope": "weird"})).is_err());
    }

    #[test]
    fn tags_intersect_with_parent_industry() {
        let f = focus_from_value(niche());
        assert!(f.tags_intersect(&["ai".into()]));
        assert!(f.tags_intersect(&["technology".into()])); // ai 的父行业
        assert!(f.tags_intersect(&[])); // 无信息放行
        assert!(!f.tags_intersect(&["gaming".into()]));
        let g = focus_from_value(json!({"scope": "general"}));
        assert!(g.tags_intersect(&["gaming".into()])); // general 全放行
    }

    #[test]
    fn excluded_and_keyword_hit() {
        let f = focus_from_value(niche());
        assert!(f.excluded_hit("加密币炒作又来了"));
        assert!(!f.excluded_hit("RAG 优化新进展"));
        assert!(f.keyword_hit("RAG pipeline 怎么搭"));
        assert!(!f.keyword_hit("体育八卦"));
    }

    #[test]
    fn customer_profile_maps_fields() {
        let f = focus_from_value(niche());
        let cp = f.customer_profile();
        assert_eq!(cp.industry, "ai");
        assert!(cp.topics.contains(&"RAG".to_string()));
        assert!(f.pillars().contains(&"llm_ops".to_string()));
    }
}
