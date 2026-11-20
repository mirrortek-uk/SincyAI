//! Quality / Policy gate (FR-007, TDD 6.2 "Quality/Policy"):
//! facts, brand rules, media specs, rights, duplicate content, channel policy,
//! AI label. The gate never relies on LLM self-assessment alone for rights.

use serde::{Deserialize, Serialize};

use crate::ai_label::AiLabel;
use crate::package::PublishingPackage;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityIssue {
    pub code: &'static str,
    pub message: String,
    /// block = must fix; warn = recorded but can pass
    pub severity: Severity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Block,
    Warn,
}

#[derive(Debug, Clone)]
pub struct GateVerdict {
    pub issues: Vec<QualityIssue>,
}

impl GateVerdict {
    pub fn passed(&self) -> bool {
        !self.issues.iter().any(|i| i.severity == Severity::Block)
    }
}

/// Brand rules snapshot used by the gate (FR-002).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BrandRules {
    pub forbidden_topics: Vec<String>,
    pub required_cta: Option<String>,
    pub max_text_len: Option<usize>,
    /// per-platform media spec: platform -> (max_bytes, allowed mimes)
    pub media_specs: std::collections::HashMap<String, MediaSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaSpec {
    pub max_bytes: u64,
    pub allowed_mimes: Vec<String>,
}

/// The quality/policy gate. Pure function over package + brand rules + known hashes.
pub struct QualityGate {
    pub brand_rules: BrandRules,
    /// sha256 of already-published assets in this brand (duplicate detection)
    pub known_hashes: Vec<String>,
}

impl QualityGate {
    pub fn new(brand_rules: BrandRules) -> Self {
        Self {
            brand_rules,
            known_hashes: Vec::new(),
        }
    }

    pub fn with_known_hashes(mut self, hashes: Vec<String>) -> Self {
        self.known_hashes = hashes;
        self
    }

    pub fn check(&self, pkg: &PublishingPackage, ai_label: &AiLabel) -> GateVerdict {
        let mut issues = Vec::new();

        // AI 标识 (FR-024)
        if ai_label.is_ai_generated
            && ai_label.tools.is_empty()
            && ai_label.c2pa_manifest_ref.is_none()
        {
            issues.push(QualityIssue {
                code: "ai_label_missing_provenance",
                message: "AI 生成内容缺少工具链或 C2PA 凭证".into(),
                severity: Severity::Block,
            });
        }

        // 品牌禁用主题 (FR-002)
        let text_lower = pkg.text().to_lowercase();
        for topic in &self.brand_rules.forbidden_topics {
            if text_lower.contains(&topic.to_lowercase()) {
                issues.push(QualityIssue {
                    code: "forbidden_topic",
                    message: format!("命中品牌禁用主题: {topic}"),
                    severity: Severity::Block,
                });
            }
        }

        // CTA
        if let Some(cta) = &self.brand_rules.required_cta
            && !pkg.text().contains(cta.as_str())
        {
            issues.push(QualityIssue {
                code: "missing_cta",
                message: format!("缺少必需 CTA: {cta}"),
                severity: Severity::Block,
            });
        }

        // text length
        if let Some(max) = self.brand_rules.max_text_len
            && pkg.text().chars().count() > max
        {
            issues.push(QualityIssue {
                code: "text_too_long",
                message: format!("文案超长: {} > {max}", pkg.text().chars().count()),
                severity: Severity::Block,
            });
        }

        // media spec per platform (FR-007 渠道规格)
        if let Some(spec) = self.brand_rules.media_specs.get(pkg.platform()) {
            for m in pkg.media() {
                if m.bytes > spec.max_bytes {
                    issues.push(QualityIssue {
                        code: "media_too_large",
                        message: format!(
                            "媒体 {} 超出平台上限 {} bytes",
                            m.asset_id, spec.max_bytes
                        ),
                        severity: Severity::Block,
                    });
                }
                if !spec.allowed_mimes.iter().any(|a| a == &m.mime) {
                    issues.push(QualityIssue {
                        code: "media_mime_not_allowed",
                        message: format!("平台 {} 不允许 mime {}", pkg.platform(), m.mime),
                        severity: Severity::Block,
                    });
                }
            }
        }

        // duplicate content (FR-007)
        for m in pkg.media() {
            if self.known_hashes.iter().any(|h| h == &m.sha256) {
                issues.push(QualityIssue {
                    code: "duplicate_content",
                    message: format!(
                        "媒体 sha256 {} 与已发布内容重复",
                        &m.sha256[..m.sha256.len().min(12)]
                    ),
                    severity: Severity::Block,
                });
            }
        }

        GateVerdict { issues }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::{MediaAsset, PackageEditor};
    use serde_json::json;
    use uuid::Uuid;

    fn pkg_with(text: &str, media: Vec<MediaAsset>) -> PublishingPackage {
        let label = AiLabel::ai_generated(vec![json!({"model": "m"})]);
        PublishingPackage::create(
            Uuid::new_v4(),
            "a",
            "b",
            "youtube",
            "acc",
            "publish_video",
            text,
            media,
            label,
            json!({}),
        )
        .expect("pkg")
    }

    fn rules() -> BrandRules {
        let mut specs = std::collections::HashMap::new();
        specs.insert(
            "youtube".to_string(),
            MediaSpec {
                max_bytes: 1000,
                allowed_mimes: vec!["video/mp4".into()],
            },
        );
        BrandRules {
            forbidden_topics: vec!["赌博".into()],
            required_cta: Some("关注我们".into()),
            max_text_len: Some(100),
            media_specs: specs,
        }
    }

    #[test]
    fn gate_blocks_forbidden_topic_and_missing_cta() {
        let gate = QualityGate::new(rules());
        let pkg = pkg_with("参与赌博吧", vec![]);
        let v = gate.check(&pkg, pkg.ai_label());
        assert!(!v.passed());
        assert!(v.issues.iter().any(|i| i.code == "forbidden_topic"));
        assert!(v.issues.iter().any(|i| i.code == "missing_cta"));
    }

    #[test]
    fn gate_passes_clean_package() {
        let gate = QualityGate::new(rules());
        let pkg = pkg_with("好内容，关注我们", vec![]);
        let v = gate.check(&pkg, pkg.ai_label());
        assert!(v.passed(), "{:?}", v.issues);
    }

    #[test]
    fn gate_blocks_oversize_and_duplicate_media() {
        let media = MediaAsset {
            asset_id: Uuid::new_v4(),
            mime: "video/mp4".into(),
            bytes: 5000,
            sha256: "deadbeef".into(),
            storage_ref: "s3://x".into(),
            duration_secs: None,
            width: None,
            height: None,
        };
        let pkg = pkg_with("关注我们", vec![media.clone()]);
        let gate = QualityGate::new(rules()).with_known_hashes(vec!["deadbeef".into()]);
        let v = gate.check(&pkg, pkg.ai_label());
        assert!(v.issues.iter().any(|i| i.code == "media_too_large"));
        assert!(v.issues.iter().any(|i| i.code == "duplicate_content"));
    }

    #[test]
    fn edited_version_goes_back_through_gate() {
        let p1 = pkg_with("关注我们", vec![]);
        let editor = PackageEditor::new(&p1);
        let p2 = editor
            .new_version(Some("赌博内容".into()), None, None)
            .unwrap();
        let gate = QualityGate::new(rules());
        assert!(!gate.check(&p2, p2.ai_label()).passed());
        assert!(gate.check(&p1, p1.ai_label()).passed());
    }
}
