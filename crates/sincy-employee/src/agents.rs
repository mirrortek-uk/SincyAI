//! Logical agents (DE §8, §21: six MVP agents). Agents are capabilities invoked
//! by the orchestrator with a fixed contract — not long-running chatbots.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    Executive,
    ResearchStrategy,
    Creative,
    Content,
    CriticCompliance,
    AnalyticsLearning,
}

impl AgentKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentKind::Executive => "executive",
            AgentKind::ResearchStrategy => "research_strategy",
            AgentKind::Creative => "creative",
            AgentKind::Content => "content",
            AgentKind::CriticCompliance => "critic_compliance",
            AgentKind::AnalyticsLearning => "analytics_learning",
        }
    }

    /// 插件化展示:中文标题
    pub fn title(&self) -> &'static str {
        match self {
            AgentKind::Executive => "执行官 Agent",
            AgentKind::ResearchStrategy => "研究战略 Agent",
            AgentKind::Creative => "创意 Agent",
            AgentKind::Content => "内容 Agent",
            AgentKind::CriticCompliance => "审核合规 Agent",
            AgentKind::AnalyticsLearning => "分析学习 Agent",
        }
    }

    /// 插件化展示:明确说明这个 agent 具体干什么
    pub fn description(&self) -> &'static str {
        match self {
            AgentKind::Executive => {
                "统筹整个运营周期:把目标拆成可执行的任务、决定先做什么后做什么、 \
                 在预算内调度其他 agent、遇到阻塞时决定升级给人还是自行恢复"
            }
            AgentKind::ResearchStrategy => {
                "做市场与趋势研究:读取 World Intelligence 的情报(热点/机会/竞品), \
                 结合品牌定位产出选题策略和内容方向,回答'接下来该做什么、为什么'"
            }
            AgentKind::Creative => {
                "把策略变成创意:出标题、钩子、脚本框架和创意简报, \
                 保证内容有记忆点、符合品牌调性,而不是流水账文案"
            }
            AgentKind::Content => {
                "生产最终可发布的内容包:按平台格式改写文案、配图/视频建议、 \
                 生成带 provenance 的 Publishing Package,交给审核 gate"
            }
            AgentKind::CriticCompliance => {
                "质量与合规守门:检查品牌一致性、平台政策、版权/敏感内容、AI 内容标注, \
                 给出质量评分和放行/打回建议,不合格内容不许发布"
            }
            AgentKind::AnalyticsLearning => {
                "复盘数据并学习:读取发布后的表现指标,识别有效模式、生成假设与实验, \
                 把结论写回策略和创意 DNA,让下一个周期比上一个更好"
            }
        }
    }

    /// 插件清单条目
    pub fn plugin_json(&self) -> Value {
        serde_json::json!({
            "type": "agent",
            "kind": self.as_str(),
            "title": self.title(),
            "description": self.description(),
        })
    }

    pub fn all() -> [AgentKind; 6] {
        [
            AgentKind::Executive,
            AgentKind::ResearchStrategy,
            AgentKind::Creative,
            AgentKind::Content,
            AgentKind::CriticCompliance,
            AgentKind::AnalyticsLearning,
        ]
    }

    pub fn all_plugin_json() -> Vec<Value> {
        Self::all().iter().map(|a| a.plugin_json()).collect()
    }
}

/// Invocation contract (DE §8): everything an agent needs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRequest {
    pub task_id: String,
    pub employee_id: String,
    pub config_version: i32,
    pub goal_context: Value,
    pub strategy_context: Value,
    pub memory: Vec<Value>,
    pub evidence_refs: Vec<String>,
    pub allowed_tools: Vec<String>,
    pub permission_scope: String,
    pub output_schema: String,
    pub budget_usd: f64,
}

/// What an agent returns (DE §8).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResult {
    pub structured: Value,
    pub evidence_refs: Vec<String>,
    pub confidence: f64,
    pub assumptions: Vec<String>,
    pub cost_usd: f64,
    pub recommended_next: Option<String>,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_agent_has_plugin_description() {
        let all = AgentKind::all_plugin_json();
        assert_eq!(all.len(), 6);
        for a in &all {
            assert_eq!(a["type"], "agent");
            assert!(a["title"].as_str().unwrap().len() > 2);
            assert!(a["description"].as_str().unwrap().len() > 20);
        }
    }
}
