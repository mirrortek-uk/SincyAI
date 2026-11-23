//! Real YouTube Data API v3 adapter (WP-06 production path, replaces the mock
//! once platform审核 passes). videos.insert via resumable upload.
//!
//! Credentials: the per-account OAuth access token is referenced from
//! social_accounts.oauth_ref as a vault:// URI (TDD 15 — never plaintext in
//! DB). Vault resolution: `vault://sincy/<path>` maps to env
//! SINCY_VAULT_<PATH uppercased, / and - replaced by _>; a direct
//! SINCY_YOUTUBE_ACCESS_TOKEN env override is accepted for local dev only.
//!
//! Error taxonomy (Guide 3.2): failures BEFORE any byte is sent are
//! NoSideEffectFailure (eligible for degradation); any failure at/after the
//! media PUT is Uncertain — never retried, never degraded, human verifies.

use async_trait::async_trait;
use serde_json::{Value, json};
use sincy_common::AppError;
use sincy_routing::action::{
    CapabilityDescriptor, ChannelAction, ExecCtx, ExecOutcome, Precheck, PublishingPackageRef,
    VerifyOutcome,
};
use sincy_routing::capability::Route;
use sqlx::Row;

pub const YOUTUBE_UPLOAD_BASE: &str = "https://www.googleapis.com/upload/youtube/v3/videos";
pub const YOUTUBE_API_BASE: &str = "https://www.googleapis.com/youtube/v3/videos";
/// Default quota cost of a `videos.insert` call, used only as a fallback when
/// the channel_capabilities row carries no `quota_cost`. Google cut this from
/// ~1600 to ~100 units on 2026-12-04 and moved uploads into a separate quota
/// bucket on 2026-06-01; the authoritative value lives in the capability
/// matrix (`constraints.quota_cost`) so ops can correct it without a code
/// release. Verify the live cost in Cloud Console.
pub const QUOTA_COST_VIDEOS_INSERT: i64 = 100;

/// A media blob resolved from the object store by the caller.
pub struct MediaBlob {
    pub bytes: Vec<u8>,
    pub mime: String,
    pub filename: String,
}

pub struct YouTubeAdapter {
    access_token: String,
    http: reqwest::Client,
    pub quota_cost: i64,
    /// media to upload; None => metadata-only precheck failure (video required)
    pub media: Option<MediaBlob>,
    pub title: String,
    pub description: String,
    pub privacy_status: String,
}

impl YouTubeAdapter {
    pub fn new(access_token: String) -> Self {
        Self {
            access_token,
            http: reqwest::Client::new(),
            quota_cost: QUOTA_COST_VIDEOS_INSERT,
            media: None,
            title: String::new(),
            description: String::new(),
            privacy_status: "private".into(),
        }
    }

    /// Resolve the OAuth access token for a bound social account.
    /// account row's oauth_ref is a vault:// reference; plaintext never stored.
    pub async fn for_account(
        pool: &sqlx::PgPool,
        agency_id: &uuid::Uuid,
        account_id: &uuid::Uuid,
    ) -> Result<Self, AppError> {
        let row = sqlx::query(
            "SELECT oauth_ref, state FROM social_accounts WHERE id = $1 AND agency_id = $2",
        )
        .bind(account_id)
        .bind(agency_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::retryable("db", e.to_string()))?
        .ok_or_else(|| AppError::non_retryable("account_not_found", "social account not bound"))?;
        let state: String = row.get("state");
        if state != "connected" {
            return Err(AppError::needs_user(
                "account_not_authorized",
                format!("account state is {state}; complete OAuth in console first"),
            ));
        }
        let oauth_ref: Option<String> = row.get("oauth_ref");
        let token = match oauth_ref.as_deref() {
            Some(r) => resolve_vault_ref(r).ok_or_else(|| {
                AppError::needs_user(
                    "credential_unavailable",
                    format!("vault reference {r} not resolvable in this environment"),
                )
            })?,
            None => {
                return Err(AppError::needs_user(
                    "account_not_authorized",
                    "no oauth_ref",
                ));
            }
        };
        Ok(Self::new(token))
    }
}

/// vault://sincy/youtube/abc -> env SINCY_VAULT_SINCY_YOUTUBE_ABC
pub fn resolve_vault_ref(vault_uri: &str) -> Option<String> {
    if let Some(path) = vault_uri.strip_prefix("vault://") {
        let key = format!(
            "SINCY_VAULT_{}",
            path.to_uppercase().replace(['/', '-'], "_")
        );
        if let Ok(v) = std::env::var(&key) {
            return Some(v);
        }
    }
    // local dev convenience
    std::env::var("SINCY_YOUTUBE_ACCESS_TOKEN").ok()
}

#[async_trait]
impl ChannelAction for YouTubeAdapter {
    fn capability(&self) -> CapabilityDescriptor {
        CapabilityDescriptor {
            platform: "youtube".into(),
            action: "publish_video".into(),
            route: Route::Api,
            constraints: json!({
                "quota_cost": self.quota_cost,
                "media": ["video/mp4"],
                "max_bytes": 268435456usize,
                "resumable": true,
            }),
        }
    }

    async fn precheck(&self, _ctx: &ExecCtx, _pkg: &PublishingPackageRef) -> Precheck {
        if self.access_token.is_empty() {
            return Precheck::blocked("no access token");
        }
        match &self.media {
            None => Precheck::blocked("publish_video requires a media asset in the package"),
            Some(m) if m.bytes.is_empty() => Precheck::blocked("media asset is empty"),
            Some(m) if m.bytes.len() > 268435456 => Precheck::blocked("media exceeds 256 MiB"),
            Some(_) => Precheck::ok(),
        }
    }

    async fn execute(&self, ctx: &ExecCtx, pkg: &PublishingPackageRef) -> ExecOutcome {
        let media = match &self.media {
            Some(m) => m,
            None => {
                return ExecOutcome::NoSideEffectFailure(AppError::no_side_effect(
                    "media_required",
                    "publish_video requires media in the package",
                ));
            }
        };
        // 1. initiate resumable session — no side effect yet
        let init_body = json!({
            "snippet": { "title": self.title, "description": self.description },
            "status": { "privacyStatus": self.privacy_status, "selfDeclaredMadeForKids": false },
        });
        let init = self
            .http
            .post(format!(
                "{YOUTUBE_UPLOAD_BASE}?uploadType=resumable&part=snippet,status"
            ))
            .bearer_auth(&self.access_token)
            .header("X-Upload-Content-Type", &media.mime)
            .header("X-Upload-Content-Length", media.bytes.len())
            .json(&init_body)
            .send()
            .await;
        let session_uri = match init {
            Ok(resp) if resp.status().is_success() => {
                match resp.headers().get("location").and_then(|v| v.to_str().ok()) {
                    Some(u) => u.to_string(),
                    None => {
                        return ExecOutcome::NoSideEffectFailure(AppError::no_side_effect(
                            "upload_init_failed",
                            "resumable session created but no Location header",
                        ));
                    }
                }
            }
            Ok(resp) => {
                let status = resp.status().as_u16();
                let body = resp.text().await.unwrap_or_default();
                let kind = if status == 401 || status == 403 {
                    "auth_failed"
                } else {
                    "upload_init_rejected"
                };
                return ExecOutcome::NoSideEffectFailure(AppError::no_side_effect(
                    kind,
                    format!(
                        "YouTube init returned {status}: {}",
                        &body[..body.len().min(300)]
                    ),
                ));
            }
            Err(e) => {
                return ExecOutcome::NoSideEffectFailure(AppError::retryable(
                    "network",
                    format!("upload init network error: {e}"),
                ));
            }
        };
        // 2. PUT the media bytes — from here on the side effect may have happened
        let put = self
            .http
            .put(&session_uri)
            .header("Content-Type", &media.mime)
            .header("Content-Length", media.bytes.len())
            .body(media.bytes.clone())
            .send()
            .await;
        match put {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await.unwrap_or_else(|_| json!({}));
                let id = body["id"].as_str().map(|s| s.to_string());
                ExecOutcome::Success {
                    external_content_id: id.clone(),
                    receipt: json!({
                        "platform": "youtube",
                        "api": "videos.insert",
                        "upload_type": "resumable",
                        "video_id": id,
                        "package_id": pkg.package_id,
                        "package_version": pkg.version,
                        "idempotency_key": ctx.idempotency_key,
                        "quota_cost": self.quota_cost,
                    }),
                }
            }
            Ok(resp) => ExecOutcome::Uncertain(AppError::uncertain(
                "upload_result_unknown",
                format!(
                    "media PUT returned {}: {}",
                    resp.status().as_u16(),
                    resp.text().await.unwrap_or_default()
                ),
            )),
            Err(e) => ExecOutcome::Uncertain(AppError::uncertain(
                "upload_result_unknown",
                format!("media PUT network error after bytes sent: {e}"),
            )),
        }
    }

    async fn verify(&self, ctx: &ExecCtx, attempt: &str) -> VerifyOutcome {
        // attempt carries the video_id from the receipt
        let resp = self
            .http
            .get(format!("{YOUTUBE_API_BASE}?part=status&id={attempt}"))
            .bearer_auth(&self.access_token)
            .send()
            .await;
        match resp {
            Ok(r) if r.status().is_success() => {
                let body: Value = r.json().await.unwrap_or_else(|_| json!({}));
                let item = &body["items"][0];
                let upload_status = item["status"]["uploadStatus"].as_str().unwrap_or("");
                let verified = matches!(upload_status, "uploaded" | "processed");
                VerifyOutcome {
                    verified,
                    external_content_id: item["id"].as_str().map(|s| s.to_string()),
                    evidence: json!({
                        "platform": "youtube",
                        "video_id": attempt,
                        "uploadStatus": upload_status,
                        "idempotency_key": ctx.idempotency_key,
                    }),
                }
            }
            _ => VerifyOutcome {
                verified: false,
                external_content_id: None,
                evidence: json!({"error": "verify call failed"}),
            },
        }
    }
}
