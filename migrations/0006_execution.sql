-- 0006_execution: device tasks, append-only events, idempotency, kill switches
-- (FR-011/015/025, TDD 14, WP-09/WP-14). Tenant-scoped; kill_switches supports
-- a GLOBAL scope (agency_id NULL) restricted to Platform Admin in app layer.

BEGIN;

-- one row per external side effect attempt key (TDD 11.2)
CREATE TABLE IF NOT EXISTS idempotency_records (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agency_id       UUID NOT NULL REFERENCES agencies (id),
    idempotency_key TEXT NOT NULL,
    task_id         UUID,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT idempotency_records_tenant_key UNIQUE (agency_id, id),
    UNIQUE (agency_id, idempotency_key)
);
ALTER TABLE idempotency_records ENABLE ROW LEVEL SECURITY;
ALTER TABLE idempotency_records FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON idempotency_records;
CREATE POLICY tenant_isolation ON idempotency_records
    USING (agency_id::text = current_setting('app.current_agency_id', true))
    WITH CHECK (agency_id::text = current_setting('app.current_agency_id', true));

-- device/publishing execution tasks with lease + state machine fields (TDD 14.2)
CREATE TABLE IF NOT EXISTS device_tasks (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agency_id       UUID NOT NULL REFERENCES agencies (id),
    brand_id        UUID,
    idempotency_key TEXT NOT NULL,
    state           TEXT NOT NULL DEFAULT 'RECEIVED',
    attempt         INTEGER NOT NULL DEFAULT 0,
    lease_expiry    TIMESTAMPTZ,
    deadline        TIMESTAMPTZ,
    policy_version  TEXT,
    route_id        UUID,
    adapter_version INTEGER,
    package_id      UUID,
    package_version INTEGER,
    side_effect_done BOOLEAN NOT NULL DEFAULT false,  -- 越过 side_effect 后禁止自动重试/降级
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT device_tasks_tenant_key UNIQUE (agency_id, id),
    UNIQUE (agency_id, idempotency_key)
);
ALTER TABLE device_tasks ENABLE ROW LEVEL SECURITY;
ALTER TABLE device_tasks FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON device_tasks;
CREATE POLICY tenant_isolation ON device_tasks
    USING (agency_id::text = current_setting('app.current_agency_id', true))
    WITH CHECK (agency_id::text = current_setting('app.current_agency_id', true));

-- append-only event log; current state is a projection (TDD 11.2)
CREATE TABLE IF NOT EXISTS task_events (
    id          BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    agency_id   UUID NOT NULL REFERENCES agencies (id),
    task_id     UUID NOT NULL,
    event       TEXT NOT NULL,
    from_state  TEXT,
    to_state    TEXT NOT NULL,
    detail      JSONB NOT NULL DEFAULT '{}'::jsonb,
    at          TIMESTAMPTZ NOT NULL DEFAULT now()
);
ALTER TABLE task_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE task_events FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON task_events;
CREATE POLICY tenant_isolation ON task_events
    USING (agency_id::text = current_setting('app.current_agency_id', true))
    WITH CHECK (agency_id::text = current_setting('app.current_agency_id', true));
CREATE INDEX IF NOT EXISTS idx_task_events_task ON task_events (agency_id, task_id, id);

-- transactional outbox (ADR-007)
CREATE TABLE IF NOT EXISTS outbox_events (
    id          BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    agency_id   UUID REFERENCES agencies (id),
    aggregate   TEXT NOT NULL,
    event_type  TEXT NOT NULL,
    payload     JSONB NOT NULL,
    published   BOOLEAN NOT NULL DEFAULT false,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- kill switches (FR-025): scope brand|client|agency|global
-- global rows have agency_id NULL (platform-level, Platform Admin only)
CREATE TABLE IF NOT EXISTS kill_switches (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agency_id   UUID REFERENCES agencies (id),   -- NULL = global
    scope       TEXT NOT NULL CHECK (scope IN ('brand', 'client', 'agency', 'global')),
    target_id   UUID,                             -- brand_id / client_id / agency_id per scope
    active      BOOLEAN NOT NULL DEFAULT true,
    reason      TEXT,
    created_by  TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT kill_switches_tenant_key UNIQUE (agency_id, id)
);
-- tenant rows: RLS; global rows (agency_id NULL): visible to all, writable by admin only (app layer)
ALTER TABLE kill_switches ENABLE ROW LEVEL SECURITY;
ALTER TABLE kill_switches FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON kill_switches;
CREATE POLICY tenant_isolation ON kill_switches
    USING (
        agency_id IS NULL
        OR agency_id::text = current_setting('app.current_agency_id', true)
    )
    WITH CHECK (
        agency_id IS NULL
        OR agency_id::text = current_setting('app.current_agency_id', true)
    );
CREATE INDEX IF NOT EXISTS idx_kill_switches_active ON kill_switches (active) WHERE active;

COMMIT;
