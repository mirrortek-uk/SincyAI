//! Real X (Twitter) API v2 adapter — PRD 5.3: X publishes via API (paid tier).
//!
//! - POST /2/tweets creates a text post (OAuth 2.0 user context bearer token
//!   from the bound account's vault:// oauth_ref; never plaintext in DB).
//! - Optional single media attachment via POST /2/media/upload (raw bytes).
//! - GET /2/tweets/:id verifies the post exists (FR-015).
//!
//! Error taxonomy: 401/403/429/4xx => NoSideEffectFailure (tweet not created,
//! safe to degrade); network/5xx after the request was sent => Uncertain
//! (the post may exist — never retry, human verifies).

use async_trait::async_trait;
use serde_json::{Value, json};
use sincy_common::AppError;
use sincy_routing::action::{
    CapabilityDescriptor, ChannelAction, ExecCtx, ExecOutcome, Precheck, PublishingPackageRef,
    VerifyOutcome,
};
use sincy_routing::capability::Route;
use sqlx::Row;

pub const X_API_BASE: &str = "https://api.x.com/2";
pub const X_MAX_CHARS: usize = 280;
pub const QUOTA_COST_TWEET: i64 = 1;

use super::youtube::MediaBlob;

pub struct XAdapter {
    access_token: String,
    http: reqwest::Client,
    /// overridable for tests
    pub api_base: String,
    pub quota_cost: i64,
    pub text: String,
    pub media: Option<MediaBlob>,
}

impl XAdapter {
    pub fn new(access_token: String) -> Self {
        Self {
            access_token,
            http: reqwest::Client::new(),
            api_base: X_API_BASE.to_string(),
            quota_cost: QUOTA_COST_TWEET,
            text: String::new(),
            media: None,
        }
    }

    /// Resolve the OAuth 2.0 user-context access token for a bound account.
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
            Some(r) => super::youtube::resolve_vault_ref(r).ok_or_else(|| {
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

    /// Upload media (v2 simple upload, single attachment) -> media_id string.
    async fn upload_media(&self, media: &MediaBlob) -> Result<String, ExecOutcome> {
        let resp = self
            .http
            .post(format!("{}/media/upload", self.api_base))
            .bearer_auth(&self.access_token)
            .header("Content-Type", &media.mime)
            .header("Content-Length", media.bytes.len())
            .body(media.bytes.clone())
            .send()
            .await;
        match resp {
            Ok(r) if r.status().is_success() => {
                let v: Value = r.json().await.unwrap_or_else(|_| json!({}));
                match v["data"]["id"].as_str() {
                    Some(id) => Ok(id.to_string()),
                    None => Err(ExecOutcome::NoSideEffectFailure(AppError::no_side_effect(
                        "media_upload_bad_response",
                        "no media id in response",
                    ))),
                }
            }
            Ok(r) => Err(ExecOutcome::NoSideEffectFailure(AppError::no_side_effect(
                "media_upload_rejected",
                format!("media upload returned {}", r.status().as_u16()),
            ))),
            Err(e) => Err(ExecOutcome::NoSideEffectFailure(AppError::retryable(
                "network",
                format!("media upload network error: {e}"),
            ))),
        }
    }
}

#[async_trait]
impl ChannelAction for XAdapter {
    fn capability(&self) -> CapabilityDescriptor {
        CapabilityDescriptor {
            platform: "x".into(),
            action: "publish_post".into(),
            route: Route::Api,
            constraints: json!({
                "quota_cost": self.quota_cost,
                "max_chars": X_MAX_CHARS,
                "media": ["image/jpeg", "image/png", "video/mp4"],
            }),
        }
    }

    async fn precheck(&self, _ctx: &ExecCtx, _pkg: &PublishingPackageRef) -> Precheck {
        if self.access_token.is_empty() {
            return Precheck::blocked("no access token");
        }
        if self.text.trim().is_empty() && self.media.is_none() {
            return Precheck::blocked("tweet needs text or media");
        }
        if self.text.chars().count() > X_MAX_CHARS {
            return Precheck::blocked("text exceeds 280 weighted characters");
        }
        if let Some(m) = &self.media
            && m.bytes.len() > 5_242_880
        {
            return Precheck::blocked("media exceeds 5 MiB (v2 simple upload limit)");
        }
        Precheck::ok()
    }

    async fn execute(&self, ctx: &ExecCtx, pkg: &PublishingPackageRef) -> ExecOutcome {
        let mut payload = json!({ "text": self.text });
        if let Some(m) = &self.media {
            match self.upload_media(m).await {
                Ok(media_id) => {
                    payload["media"] = json!({ "media_ids": [media_id] });
                }
                Err(outcome) => return outcome,
            }
        }
        let resp = self
            .http
            .post(format!("{}/tweets", self.api_base))
            .bearer_auth(&self.access_token)
            .json(&payload)
            .send()
            .await;
        match resp {
            Ok(r) if r.status().is_success() => {
                let v: Value = r.json().await.unwrap_or_else(|_| json!({}));
                let id = v["data"]["id"].as_str().map(|s| s.to_string());
                ExecOutcome::Success {
                    external_content_id: id.clone(),
                    receipt: json!({
                        "platform": "x",
                        "api": "POST /2/tweets",
                        "tweet_id": id,
                        "edit_history_tweet_ids": v["data"]["edit_history_tweet_ids"],
                        "package_id": pkg.package_id,
                        "package_version": pkg.version,
                        "idempotency_key": ctx.idempotency_key,
                        "quota_cost": self.quota_cost,
                    }),
                }
            }
            Ok(r) => {
                let status = r.status().as_u16();
                let body = r.text().await.unwrap_or_default();
                // 4xx: the tweet was NOT created -> safe to degrade/retry-elsewhere
                if (400..500).contains(&status) {
                    ExecOutcome::NoSideEffectFailure(AppError::no_side_effect(
                        "tweet_rejected",
                        format!("X returned {status}: {}", &body[..body.len().min(300)]),
                    ))
                } else {
                    // 5xx: request reached the server, post may exist -> Uncertain
                    ExecOutcome::Uncertain(AppError::uncertain(
                        "tweet_result_unknown",
                        format!("X returned {status} after request sent"),
                    ))
                }
            }
            Err(e) => ExecOutcome::Uncertain(AppError::uncertain(
                "tweet_result_unknown",
                format!("network error after request sent: {e}"),
            )),
        }
    }

    async fn verify(&self, _ctx: &ExecCtx, attempt: &str) -> VerifyOutcome {
        let resp = self
            .http
            .get(format!("{}/tweets/{}", self.api_base, attempt))
            .bearer_auth(&self.access_token)
            .query(&[("tweet.fields", "public_metrics,created_at")])
            .send()
            .await;
        match resp {
            Ok(r) if r.status().is_success() => {
                let v: Value = r.json().await.unwrap_or_else(|_| json!({}));
                VerifyOutcome {
                    verified: v["data"]["id"].as_str().is_some(),
                    external_content_id: v["data"]["id"].as_str().map(|s| s.to_string()),
                    evidence: json!({
                        "platform": "x",
                        "tweet_id": attempt,
                        "public_metrics": v["data"]["public_metrics"],
                        "created_at": v["data"]["created_at"],
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

#[cfg(test)]
mod tests {
    use super::*;
    use sincy_routing::action::ExecCtx;

    fn ctx() -> ExecCtx {
        ExecCtx {
            agency_id: "a".into(),
            brand_id: "b".into(),
            account_id: "acc".into(),
            task_id: "t".into(),
            route_id: "r".into(),
            idempotency_key: "idem-x-1".into(),
        }
    }

    fn pkg() -> PublishingPackageRef {
        PublishingPackageRef {
            package_id: "p".into(),
            version: 1,
            platform: "x".into(),
            action: "publish_post".into(),
        }
    }

    #[tokio::test]
    async fn precheck_blocks_empty_and_long_text() {
        let a = XAdapter::new("tok".into());
        let p = a.precheck(&ctx(), &pkg()).await;
        assert!(!p.ok);
        let mut a2 = XAdapter::new("tok".into());
        a2.text = "x".repeat(281);
        assert!(!a2.precheck(&ctx(), &pkg()).await.ok);
        let mut a3 = XAdapter::new("tok".into());
        a3.text = "hello X".into();
        assert!(a3.precheck(&ctx(), &pkg()).await.ok);
    }

    /// Tiny canned HTTP server to exercise the real request path offline.
    async fn canned_server(status: u16, body: &'static str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 65536];
            let _ = sock.read(&mut buf).await;
            let resp = format!(
                "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = sock.write_all(resp.as_bytes()).await;
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn execute_success_returns_tweet_id() {
        let base = canned_server(200, r#"{"data":{"id":"1234567890","text":"hi"}}"#).await;
        let mut a = XAdapter::new("tok".into());
        a.api_base = base;
        a.text = "hi".into();
        match a.execute(&ctx(), &pkg()).await {
            ExecOutcome::Success {
                external_content_id,
                receipt,
            } => {
                assert_eq!(external_content_id.as_deref(), Some("1234567890"));
                assert_eq!(receipt["platform"], "x");
                assert_eq!(receipt["tweet_id"], "1234567890");
            }
            _ => panic!("expected success"),
        }
    }

    #[tokio::test]
    async fn execute_403_is_no_side_effect() {
        let base = canned_server(403, r#"{"detail":"duplicate content"}"#).await;
        let mut a = XAdapter::new("tok".into());
        a.api_base = base;
        a.text = "dup".into();
        match a.execute(&ctx(), &pkg()).await {
            ExecOutcome::NoSideEffectFailure(e) => assert_eq!(e.code, "tweet_rejected"),
            _ => panic!("403 must be NoSideEffectFailure"),
        }
    }

    #[tokio::test]
    async fn execute_500_is_uncertain() {
        let base = canned_server(500, r#"{"detail":"internal"}"#).await;
        let mut a = XAdapter::new("tok".into());
        a.api_base = base;
        a.text = "maybe".into();
        match a.execute(&ctx(), &pkg()).await {
            ExecOutcome::Uncertain(e) => assert_eq!(e.code, "tweet_result_unknown"),
            _ => panic!("5xx must be Uncertain, never retried"),
        }
    }
}
