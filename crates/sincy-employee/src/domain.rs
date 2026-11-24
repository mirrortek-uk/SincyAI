//! Employee domain entities (DE §6, §7, §17).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EmployeeStatus {
    Created,
    Configuring,
    Ready,
    Running,
    Paused,
    Stopped,
    WaitingApproval,
    Degraded,
    Error,
    Archived,
}

impl std::fmt::Display for EmployeeStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl EmployeeStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            EmployeeStatus::Created => "CREATED",
            EmployeeStatus::Configuring => "CONFIGURING",
            EmployeeStatus::Ready => "READY",
            EmployeeStatus::Running => "RUNNING",
            EmployeeStatus::Paused => "PAUSED",
            EmployeeStatus::Stopped => "STOPPED",
            EmployeeStatus::WaitingApproval => "WAITING_APPROVAL",
            EmployeeStatus::Degraded => "DEGRADED",
            EmployeeStatus::Error => "ERROR",
            EmployeeStatus::Archived => "ARCHIVED",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "CREATED" => EmployeeStatus::Created,
            "CONFIGURING" => EmployeeStatus::Configuring,
            "READY" => EmployeeStatus::Ready,
            "RUNNING" => EmployeeStatus::Running,
            "PAUSED" => EmployeeStatus::Paused,
            "STOPPED" => EmployeeStatus::Stopped,
            "WAITING_APPROVAL" => EmployeeStatus::WaitingApproval,
            "DEGRADED" => EmployeeStatus::Degraded,
            "ERROR" => EmployeeStatus::Error,
            "ARCHIVED" => EmployeeStatus::Archived,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Employee {
    pub id: Uuid,
    pub agency_id: Uuid,
    pub name: String,
    pub role: String,
    pub status: EmployeeStatus,
    pub config_version: i32,
    pub brand_id: Option<Uuid>,
    /// primary channel (MVP: x)
    pub channel: String,
    /// false = generated packages need human approval before publishing (FR-024)
    pub auto_publish: bool,
    pub created_at: DateTime<Utc>,
}

/// Versioned identity (DE §6).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    #[serde(default = "default_strategy_version")]
    pub version: i32,
    pub name: String,
    pub role: String,
    #[serde(default)]
    pub personality: Value,
    #[serde(default = "default_voice")]
    pub voice: String,
    /// 输出语言 (BCP-47), 默认中文; 由品牌 language 或创建时显式指定
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default)]
    pub audience: Vec<String>,
    #[serde(default)]
    pub expertise: Vec<String>,
    #[serde(default)]
    pub values: Vec<String>,
    #[serde(default)]
    pub boundaries: Vec<String>,
}

fn default_voice() -> String {
    "neutral".into()
}

pub fn default_language() -> String {
    "zh-CN".into()
}

/// Mission as structured fields + text (DE §6).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mission {
    pub statement: String,
    #[serde(default = "default_domain")]
    pub domain: String,
    #[serde(default)]
    pub audience: Vec<String>,
    #[serde(default)]
    pub value_proposition: String,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default)]
    pub success_definition: String,
}

fn default_domain() -> String {
    "content".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Goal {
    pub id: Uuid,
    pub kind: String, // growth/engagement/authority/reach/conversion/quality/consistency
    pub target: Value, // {metric, value, horizon}
    pub deadline: Option<DateTime<Utc>>,
    pub status: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StrategyStatus {
    #[default]
    Draft,
    Review,
    Active,
    Superseded,
}
// Default is Draft (derived above)

/// Versioned strategy (DE §7: DRAFT → REVIEW → ACTIVE → SUPERSEDED).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Strategy {
    #[serde(default = "default_strategy_version")]
    pub version: i32,
    #[serde(default)]
    pub status: StrategyStatus,
    /// content pillars, cadence, formats, channel mix, risk tolerance, exploration budget
    #[serde(default)]
    pub pillars: Vec<String>,
    #[serde(default = "default_cadence")]
    pub cadence_per_week: u32,
    #[serde(default = "default_formats")]
    pub formats: Vec<String>,
    #[serde(default = "default_risk")]
    pub risk_tolerance: f64,
    #[serde(default = "default_risk")]
    pub exploration_budget: f64,
    /// patch-B KPI: 目标粉丝数 (可选)。设置后 cycle 读 account_growth 对照,
    /// 落后目标时 plan 注入增长导向 (涨粉角度), drain 用 Δ粉丝当 reward。
    #[serde(default)]
    pub target_followers: Option<i64>,
}

fn default_cadence() -> u32 {
    7
}

fn default_formats() -> Vec<String> {
    vec!["text".into()]
}

fn default_risk() -> f64 {
    0.3
}

fn default_strategy_version() -> i32 {
    1
}

impl Default for Strategy {
    fn default() -> Self {
        Strategy {
            version: 1,
            status: StrategyStatus::Draft,
            pillars: vec![],
            cadence_per_week: 7,
            formats: vec!["text".into()],
            risk_tolerance: 0.3,
            exploration_budget: 0.2,
            target_followers: None,
        }
    }
}
