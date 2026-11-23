//! Provider Manifest (Sincy Pulse §4): 每个第三方情报源一份 manifest,
//! 驱动注册/调度/归一化, 新增 provider 不改代码 (Admin Console 上传即可)。
//!
//! 红线: legal_status != public/commercial_ok 的 provider 不会被调度器启用。

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 法务状态 (设计 §6): 聚合分析可以, 再分发权不默认获得。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LegalStatus {
    Public,
    CommercialOk,
    NoRedistribute,
    AttributionRequired,
    #[default]
    Unknown,
    Blocked,
}

impl LegalStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            LegalStatus::Public => "public",
            LegalStatus::CommercialOk => "commercial_ok",
            LegalStatus::NoRedistribute => "no_redistribute",
            LegalStatus::AttributionRequired => "attribution_required",
            LegalStatus::Unknown => "unknown",
            LegalStatus::Blocked => "blocked",
        }
    }
    /// 是否允许调度采集 (unknown 也不允许 — 默认拒绝, 符合红线)
    pub fn schedulable(&self) -> bool {
        matches!(self, LegalStatus::Public | LegalStatus::CommercialOk)
    }
}

/// 采集方式 (设计 §2 Acquisition Engine): API / JSON / RSS / HTML / Browser / MCP
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcquisitionType {
    Api,
    Json,
    Rss,
    Html,
    Browser,
    Mcp,
    /// 内置 feed (复用 intel_collector 的 FeedConfig), 贴着现有代码走
    BuiltinFeed,
}

impl AcquisitionType {
    pub fn as_str(&self) -> &'static str {
        match self {
            AcquisitionType::Api => "api",
            AcquisitionType::Json => "json",
            AcquisitionType::Rss => "rss",
            AcquisitionType::Html => "html",
            AcquisitionType::Browser => "browser",
            AcquisitionType::Mcp => "mcp",
            AcquisitionType::BuiltinFeed => "builtin_feed",
        }
    }
}

/// 数据集: 一个 manifest 里的一组同频采集目标
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dataset {
    pub id: String,
    /// 更新频率 (秒), 调度器据此判断是否到期
    pub update_frequency_secs: u64,
    /// 对 builtin_feed provider: 对应 intel_collector 里的 feed label
    #[serde(default)]
    pub feed_label: Option<String>,
    /// 对 web 类 provider (json/html/api): 该 dataset 抓取的 URL
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub entity_type: String,
}

/// Provider Manifest 顶层
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderManifest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub platform: String,
    #[serde(default = "default_category")]
    pub category: String,
    /// 能力标签, 引擎按能力选 provider (设计 §3 capabilities)
    #[serde(default)]
    pub capabilities: Vec<String>,
    pub acquisition: Acquisition,
    #[serde(default)]
    pub datasets: Vec<Dataset>,
    /// 归一化产出的实体类型 (social_account / social_post / ranking / trend ...)
    #[serde(default)]
    pub entity_types: Vec<String>,
    /// 初始可靠性分 (设计 §7 reliability), 引擎据此算 confidence
    #[serde(default = "default_reliability")]
    pub reliability: f64,
    /// 法务状态, 默认 unknown = 不启用
    #[serde(default)]
    pub legal_status: LegalStatus,
    /// 额外配置 (端点/密钥引用等), 不存明文 key — key 走 vault:// 引用
    #[serde(default)]
    pub config: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Acquisition {
    #[serde(rename = "type")]
    pub acq_type: AcquisitionType,
    /// 策略: json_api / html_parser / rss / browser_mcp ...
    #[serde(default)]
    pub strategy: String,
}

fn default_category() -> String {
    "social_intelligence".into()
}
fn default_reliability() -> f64 {
    0.5
}

impl ProviderManifest {
    pub fn parse(json: &Value) -> Result<Self, String> {
        serde_json::from_value(json.clone()).map_err(|e| format!("invalid manifest: {e}"))
    }

    pub fn dataset(&self, dataset_id: &str) -> Option<&Dataset> {
        self.datasets.iter().find(|d| d.id == dataset_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_manifest() {
        let m = ProviderManifest::parse(&serde_json::json!({
            "id": "xbangdan",
            "name": "Xbangdan",
            "platform": "x",
            "capabilities": ["account_ranking", "viral_content"],
            "acquisition": {"type": "html", "strategy": "html_parser"},
            "datasets": [
                {"id": "account_ranking", "update_frequency_secs": 21600, "entity_type": "social_account"},
                {"id": "viral_posts", "update_frequency_secs": 3600, "entity_type": "social_post"}
            ],
            "entity_types": ["social_account", "social_post"],
            "reliability": 0.75,
            "legal_status": "commercial_ok"
        }))
        .unwrap();
        assert_eq!(m.id, "xbangdan");
        assert_eq!(m.datasets.len(), 2);
        assert!(m.legal_status.schedulable());
        assert_eq!(
            m.dataset("viral_posts").unwrap().update_frequency_secs,
            3600
        );
    }

    #[test]
    fn unknown_legal_status_is_not_schedulable() {
        let m = ProviderManifest::parse(&serde_json::json!({
            "id": "x", "name": "X", "acquisition": {"type": "api"}
        }))
        .unwrap();
        assert_eq!(m.legal_status, LegalStatus::Unknown);
        assert!(!m.legal_status.schedulable());
    }

    #[test]
    fn rejects_bad_manifest() {
        assert!(ProviderManifest::parse(&serde_json::json!({"id": 1})).is_err());
    }
}
