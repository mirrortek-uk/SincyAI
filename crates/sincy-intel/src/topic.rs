//! 真话题抽取 (替代"正文前 60 字冒充 topic"的占位做法)。
//!
//! 两层: ① 规范话题词表 (canonical topic lexicon, 中英文关键词, 与 industry.rs
//! 词库同款机制) 命中打分取 top-k; ② 词表全未命中时退化为关键词抽取
//! (英文: 去停用词的长词按词频; 中文: 高频 bigram), 而不是截断文本。

/// 规范话题: id + 中英文关键词
struct TopicDef {
    id: &'static str,
    keywords: &'static [&'static str],
}

fn lexicon() -> Vec<TopicDef> {
    vec![
        TopicDef {
            id: "ai_agents",
            keywords: &[
                "ai agent",
                "agents",
                "智能体",
                "agent 框架",
                "autonomous agent",
                "multi-agent",
                "多智能体",
            ],
        },
        TopicDef {
            id: "llm_models",
            keywords: &[
                "llm",
                "gpt",
                "大模型",
                "chatgpt",
                "claude",
                "gemini",
                "deepseek",
                "qwen",
                "llama",
                "mistral",
                "生成式",
            ],
        },
        TopicDef {
            id: "aigc_content",
            keywords: &[
                "aigc",
                "ai 生成",
                "midjourney",
                "sora",
                "生成式 ai",
                "ai 绘画",
                "ai 视频",
            ],
        },
        TopicDef {
            id: "chips_semiconductors",
            keywords: &[
                "芯片",
                "半导体",
                "gpu",
                "英伟达",
                "nvidia",
                "tsmc",
                "台积电",
                "制程",
                "arm 架构",
            ],
        },
        TopicDef {
            id: "open_source",
            keywords: &["open source", "开源", "github trending", "star", "仓库"],
        },
        TopicDef {
            id: "cybersecurity",
            keywords: &[
                "cybersecurity",
                "漏洞",
                "黑客",
                "ransomware",
                "安全事件",
                "cve",
                "0day",
                "渗透",
            ],
        },
        TopicDef {
            id: "cloud_infra",
            keywords: &[
                "cloud",
                "aws",
                "azure",
                "阿里云",
                "腾讯云",
                "kubernetes",
                "云原生",
                "serverless",
                "数据库",
            ],
        },
        TopicDef {
            id: "crypto_web3",
            keywords: &[
                "crypto",
                "bitcoin",
                "比特币",
                "eth",
                "web3",
                "defi",
                "nft",
                "加密货币",
                "链上",
            ],
        },
        TopicDef {
            id: "ev_automotive",
            keywords: &[
                "新能源车",
                "电动车",
                "ev",
                "tesla",
                "特斯拉",
                "比亚迪",
                "蔚小理",
                "智能驾驶",
                "自动驾驶",
            ],
        },
        TopicDef {
            id: "funding_ma",
            keywords: &[
                "融资",
                "轮融资",
                "ipo",
                "上市",
                "并购",
                "估值",
                "独角兽",
                "funding",
                "series a",
                "series b",
                "vc",
            ],
        },
        TopicDef {
            id: "mobile_devices",
            keywords: &[
                "手机",
                "iphone",
                "苹果",
                "华为",
                "小米",
                "鸿蒙",
                "android",
                "新品发布",
            ],
        },
        TopicDef {
            id: "short_video_social",
            keywords: &[
                "短视频",
                "抖音",
                "tiktok",
                "小红书",
                "微博热搜",
                "热榜",
                "viral",
                "爆火",
                "涨粉",
            ],
        },
        TopicDef {
            id: "embodied_ai",
            keywords: &[
                "具身智能",
                "embodied ai",
                "机器人",
                "robotaxi",
                "人形机器人",
            ],
        },
        TopicDef {
            id: "ai_regulation",
            keywords: &[
                "ai 监管",
                "人工智能法",
                "监管",
                "ai 法案",
                "合规",
                "算法备案",
                "生成式人工智能",
            ],
        },
        TopicDef {
            id: "enterprise_software",
            keywords: &["saas", "企业服务", "b2b", "erp", "crm", "订阅"],
        },
        TopicDef {
            id: "consumer_apps",
            keywords: &["app", "小程序", "独立开发者", "上架", "苹果应用", "新 app"],
        },
        TopicDef {
            id: "games",
            keywords: &["游戏", "手游", "3a", "游戏版本", "电竞", "独立游戏"],
        },
        TopicDef {
            id: "ecommerce_live",
            keywords: &[
                "电商",
                "直播带货",
                "天猫",
                "京东",
                "拼多多",
                "跨境电商",
                "出海",
                "temu",
                "shein",
            ],
        },
        TopicDef {
            id: "macro_economy",
            keywords: &[
                "美联储",
                "降息",
                "加息",
                "cpi",
                "通胀",
                "gdp",
                "经济",
                "股市",
                "a股",
                "港股",
                "美股",
            ],
        },
        TopicDef {
            id: "real_estate",
            keywords: &["房地产", "楼市", "房价", "地产", "土地拍卖", "城投"],
        },
        TopicDef {
            id: "new_energy",
            keywords: &["光伏", "风电", "储能", "电池", "新能源", "碳中和", "锂矿"],
        },
        TopicDef {
            id: "healthcare_ai",
            keywords: &[
                "医疗",
                "医药",
                "生物医药",
                "ai 医疗",
                "医疗器械",
                "创新药",
                "医院",
            ],
        },
        TopicDef {
            id: "education",
            keywords: &["教育", "培训", "留学", "edtech", "在线教育", "高考"],
        },
        TopicDef {
            id: "layoffs_hiring",
            keywords: &["裁员", "招聘", "降薪", "大厂", "offer", "内卷", "35岁"],
        },
    ]
}

const EN_STOPWORDS: &[&str] = &[
    "this", "that", "with", "from", "have", "has", "will", "would", "could", "should", "about",
    "into", "over", "after", "before", "between", "their", "there", "which", "what", "when",
    "where", "your", "yours", "them", "they", "then", "than", "also", "more", "most", "some",
    "such", "only", "just", "like", "been", "were", "was", "are", "the", "and", "for", "not",
    "but", "you", "all", "can", "her", "his", "its", "our", "out", "per", "via", "new",
];

/// 抽取真话题标签: 词表命中优先, 未命中退化为关键词(bigram/长词)抽取。返回 top-k。
pub fn extract_topics(text: &str, k: usize) -> Vec<String> {
    let lower = text.to_lowercase();
    let mut scored: Vec<(&'static str, f64)> = Vec::new();
    for def in lexicon() {
        let hits = def
            .keywords
            .iter()
            .filter(|kw| lower.contains(&kw.to_lowercase()))
            .count() as f64;
        if hits > 0.0 {
            let s = (hits / def.keywords.len() as f64).min(1.0) * 0.5 + hits.min(3.0) / 6.0;
            scored.push((def.id, s.min(1.0)));
        }
    }
    if !scored.is_empty() {
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        return scored
            .into_iter()
            .take(k)
            .map(|(id, _)| id.to_string())
            .collect();
    }
    // 退化: 关键词抽取
    fallback_keywords(text, k)
}

/// 关键词抽取: 英文长词去停用词按词频; 中文按 bigram 频次。取 top-k。
fn fallback_keywords(text: &str, k: usize) -> Vec<String> {
    use std::collections::HashMap;
    let mut freq: HashMap<String, usize> = HashMap::new();
    // 英文词
    let mut cur = String::new();
    for c in text.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_alphanumeric() {
            cur.push(c.to_ascii_lowercase());
        } else {
            if cur.len() >= 4 && !EN_STOPWORDS.contains(&cur.as_str()) {
                *freq.entry(cur.clone()).or_insert(0) += 1;
            }
            cur.clear();
        }
    }
    // 中文 bigram
    let chars: Vec<char> = text.chars().filter(|c| c.is_alphanumeric()).collect();
    for w in chars.windows(2) {
        let bg: String = w.iter().collect();
        if w.iter().all(|c| *c >= '\u{4e00}' && *c <= '\u{9fff}') {
            *freq.entry(bg).or_insert(0) += 1;
        }
    }
    let mut v: Vec<(String, usize)> = freq.into_iter().filter(|(_, n)| *n >= 1).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v.into_iter().take(k).map(|(s, _)| s).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexicon_hit_returns_canonical_topics() {
        let t = extract_topics(
            "OpenAI 发布新一代大模型 GPT-5, AI Agent 能力再升级, 智能体框架全面开源",
            5,
        );
        assert!(t.contains(&"llm_models".to_string()));
        assert!(t.contains(&"ai_agents".to_string()));
        assert!(t.contains(&"open_source".to_string()));
        assert!(!t.iter().any(|x| x.len() > 40)); // 不再是 60 字截断
    }

    #[test]
    fn no_lexicon_hit_falls_back_to_keywords() {
        let t = extract_topics("量子计算取得突破, 超导量子比特数再创新高", 5);
        assert!(!t.is_empty());
        assert!(t.iter().all(|x| x.chars().count() <= 12));
    }

    #[test]
    fn empty_text_returns_empty() {
        assert!(extract_topics("", 5).is_empty());
    }
}
