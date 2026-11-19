//! AI content labeling (FR-024): AIGC 标识办法 / EU AI Act 透明度义务 / C2PA 预留.

use serde::{Deserialize, Serialize};

/// Per-platform disclosure requirement for AI-generated content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformLabelRequirement {
    pub platform: String,
    /// e.g. "label_required", "text_disclosure", "none"
    pub mode: String,
    /// template text appended to the copy when text_disclosure
    pub disclosure_text: Option<String>,
}

/// AI generation label attached to a content item (stored in ai_labels).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiLabel {
    pub is_ai_generated: bool,
    /// model/tool provenance, e.g. [{"provider": "...", "model": "...", "kind": "llm"}]
    pub tools: Vec<serde_json::Value>,
    /// C2PA manifest reference (reserved for v1.1+)
    pub c2pa_manifest_ref: Option<String>,
    pub platform_requirements: Vec<PlatformLabelRequirement>,
}

impl AiLabel {
    pub fn ai_generated(tools: Vec<serde_json::Value>) -> Self {
        Self {
            is_ai_generated: true,
            tools,
            c2pa_manifest_ref: None,
            platform_requirements: Vec::new(),
        }
    }

    pub fn human_made() -> Self {
        Self {
            is_ai_generated: false,
            tools: Vec::new(),
            c2pa_manifest_ref: None,
            platform_requirements: Vec::new(),
        }
    }

    /// The disclosure text to append for a given platform, if required.
    pub fn disclosure_for(&self, platform: &str) -> Option<String> {
        if !self.is_ai_generated {
            return None;
        }
        self.platform_requirements
            .iter()
            .find(|r| r.platform == platform)
            .and_then(|r| match r.mode.as_str() {
                "text_disclosure" => r.disclosure_text.clone(),
                "label_required" => Some("AI-generated content".to_string()),
                _ => None,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ai_label_disclosure_per_platform() {
        let mut label = AiLabel::ai_generated(vec![json!({"model": "gpt-x", "kind": "llm"})]);
        label.platform_requirements = vec![
            PlatformLabelRequirement {
                platform: "xiaohongshu".into(),
                mode: "text_disclosure".into(),
                disclosure_text: Some("本内容由 AI 生成".into()),
            },
            PlatformLabelRequirement {
                platform: "youtube".into(),
                mode: "label_required".into(),
                disclosure_text: None,
            },
        ];
        assert_eq!(
            label.disclosure_for("xiaohongshu").as_deref(),
            Some("本内容由 AI 生成")
        );
        assert_eq!(
            label.disclosure_for("youtube").as_deref(),
            Some("AI-generated content")
        );
        assert_eq!(label.disclosure_for("tiktok"), None);
        assert_eq!(AiLabel::human_made().disclosure_for("youtube"), None);
    }
}
