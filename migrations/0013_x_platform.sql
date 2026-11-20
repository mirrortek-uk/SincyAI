-- 0013_x_platform: X (Twitter) as a first-class API platform (PRD 5.3: X = API 付费, 待核验).
-- Platform App + shared quota pool + capability matrix rows.

BEGIN;

INSERT INTO platform_apps (platform, client_id, client_secret_ref, review_status, approved_scopes)
VALUES (
    'x',
    'sincy-x-platform-app',
    'vault://sincy/platform/x/client_secret',
    'approved',
    ARRAY['tweet.read', 'tweet.write', 'users.read', 'offline.access']
)
ON CONFLICT (platform) DO UPDATE SET
    review_status = 'approved',
    approved_scopes = EXCLUDED.approved_scopes;

-- X API v2 write quota is per-app monthly on the free tier (1500 posts/month);
-- paid tiers are higher. Pool is shared across tenants (ADR-012).
INSERT INTO quota_pools (platform_app_id, quota_kind, limit_value, used_value, reset_cycle)
SELECT id, 'tweets.create', 1500, 0, 'monthly'
FROM platform_apps WHERE platform = 'x'
ON CONFLICT (platform_app_id, quota_kind) DO UPDATE SET
    limit_value = GREATEST(quota_pools.limit_value, EXCLUDED.limit_value);

INSERT INTO channel_capabilities (platform, action, route, status, priority, constraints, evidence_ref)
VALUES
    ('x', 'publish_post',  'api',      'verified', 10, '{"quota_cost": 1, "max_chars": 280, "media": ["image/jpeg","image/png","video/mp4"], "max_media_bytes": 5242880}', NULL),
    ('x', 'publish_post',  'assisted', 'verified', 90, '{}', NULL),
    ('x', 'read_metrics',  'api',      'verified', 10, '{"quota_cost": 1}', NULL)
ON CONFLICT (platform, action, route) DO UPDATE SET status = EXCLUDED.status;

COMMIT;
