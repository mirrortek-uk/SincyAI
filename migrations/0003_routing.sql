-- 0003_routing: capability matrix, routing overrides, execution route audit
-- (FR-019, TDD 7 / 11.1). channel_capabilities is PLATFORM-LEVEL: no agency_id,
-- no tenant RLS; writes restricted to Platform Admin (enforced in app layer +
-- a dedicated admin role in 0005).

BEGIN;

-- platform-level: one row per platform x action x route
CREATE TABLE IF NOT EXISTS channel_capabilities (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    platform        TEXT NOT NULL,          -- youtube, instagram, xiaohongshu ...
    action          TEXT NOT NULL,          -- publish_video, schedule, reply_comment, read_metrics ...
    route           TEXT NOT NULL CHECK (route IN ('api', 'device', 'assisted')),
    status          TEXT NOT NULL CHECK (status IN ('verified', 'unverified', 'degraded', 'unsupported')),
    constraints     JSONB NOT NULL DEFAULT '{}'::jsonb,
    priority        INTEGER NOT NULL DEFAULT 100,   -- lower = preferred
    verified_at     TIMESTAMPTZ,
    evidence_ref    TEXT,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (platform, action, route)
);

-- tenant/account level overrides (FR-019): e.g. disable device route for a client
CREATE TABLE IF NOT EXISTS routing_overrides (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agency_id   UUID NOT NULL REFERENCES agencies (id),
    account_id  UUID,                        -- optional: per social account
    platform    TEXT NOT NULL,
    action      TEXT NOT NULL,
    route       TEXT NOT NULL CHECK (route IN ('api', 'device', 'assisted')),
    disabled    BOOLEAN NOT NULL DEFAULT true,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT routing_overrides_tenant_key UNIQUE (agency_id, id),
    UNIQUE (agency_id, account_id, platform, action, route)
);
ALTER TABLE routing_overrides ENABLE ROW LEVEL SECURITY;
ALTER TABLE routing_overrides FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON routing_overrides;
CREATE POLICY tenant_isolation ON routing_overrides
    USING (agency_id::text = current_setting('app.current_agency_id', true))
    WITH CHECK (agency_id::text = current_setting('app.current_agency_id', true));

-- audit: every route decision with skipped paths and reasons (TDD 7.2)
CREATE TABLE IF NOT EXISTS execution_routes (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agency_id       UUID NOT NULL REFERENCES agencies (id),
    task_id         UUID NOT NULL,
    platform        TEXT NOT NULL,
    action          TEXT NOT NULL,
    selected_route  TEXT NOT NULL CHECK (selected_route IN ('api', 'device', 'assisted')),
    capability_status TEXT NOT NULL,
    priority        INTEGER NOT NULL,
    skipped         JSONB NOT NULL DEFAULT '[]'::jsonb,   -- [{route, reason}]
    fallback_events JSONB NOT NULL DEFAULT '[]'::jsonb,   -- degradation history
    decided_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT execution_routes_tenant_key UNIQUE (agency_id, id)
);
ALTER TABLE execution_routes ENABLE ROW LEVEL SECURITY;
ALTER TABLE execution_routes FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON execution_routes;
CREATE POLICY tenant_isolation ON execution_routes
    USING (agency_id::text = current_setting('app.current_agency_id', true))
    WITH CHECK (agency_id::text = current_setting('app.current_agency_id', true));

CREATE INDEX IF NOT EXISTS idx_execution_routes_task ON execution_routes (agency_id, task_id);
CREATE INDEX IF NOT EXISTS idx_capabilities_lookup ON channel_capabilities (platform, action);

-- seed: YouTube capability matrix (structure per PRD 5.3; statuses marked
-- unverified until week-0 verification — never claimed as verified)
INSERT INTO channel_capabilities (platform, action, route, status, priority, constraints, evidence_ref)
VALUES
    ('youtube', 'publish_video', 'api', 'unverified', 10, '{"quota_cost": 1600, "max_bytes": 268435456}', 'PRD 5.1 quota doc 2026-11'),
    ('youtube', 'schedule',      'api', 'unverified', 10, '{}', NULL),
    ('youtube', 'reply_comment', 'api', 'unverified', 10, '{}', NULL),
    ('youtube', 'read_metrics',  'api', 'unverified', 10, '{}', NULL),
    ('youtube', 'publish_video', 'device', 'unverified', 20, '{"requires": ["automation_consent", "adapter"]}', NULL),
    ('youtube', 'publish_video', 'assisted', 'verified', 30, '{}', 'always available fallback')
ON CONFLICT (platform, action, route) DO NOTHING;

COMMIT;
