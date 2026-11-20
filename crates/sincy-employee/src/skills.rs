//! Skill 生态 (DE guide §: skills 是可复用、可版本化、可测试的能力模块;
//! agent 是角色/决策主体, 按域动态调用 skill, skill 再复用 tool)。
//!
//! 9 个 skill 域, 初始 catalog 见 migrations/0021_skills.sql (77 个)。

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::agents::AgentKind;

/// 9 个 skill 域 (数量不固定, 可动态注册新 skill 进任意域)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillDomain {
    Research,
    Strategy,
    Creative,
    Content,
    QualityCompliance,
    Publishing,
    Analytics,
    Learning,
    Memory,
}

impl SkillDomain {
    pub const ALL: [SkillDomain; 9] = [
        SkillDomain::Research,
        SkillDomain::Strategy,
        SkillDomain::Creative,
        SkillDomain::Content,
        SkillDomain::QualityCompliance,
        SkillDomain::Publishing,
        SkillDomain::Analytics,
        SkillDomain::Learning,
        SkillDomain::Memory,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            SkillDomain::Research => "research",
            SkillDomain::Strategy => "strategy",
            SkillDomain::Creative => "creative",
            SkillDomain::Content => "content",
            SkillDomain::QualityCompliance => "quality_compliance",
            SkillDomain::Publishing => "publishing",
            SkillDomain::Analytics => "analytics",
            SkillDomain::Learning => "learning",
            SkillDomain::Memory => "memory",
        }
    }

    pub fn title_zh(&self) -> &'static str {
        match self {
            SkillDomain::Research => "研究",
            SkillDomain::Strategy => "战略",
            SkillDomain::Creative => "创意",
            SkillDomain::Content => "内容生产",
            SkillDomain::QualityCompliance => "质量与合规",
            SkillDomain::Publishing => "发布",
            SkillDomain::Analytics => "数据分析",
            SkillDomain::Learning => "学习与优化",
            SkillDomain::Memory => "记忆管理",
        }
    }

    pub fn parse(s: &str) -> Option<SkillDomain> {
        SkillDomain::ALL.iter().find(|d| d.as_str() == s).copied()
    }
}

/// 一条 skill 注册记录 (当前版本)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub id: Uuid,
    pub domain: String,
    pub name: String,
    pub version: i32,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub tools: Vec<String>,
    pub status: String,
}

impl Skill {
    pub fn domain(&self) -> Option<SkillDomain> {
        SkillDomain::parse(&self.domain)
    }
}

/// 每个核心 agent 负责的 skill 域 (Executive 是协调者, 可跨域调用, 这里不绑定私有域)。
pub fn agent_skill_domains(agent: &AgentKind) -> Vec<SkillDomain> {
    match agent {
        AgentKind::Executive => vec![],
        AgentKind::ResearchStrategy => vec![SkillDomain::Research, SkillDomain::Strategy],
        AgentKind::Creative => vec![SkillDomain::Creative],
        AgentKind::Content => vec![SkillDomain::Content],
        AgentKind::CriticCompliance => {
            vec![SkillDomain::QualityCompliance, SkillDomain::Publishing]
        }
        AgentKind::AnalyticsLearning => {
            vec![
                SkillDomain::Analytics,
                SkillDomain::Learning,
                SkillDomain::Memory,
            ]
        }
    }
}

/// 从员工绑定的 skills 里挑出某 agent 负责的域。
pub fn skills_for_agent<'a>(skills: &'a [Skill], agent: &AgentKind) -> Vec<&'a Skill> {
    let domains = agent_skill_domains(agent);
    skills
        .iter()
        .filter(|s| s.status == "active" && domains.iter().any(|d| d.as_str() == s.domain))
        .collect()
}

/// 为某渠道挑选内容生产 skill (x 文本帖 → social_post_generation, 兜底 text_generation)。
pub fn pick_content_skill<'a>(skills: &'a [Skill], channel: &str) -> Option<&'a Skill> {
    let content = skills_for_agent(skills, &AgentKind::Content);
    let preferred = match channel {
        "x" => ["social_post_generation", "text_generation"],
        _ => ["text_generation", "article_writing"],
    };
    for p in preferred {
        if let Some(s) = content.iter().find(|s| s.name == p) {
            return Some(s);
        }
    }
    content.first().copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nine_domains() {
        assert_eq!(SkillDomain::ALL.len(), 9);
        for d in SkillDomain::ALL {
            assert_eq!(SkillDomain::parse(d.as_str()), Some(d));
        }
    }

    #[test]
    fn agent_domain_mapping() {
        assert!(agent_skill_domains(&AgentKind::Executive).is_empty());
        assert_eq!(agent_skill_domains(&AgentKind::ResearchStrategy).len(), 2);
        assert_eq!(agent_skill_domains(&AgentKind::CriticCompliance).len(), 2);
        assert_eq!(agent_skill_domains(&AgentKind::AnalyticsLearning).len(), 3);
    }

    #[test]
    fn pick_x_post_skill() {
        let skills = vec![
            Skill {
                id: Uuid::new_v4(),
                domain: "content".into(),
                name: "text_generation".into(),
                version: 1,
                title: "文本生成".into(),
                description: "".into(),
                tools: vec![],
                status: "active".into(),
            },
            Skill {
                id: Uuid::new_v4(),
                domain: "content".into(),
                name: "social_post_generation".into(),
                version: 1,
                title: "社交帖子生成".into(),
                description: "".into(),
                tools: vec![],
                status: "active".into(),
            },
        ];
        assert_eq!(
            pick_content_skill(&skills, "x").unwrap().name,
            "social_post_generation"
        );
        assert_eq!(
            pick_content_skill(&skills, "blog").unwrap().name,
            "text_generation"
        );
    }
}
