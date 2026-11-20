-- 0010_analytics: platform metrics, performance snapshots, attribution (FR-016, WP-14).
-- Tenant-scoped with RLS. Metric口径 must be labeled (source field).

BEGIN;

CREATE TABLE IF NOT EXISTS platform_metrics (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agency_id       UUID NOT NULL REFERENCES agencies (id),
    platform        TEXT NOT NULL,
    external_content_id TEXT NOT NULL,      -- platform-side id (per platform/account唯一)
    account_id      UUID,
    metric_date     DATE NOT NULL,
    metrics         JSONB NOT NULL,           -- views, likes, comments, shares ...
    source          TEXT NOT NULL DEFAULT 'api' CHECK (source IN ('api', 'device', 'manual')),
    fetched_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT platform_metrics_tenant_key UNIQUE (agency_id, id),
    UNIQUE (agency_id, platform, external_content_id, metric_date, source)
);
ALTER TABLE platform_metrics ENABLE ROW LEVEL SECURITY;
ALTER TABLE platform_metrics FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON platform_metrics;
CREATE POLICY tenant_isolation ON platform_metrics
    USING (agency_id::text = current_setting('app.current_agency_id', true))
    WITH CHECK (agency_id::text = current_setting('app.current_agency_id', true));

CREATE TABLE IF NOT EXISTS content_performance_snapshots (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agency_id       UUID NOT NULL REFERENCES agencies (id),
    content_item_id UUID NOT NULL,
    content_version INTEGER NOT NULL,
    package_id      UUID,
    platform        TEXT NOT NULL,
    external_content_id TEXT,
    snapshot_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    metrics         JSONB NOT NULL,
    CONSTRAINT cps_tenant_key UNIQUE (agency_id, id)
);
ALTER TABLE content_performance_snapshots ENABLE ROW LEVEL SECURITY;
ALTER TABLE content_performance_snapshots FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON content_performance_snapshots;
CREATE POLICY tenant_isolation ON content_performance_snapshots
    USING (agency_id::text = current_setting('app.current_agency_id', true))
    WITH CHECK (agency_id::text = current_setting('app.current_agency_id', true));

COMMIT;
