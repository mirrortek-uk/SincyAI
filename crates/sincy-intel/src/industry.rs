//! Sincy-04 行业情报与客户相关性: 行业树 / 规则分类器 / 客户画像相关性评分 (v1 规则版)。
//!
//! 设计原则 (Sincy-04 §5): Source 不绑定行业, 绑定 Capability;
//! 同一条情报对不同行业客户价值不同 —— 由 Industry Classification + Relevance Engine 决定。

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 行业定义 (与 migration 0033 种子一致, 代码内为单一事实源)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndustryDef {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    pub level: u8,
    #[serde(default)]
    pub keywords: Vec<String>,
}

/// 内置行业树 (Sincy-04 §8)
pub fn taxonomy() -> Vec<IndustryDef> {
    #[allow(clippy::type_complexity)]
    let rows: &[(&str, &str, Option<&str>, u8, &[&str])] = &[
        (
            "technology",
            "科技",
            None,
            1,
            &[
                "technology",
                "tech",
                "software",
                "app",
                "startup",
                "程序",
                "科技",
                "软件",
                "互联网",
                "人工智能",
            ],
        ),
        (
            "ai",
            "人工智能",
            Some("technology"),
            2,
            &[
                "ai",
                "llm",
                "gpt",
                "agent",
                "machine learning",
                "aigc",
                "生成式",
                "智能体",
                "大模型",
            ],
        ),
        (
            "saas",
            "SaaS",
            Some("technology"),
            2,
            &["saas", "subscription", "b2b software", "订阅软件"],
        ),
        (
            "cybersecurity",
            "网络安全",
            Some("technology"),
            2,
            &["cybersecurity", "security", "漏洞", "黑客", "安全"],
        ),
        (
            "cloud",
            "云计算",
            Some("technology"),
            2,
            &["cloud", "aws", "azure", "kubernetes", "云服务", "云原生"],
        ),
        (
            "finance",
            "金融",
            None,
            1,
            &[
                "finance",
                "bank",
                "fintech",
                "investment",
                "stock",
                "crypto",
                "经济",
                "金融",
                "银行",
                "股市",
                "加密",
            ],
        ),
        (
            "fintech",
            "金融科技",
            Some("finance"),
            2,
            &["fintech", "支付", "数字银行", "payment"],
        ),
        (
            "investment",
            "投资",
            Some("finance"),
            2,
            &["investment", "vc", "pe", "fund", "投资", "融资", "基金"],
        ),
        (
            "healthcare",
            "医疗健康",
            None,
            1,
            &[
                "healthcare",
                "medical",
                "hospital",
                "pharma",
                "医疗",
                "医院",
                "医药",
                "健康",
                "医疗器械",
            ],
        ),
        (
            "pharma",
            "医药",
            Some("healthcare"),
            2,
            &["pharma", "drug", "biotech", "药物", "生物医药", "制药"],
        ),
        (
            "wellness",
            "大健康",
            Some("healthcare"),
            2,
            &["wellness", "fitness", "养生", "健身"],
        ),
        (
            "retail",
            "零售电商",
            None,
            1,
            &[
                "retail",
                "ecommerce",
                "e-commerce",
                "shop",
                "电商",
                "零售",
                "购物",
                "品牌",
            ],
        ),
        (
            "fashion",
            "时尚",
            Some("retail"),
            2,
            &["fashion", "apparel", "服装", "穿搭", "潮牌"],
        ),
        (
            "beauty",
            "美妆",
            Some("retail"),
            2,
            &[
                "beauty",
                "skincare",
                "makeup",
                "cosmetics",
                "美妆",
                "护肤",
                "彩妆",
            ],
        ),
        (
            "food",
            "食品餐饮",
            Some("retail"),
            2,
            &[
                "food",
                "restaurant",
                "catering",
                "餐饮",
                "食品",
                "美食",
                "餐厅",
            ],
        ),
        (
            "real_estate",
            "房地产",
            None,
            1,
            &[
                "real estate",
                "property",
                "housing",
                "mortgage",
                "房地产",
                "房产",
                "楼市",
                "地产",
            ],
        ),
        (
            "education",
            "教育",
            None,
            1,
            &[
                "education",
                "edtech",
                "learning",
                "school",
                "教育",
                "培训",
                "学校",
                "课程",
            ],
        ),
        (
            "edtech",
            "教育科技",
            Some("education"),
            2,
            &["edtech", "online course", "mooc", "在线教育"],
        ),
        (
            "manufacturing",
            "制造业",
            None,
            1,
            &[
                "manufacturing",
                "factory",
                "supply chain",
                "制造",
                "工厂",
                "供应链",
            ],
        ),
        (
            "automotive",
            "汽车",
            None,
            1,
            &[
                "automotive",
                "car",
                "ev",
                "tesla",
                "汽车",
                "新能源车",
                "电动车",
            ],
        ),
        (
            "travel",
            "旅游酒店",
            None,
            1,
            &["travel", "hotel", "tourism", "旅游", "酒店", "出行"],
        ),
        (
            "media",
            "媒体娱乐",
            None,
            1,
            &[
                "media",
                "entertainment",
                "game",
                "gaming",
                "film",
                "媒体",
                "游戏",
                "娱乐",
                "影视",
            ],
        ),
        (
            "gaming",
            "游戏",
            Some("media"),
            2,
            &["game", "gaming", "esports", "手游", "游戏"],
        ),
        (
            "professional_services",
            "专业服务",
            None,
            1,
            &[
                "legal",
                "consulting",
                "accounting",
                "law firm",
                "律所",
                "咨询",
                "会计",
                "法律",
            ],
        ),
        (
            "marketing",
            "营销广告",
            None,
            1,
            &[
                "marketing",
                "advertising",
                "brand",
                "seo",
                "营销",
                "广告",
                "品牌营销",
            ],
        ),
    ];
    rows.iter()
        .map(|(id, name, parent, level, kws)| IndustryDef {
            id: id.to_string(),
            name: name.to_string(),
            parent_id: parent.map(|s| s.to_string()),
            level: *level,
            keywords: kws.iter().map(|s| s.to_string()).collect(),
        })
        .collect()
}

/// 规则分类: 文本 → [(industry_id, score 0..1)], 按命中关键词数归一化, 只返回 >0 的, 最多 5 个。
/// 命中子行业时同时给父行业加分 (父行业覆盖更广)。
pub fn classify_industries(text: &str) -> Vec<(String, f64)> {
    let lower = text.to_lowercase();
    let mut scores: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    for def in taxonomy() {
        let hits = def
            .keywords
            .iter()
            .filter(|k| lower.contains(&k.to_lowercase()))
            .count() as f64;
        if hits > 0.0 {
            let s =
                (hits / (def.keywords.len() as f64).max(1.0)).min(1.0) * 0.5 + hits.min(3.0) / 6.0;
            *scores.entry(def.id.clone()).or_insert(0.0) += s.min(1.0);
            if let Some(p) = &def.parent_id {
                *scores.entry(p.clone()).or_insert(0.0) += s.min(1.0) * 0.5;
            }
        }
    }
    let mut v: Vec<(String, f64)> = scores.into_iter().collect();
    v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    v.truncate(5);
    v
}

/// Customer Intelligence Profile (Sincy-04 §9), 存于 brand_intel_profiles.profile
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CustomerProfile {
    #[serde(default)]
    pub industry: String,
    #[serde(default)]
    pub sub_industry: String,
    #[serde(default)]
    pub business_model: String,
    #[serde(default)]
    pub target_audience: String,
    #[serde(default)]
    pub geography: Vec<String>,
    #[serde(default)]
    pub competitors: Vec<String>,
    #[serde(default)]
    pub products: Vec<String>,
    #[serde(default)]
    pub topics: Vec<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub business_goals: Vec<String>,
    #[serde(default)]
    pub brand_strategy: String,
}

impl CustomerProfile {
    pub fn from_json(v: &Value) -> Self {
        serde_json::from_value(v.clone()).unwrap_or_default()
    }
    /// 该画像声明的行业集合 (一级 + 子行业)
    pub fn industries(&self) -> Vec<String> {
        let mut v = Vec::new();
        if !self.industry.is_empty() {
            v.push(self.industry.clone());
        }
        if !self.sub_industry.is_empty() {
            v.push(self.sub_industry.clone());
        }
        v
    }
}

/// Relevance Score v1 (Sincy-04 §10.2 权重):
/// Industry 30% + Topic 25% + Competitor 20% + Audience 15% + Geography 10%
pub fn relevance_score(
    item_industries: &[String],
    item_text: &str,
    profile: &CustomerProfile,
) -> f64 {
    // Industry match: 命中画像行业 (含父行业) 比例
    let want = profile.industries();
    let ind = if want.is_empty() {
        0.5 // 未设置画像时中性
    } else {
        let hit = item_industries.iter().filter(|i| want.contains(i)).count() as f64;
        // 父行业也算半分
        let tax = taxonomy();
        let mut half = 0.0f64;
        for i in item_industries {
            if let Some(def) = tax.iter().find(|d| d.id == *i)
                && let Some(p) = &def.parent_id
                && want.contains(p)
            {
                half += 0.5;
            }
        }
        ((hit + half) / want.len() as f64).min(1.0)
    };
    // Topic match: 画像 topics/keywords 命中条目文本
    let lower = item_text.to_lowercase();
    let pool: Vec<&str> = profile
        .topics
        .iter()
        .chain(profile.keywords.iter())
        .map(|s| s.as_str())
        .collect();
    let topic = if pool.is_empty() {
        0.5
    } else {
        let hit = pool
            .iter()
            .filter(|k| lower.contains(&k.to_lowercase()))
            .count() as f64;
        (hit / pool.len() as f64).min(1.0)
    };
    // Competitor match: 条目文本提到竞争对手
    let comp = if profile.competitors.is_empty() {
        0.5
    } else {
        let hit = profile
            .competitors
            .iter()
            .filter(|c| lower.contains(&c.to_lowercase()))
            .count() as f64;
        (hit / profile.competitors.len() as f64).min(1.0)
    };
    // Audience match: 目标受众关键词命中
    let aud = if profile.target_audience.is_empty() {
        0.5
    } else {
        let hit = profile
            .target_audience
            .split([',', '，', ' '])
            .map(|s| s.trim())
            .filter(|s| !s.is_empty() && lower.contains(&s.to_lowercase()))
            .count() as f64;
        let total = profile
            .target_audience
            .split([',', '，', ' '])
            .filter(|s| !s.trim().is_empty())
            .count()
            .max(1) as f64;
        (hit / total).min(1.0)
    };
    // Geography match: 画像市场命中条目文本中的地区关键词 (粗粒度: 全球/中国/美国/欧洲)
    let geo = if profile.geography.is_empty() {
        0.5
    } else {
        let geo_kw: &[(&str, &[&str])] = &[
            ("global", &["global", "worldwide", "全球"]),
            ("china", &["china", "chinese", "中国"]),
            ("us", &["united states", "usa", "america", "美国"]),
            ("europe", &["europe", "eu", "欧洲"]),
            ("uk", &["uk", "britain", "英国"]),
        ];
        let mut hit = 0.0f64;
        for g in &profile.geography {
            let gl = g.to_lowercase();
            if let Some((_, kws)) = geo_kw.iter().find(|(k, _)| gl.contains(k)) {
                if kws.iter().any(|k| lower.contains(k)) {
                    hit += 1.0;
                }
            } else if lower.contains(&gl) {
                hit += 1.0;
            }
        }
        (hit / profile.geography.len() as f64).min(1.0)
    };
    (0.30 * ind + 0.25 * topic + 0.20 * comp + 0.15 * aud + 0.10 * geo) * 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifier_tags_ai_text() {
        let v = classify_industries("OpenAI releases a new LLM agent model, GPT-5 benchmarks");
        assert!(v.iter().any(|(id, _)| id == "ai" || id == "technology"));
        assert!(v[0].1 > 0.0);
    }

    #[test]
    fn classifier_tags_healthcare_zh() {
        let v = classify_industries("医院发布最新医疗器械临床研究报告");
        assert!(v.iter().any(|(id, _)| id == "healthcare"));
    }

    #[test]
    fn relevance_weights_favor_matching_industry() {
        let p = CustomerProfile {
            industry: "technology".into(),
            sub_industry: "ai".into(),
            topics: vec!["ai".into(), "llm".into()],
            competitors: vec!["openai".into()],
            ..Default::default()
        };
        let high = relevance_score(
            &["ai".to_string(), "technology".to_string()],
            "OpenAI LLM agent 发布",
            &p,
        );
        let low = relevance_score(&["real_estate".to_string()], "楼市新政", &p);
        assert!(high > low, "high={high} low={low}");
        assert!(high <= 100.0 && low >= 0.0);
    }
}
