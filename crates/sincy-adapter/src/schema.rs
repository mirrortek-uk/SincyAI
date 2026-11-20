//! Declarative App Adapter schema (TDD 9.3). Adapters live in adapters/*.yaml
//! and are published signed; devices only execute signed, version-matching ones.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap as Map;
#[allow(unused_imports)]
use std::collections::HashMap;

#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("invalid adapter definition: {0}")]
    Invalid(String),
    #[error("yaml parse error: {0}")]
    Yaml(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterDefinition {
    pub adapter: String, // e.g. xiaohongshu.publish_video
    pub version: u32,
    pub app: AppTarget,
    pub screens: Map<String, ScreenDef>,
    pub flow: Vec<FlowStep>,
    #[serde(default)]
    pub known_interrupts: Vec<Interrupt>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppTarget {
    pub package: String,
    /// e.g. ">=8.10 <8.20"
    pub version_range: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenDef {
    #[serde(default)]
    pub anchors: Vec<Anchor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anchor {
    #[serde(default)]
    pub id: Option<String>, // resource-id
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub text_any: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowStep {
    pub expect: String, // screen id
    pub action: ActionKind,
    #[serde(default)]
    pub target: Option<Anchor>,
    #[serde(default)]
    pub value: Option<String>,
    /// true for steps with external side effects (e.g. tapping 发布)
    #[serde(default)]
    pub side_effect: bool,
    /// wait for a post-condition screen
    #[serde(default)]
    pub verify: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Tap,
    SetText,
    PickMedia,
    Scroll,
    Back,
    CloseDialog,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Interrupt {
    pub screen: String,
    pub action: ActionKind,
    pub target: Anchor,
}

impl AdapterDefinition {
    pub fn from_yaml(raw: &str) -> Result<Self, AdapterError> {
        let def: AdapterDefinition =
            serde_yaml::from_str(raw).map_err(|e| AdapterError::Yaml(e.to_string()))?;
        def.validate()?;
        Ok(def)
    }

    pub fn validate(&self) -> Result<(), AdapterError> {
        if self.adapter.is_empty() || self.version == 0 {
            return Err(AdapterError::Invalid("adapter id/version required".into()));
        }
        if self.app.package.is_empty() || self.app.version_range.is_empty() {
            return Err(AdapterError::Invalid(
                "app package/version_range required".into(),
            ));
        }
        if self.flow.is_empty() {
            return Err(AdapterError::Invalid("flow must not be empty".into()));
        }
        for step in &self.flow {
            if !self.screens.contains_key(&step.expect) {
                return Err(AdapterError::Invalid(format!(
                    "step expects unknown screen {}",
                    step.expect
                )));
            }
            // side-effect steps must target something concrete
            if step.side_effect && step.target.is_none() {
                return Err(AdapterError::Invalid(
                    "side_effect step requires a target anchor".into(),
                ));
            }
            // actions that need a target
            match step.action {
                ActionKind::Tap | ActionKind::SetText | ActionKind::PickMedia => {
                    if step.target.is_none() && step.value.is_none() {
                        return Err(AdapterError::Invalid(format!(
                            "action {:?} requires target or value",
                            step.action
                        )));
                    }
                }
                ActionKind::Scroll | ActionKind::Back | ActionKind::CloseDialog => {}
            }
        }
        // at least one screen must be the challenge detector
        let has_challenge = self.screens.values().any(|s| {
            s.anchors.iter().any(|a| {
                a.text_any
                    .as_ref()
                    .map(|v| {
                        v.iter()
                            .any(|t| t.contains("验证") || t.contains("安全检测"))
                    })
                    .unwrap_or(false)
            })
        });
        if !has_challenge {
            return Err(AdapterError::Invalid(
                "adapter must declare a challenge screen (验证/安全检测)".into(),
            ));
        }
        Ok(())
    }

    pub fn challenge_screen(&self) -> Option<&str> {
        self.screens.iter().find_map(|(name, s)| {
            s.anchors
                .iter()
                .any(|a| {
                    a.text_any
                        .as_ref()
                        .map(|v| {
                            v.iter()
                                .any(|t| t.contains("验证") || t.contains("安全检测"))
                        })
                        .unwrap_or(false)
                })
                .then_some(name.as_str())
        })
    }
}
