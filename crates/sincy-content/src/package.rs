//! Publishing Package (FR-009, TDD 13): immutable, versioned, route-agnostic.
//!
//! A package bundles copy, media (by hash), target channel/account, schedule,
//! constraints and AI-label requirements. Editing creates a new version; old
//! tasks keep executing against the version they were assigned.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::ai_label::AiLabel;

#[derive(Debug, thiserror::Error)]
pub enum PackageError {
    #[error("package is immutable: create a new version via PackageEditor")]
    Immutable,
    #[error("version must increase: current {current}, got {got}")]
    VersionNotIncreasing { current: u32, got: u32 },
    #[error("media hash mismatch for asset {asset_id}: expected {expected}, got {got}")]
    HashMismatch {
        asset_id: Uuid,
        expected: String,
        got: String,
    },
    #[error("package requires an AI label (FR-024)")]
    MissingAiLabel,
    #[error("package is not approved")]
    NotApproved,
}

/// One media asset inside a package: referenced by storage + SHA-256.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaAsset {
    pub asset_id: Uuid,
    pub mime: String,
    pub bytes: u64,
    pub sha256: String,
    /// S3 key / pre-signed ref — bytes never live in the package row
    pub storage_ref: String,
    pub duration_secs: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

impl MediaAsset {
    pub fn hash_of(bytes: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(bytes);
        format!("{:x}", h.finalize())
    }
}

/// Immutable publishing package. All fields private; no setters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishingPackage {
    package_id: Uuid,
    version: u32,
    agency_id: String,
    brand_id: String,
    platform: String,
    account_id: String,
    action: String,
    scheduled_at: Option<DateTime<Utc>>,
    timezone: String,
    text: String,
    media: Vec<MediaAsset>,
    constraints: serde_json::Value,
    ai_label: AiLabel,
    approval_status: ApprovalStatus,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    AutoApproved,
    Rejected,
    Withdrawn,
}

impl PublishingPackage {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        package_id: Uuid,
        agency_id: impl Into<String>,
        brand_id: impl Into<String>,
        platform: impl Into<String>,
        account_id: impl Into<String>,
        action: impl Into<String>,
        text: impl Into<String>,
        media: Vec<MediaAsset>,
        ai_label: AiLabel,
        constraints: serde_json::Value,
    ) -> Result<Self, PackageError> {
        if ai_label.is_ai_generated
            && ai_label.tools.is_empty()
            && ai_label.c2pa_manifest_ref.is_none()
        {
            // FR-024: AI 生成内容必须有 provenance（工具链或 C2PA）
            return Err(PackageError::MissingAiLabel);
        }
        Ok(Self {
            package_id,
            version: 1,
            agency_id: agency_id.into(),
            brand_id: brand_id.into(),
            platform: platform.into(),
            account_id: account_id.into(),
            action: action.into(),
            scheduled_at: None,
            timezone: "UTC".into(),
            text: text.into(),
            media,
            constraints,
            ai_label,
            approval_status: ApprovalStatus::Pending,
            created_at: Utc::now(),
        })
    }

    pub fn package_id(&self) -> Uuid {
        self.package_id
    }
    pub fn version(&self) -> u32 {
        self.version
    }
    pub fn platform(&self) -> &str {
        &self.platform
    }
    pub fn action(&self) -> &str {
        &self.action
    }
    pub fn account_id(&self) -> &str {
        &self.account_id
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn media(&self) -> &[MediaAsset] {
        &self.media
    }
    pub fn ai_label(&self) -> &AiLabel {
        &self.ai_label
    }
    pub fn approval_status(&self) -> ApprovalStatus {
        self.approval_status
    }
    pub fn constraints(&self) -> &serde_json::Value {
        &self.constraints
    }
    pub fn scheduled_at(&self) -> Option<DateTime<Utc>> {
        self.scheduled_at
    }

    /// Integrity check: every media asset's declared hash matches recomputed hash
    /// of its bytes when bytes are provided (transfer layer supplies bytes).
    pub fn verify_media_hashes(&self, bytes: &[(Uuid, Vec<u8>)]) -> Result<(), PackageError> {
        for (asset_id, data) in bytes {
            if let Some(asset) = self.media.iter().find(|m| &m.asset_id == asset_id) {
                let got = MediaAsset::hash_of(data);
                if got != asset.sha256 {
                    return Err(PackageError::HashMismatch {
                        asset_id: *asset_id,
                        expected: asset.sha256.clone(),
                        got,
                    });
                }
            }
        }
        Ok(())
    }
}

/// Editor produces NEW immutable versions; it never mutates the source.
pub struct PackageEditor<'a> {
    source: &'a PublishingPackage,
}

impl<'a> PackageEditor<'a> {
    pub fn new(source: &'a PublishingPackage) -> Self {
        Self { source }
    }

    /// Create version+1 with edited text/media. Original stays untouched.
    pub fn new_version(
        &self,
        text: Option<String>,
        media: Option<Vec<MediaAsset>>,
        ai_label: Option<AiLabel>,
    ) -> Result<PublishingPackage, PackageError> {
        let mut v = self.source.clone();
        v.version = self.source.version + 1; // always increasing
        if let Some(t) = text {
            v.text = t;
        }
        if let Some(m) = media {
            v.media = m;
        }
        if let Some(l) = ai_label {
            v.ai_label = l;
        }
        v.approval_status = ApprovalStatus::Pending; // new version needs re-approval
        v.created_at = Utc::now();
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn label() -> AiLabel {
        AiLabel::ai_generated(vec![json!({"model": "m", "kind": "llm"})])
    }

    fn sample_pkg() -> PublishingPackage {
        let media = vec![MediaAsset {
            asset_id: Uuid::new_v4(),
            mime: "video/mp4".into(),
            bytes: 3,
            sha256: MediaAsset::hash_of(b"abc"),
            storage_ref: "s3://bucket/k1".into(),
            duration_secs: Some(12.0),
            width: Some(1080),
            height: Some(1920),
        }];
        PublishingPackage::create(
            Uuid::new_v4(),
            "agency-1",
            "brand-1",
            "youtube",
            "acc-1",
            "publish_video",
            "hello",
            media,
            label(),
            json!({}),
        )
        .expect("create")
    }

    #[test]
    fn package_is_immutable_and_versions_increase() {
        let p1 = sample_pkg();
        assert_eq!(p1.version(), 1);
        let editor = PackageEditor::new(&p1);
        let p2 = editor
            .new_version(Some("edited".into()), None, None)
            .expect("v2");
        assert_eq!(p2.version(), 2);
        assert_eq!(p2.text(), "edited");
        assert_eq!(p2.approval_status(), ApprovalStatus::Pending);
        // original untouched
        assert_eq!(p1.version(), 1);
        assert_eq!(p1.text(), "hello");
    }

    #[test]
    fn media_hash_integrity() {
        let p = sample_pkg();
        let asset_id = p.media()[0].asset_id;
        assert!(
            p.verify_media_hashes(&[(asset_id, b"abc".to_vec())])
                .is_ok()
        );
        assert!(matches!(
            p.verify_media_hashes(&[(asset_id, b"tampered".to_vec())]),
            Err(PackageError::HashMismatch { .. })
        ));
    }

    #[test]
    fn ai_label_required_with_provenance() {
        let media = vec![];
        // AI generated but no tools and no C2PA => blocked
        let bad = AiLabel {
            is_ai_generated: true,
            tools: vec![],
            c2pa_manifest_ref: None,
            platform_requirements: vec![],
        };
        assert!(matches!(
            PublishingPackage::create(
                Uuid::new_v4(),
                "a",
                "b",
                "youtube",
                "acc",
                "publish_video",
                "t",
                media.clone(),
                bad,
                json!({})
            ),
            Err(PackageError::MissingAiLabel)
        ));
        // human-made needs no label
        assert!(
            PublishingPackage::create(
                Uuid::new_v4(),
                "a",
                "b",
                "youtube",
                "acc",
                "publish_video",
                "t",
                media,
                AiLabel::human_made(),
                json!({})
            )
            .is_ok()
        );
    }
}
