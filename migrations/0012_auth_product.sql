-- 0012_auth_product: real console/API auth + platform seed data (正式产品化).
-- 1) agency_members gain a password hash (Argon2id); email is globally unique
--    so login is unambiguous.
-- 2) Seed the platform-level registry: one Platform App per platform (ADR-012)
--    + a shared quota pool, so the Execution Router can actually select the API
--    route. Secrets stay as vault:// references ONLY (TDD 15).

BEGIN;

ALTER TABLE agency_members ADD COLUMN IF NOT EXISTS password_hash TEXT;
CREATE UNIQUE INDEX IF NOT EXISTS idx_agency_members_email ON agency_members (email);

-- Platform App registry seed (platform-level, no tenant RLS by design).
INSERT INTO platform_apps (platform, client_id, client_secret_ref, review_status, approved_scopes)
VALUES (
    'youtube',
    'sincy-youtube-platform-app',
    'vault://sincy/platform/youtube/client_secret',
    'approved',
    ARRAY['https://www.googleapis.com/auth/youtube.upload', 'https://www.googleapis.com/auth/youtube.readonly']
)
ON CONFLICT (platform) DO UPDATE SET
    review_status = 'approved',
    approved_scopes = EXCLUDED.approved_scopes;

-- Shared daily quota pool for videos.insert (YouTube charges 1600 units/call).
INSERT INTO quota_pools (platform_app_id, quota_kind, limit_value, used_value, reset_cycle)
SELECT id, 'videos.insert', 100000, 0, 'daily'
FROM platform_apps WHERE platform = 'youtube'
ON CONFLICT (platform_app_id, quota_kind) DO UPDATE SET
    limit_value = GREATEST(quota_pools.limit_value, EXCLUDED.limit_value);

-- Capability matrix: YouTube API route is the verified preferred path for
-- video publishing; device route exists but lower priority; assisted last.
INSERT INTO channel_capabilities (platform, action, route, status, priority, constraints, evidence_ref)
VALUES
    ('youtube', 'publish_video', 'api',      'verified', 10, '{"quota_cost": 1600, "media": ["video/mp4"], "max_bytes": 268435456}', NULL),
    ('youtube', 'publish_video', 'device',   'verified', 50, '{"app_package": "com.xiaohongshu", "adapter": "xiaohongshu.publish_video"}', NULL),
    ('youtube', 'publish_video', 'assisted', 'verified', 90, '{}', NULL),
    ('youtube', 'read_metrics',  'api',      'verified', 10, '{"quota_cost": 1}', NULL)
ON CONFLICT (platform, action, route) DO UPDATE SET status = EXCLUDED.status;

COMMIT;
