//! Mock YouTube API adapter (WP-06).
//!
//! IMPORTANT (Guide 7.6 / DoD): this is a MOCK. It exists so the Cloud pipeline
//! (route -> reserve quota -> execute -> receipt -> analytics) can be built and
//! tested end-to-end BEFORE the real YouTube restricted-scope审核 passes.
//! It must never be presented as real platform capability; the real adapter
//! replaces it and the mock is then disabled outside test environments.

use async_trait::async_trait;
use serde_json::json;

use sincy_routing::action::{
    CapabilityDescriptor, ChannelAction, ExecCtx, ExecOutcome, Precheck, PublishingPackageRef,
    VerifyOutcome,
};
use sincy_routing::capability::Route;

pub struct MockYouTubeAdapter {
    pub quota_pool_id: uuid::Uuid,
    pub quota_cost: i64,
    /// when true, simulate quota exhaustion (WP-06 acceptance: QUOTA_WAIT / degrade)
    pub simulate_quota_exhausted: bool,
}

#[async_trait]
impl ChannelAction for MockYouTubeAdapter {
    fn capability(&self) -> CapabilityDescriptor {
        CapabilityDescriptor {
            platform: "youtube".to_string(),
            action: "publish_video".to_string(),
            route: Route::Api,
            constraints: json!({
                "mock": true,
                "max_bytes": 268435456,
                "quota_cost": self.quota_cost,
            }),
        }
    }

    async fn precheck(&self, _ctx: &ExecCtx, _pkg: &PublishingPackageRef) -> Precheck {
        if self.simulate_quota_exhausted {
            return Precheck::blocked("quota exhausted (mock)");
        }
        Precheck::ok()
    }

    async fn execute(&self, ctx: &ExecCtx, pkg: &PublishingPackageRef) -> ExecOutcome {
        // Mock: no real network call. Returns a deterministic fake receipt.
        // Real adapter would call videos.insert here after quota reserve.
        let external_id = format!("mock-yt-{}", ctx.idempotency_key);
        ExecOutcome::Success {
            external_content_id: Some(external_id.clone()),
            receipt: json!({
                "mock": true,
                "platform": "youtube",
                "package_id": pkg.package_id,
                "package_version": pkg.version,
                "external_content_id": external_id,
                "idempotency_key": ctx.idempotency_key,
            }),
        }
    }

    async fn verify(&self, ctx: &ExecCtx, _attempt: &str) -> VerifyOutcome {
        // Mock verification: the fake receipt is always "verified".
        VerifyOutcome {
            verified: true,
            external_content_id: Some(format!("mock-yt-{}", ctx.idempotency_key)),
            evidence: json!({"mock": true, "verified_via": "mock_receipt"}),
        }
    }
}

/// Guard: refuse to run the mock outside test/dev, so it can never stand in
/// for the real adapter in production (DoD: 不以 mock 冒充真实平台能力).
pub fn assert_not_production(env: &str) {
    assert!(
        env != "production",
        "MockYouTubeAdapter must never run in production"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_execute_returns_receipt_with_idempotency_key() {
        let adapter = MockYouTubeAdapter {
            quota_pool_id: uuid::Uuid::new_v4(),
            quota_cost: 1600,
            simulate_quota_exhausted: false,
        };
        let ctx = ExecCtx {
            agency_id: "a".into(),
            brand_id: "b".into(),
            account_id: "acc".into(),
            task_id: "t".into(),
            route_id: "r".into(),
            idempotency_key: "idem-1".into(),
        };
        let pkg = PublishingPackageRef {
            package_id: "pkg-1".into(),
            version: 1,
            platform: "youtube".into(),
            action: "publish_video".into(),
        };
        match adapter.execute(&ctx, &pkg).await {
            ExecOutcome::Success {
                external_content_id,
                receipt,
            } => {
                assert_eq!(external_content_id.as_deref(), Some("mock-yt-idem-1"));
                assert_eq!(receipt["mock"], true);
            }
            _ => panic!("expected success"),
        }
    }

    #[test]
    fn mock_refuses_production() {
        assert_not_production("local");
        let result = std::panic::catch_unwind(|| assert_not_production("production"));
        assert!(result.is_err());
    }
}
