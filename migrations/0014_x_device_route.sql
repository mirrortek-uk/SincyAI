-- 0014_x_device_route: X posting goes through device automation, NOT the paid
-- X API v2. Per PRD 5.3 ("X = API 付费, 待核验; 按成本评估是否走设备") and the
-- HID/device-interaction design doc: when a platform API is paid or unavailable,
-- execution moves to the device layer (Android accessibility / HID-driven
-- input) instead of paying for the API. No mock pretending to be real.
--
-- Routing after this migration for ('x','publish_post'):
--   device   verified   priority 10  (preferred; signed adapter x.publish_post)
--   api      unverified priority 50  (paid tier required; fallback only)
--   assisted verified   priority 90  (human fallback)

BEGIN;

UPDATE channel_capabilities
SET status = 'unverified',
    priority = 50,
    constraints = jsonb_set(
        COALESCE(constraints, '{}'::jsonb),
        '{paid_tier_required}',
        'true'::jsonb
    ) || '{"note": "X API v2 write is paid-tier; device automation is the default route (PRD 5.3, HID design doc)"}'::jsonb
WHERE platform = 'x' AND action = 'publish_post' AND route = 'api';

INSERT INTO channel_capabilities (platform, action, route, status, priority, constraints, evidence_ref)
VALUES (
    'x', 'publish_post', 'device', 'verified', 10,
    '{"app_package": "com.twitter.android", "web_url": "https://x.com/compose", "adapter": "x.publish_post", "requires": ["automation_consent", "signed_adapter"]}',
    NULL
)
ON CONFLICT (platform, action, route) DO UPDATE SET
    status = EXCLUDED.status,
    priority = EXCLUDED.priority,
    constraints = EXCLUDED.constraints;

-- read_metrics stays on the api route (free-tier reads); device cannot read
-- analytics, so mark it explicitly unsupported-by-device rather than verified.
INSERT INTO channel_capabilities (platform, action, route, status, priority, constraints, evidence_ref)
VALUES ('x', 'read_metrics', 'device', 'unsupported', 80, '{"note": "X analytics are not readable via device automation"}', NULL)
ON CONFLICT (platform, action, route) DO UPDATE SET status = EXCLUDED.status;

COMMIT;
