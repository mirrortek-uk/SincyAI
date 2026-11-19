-- 0004_platform_quota: Platform App registry + shared quota ledger (FR-020, TDD 8, ADR-012).
-- platform_apps / quota_pools are PLATFORM-LEVEL (no agency_id, no tenant RLS,
-- Platform Admin only). quota_ledger / quota_allocations carry agency_id.

BEGIN;

CREATE TABLE IF NOT EXISTS platform_apps (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    platform        TEXT NOT NULL UNIQUE,        -- youtube, ...
    client_id       TEXT NOT NULL,               -- OAuth client id (secret stays in Vault)
    client_secret_ref TEXT NOT NULL,             -- vault://... reference ONLY (TDD 15)
    review_status   TEXT NOT NULL DEFAULT 'not_submitted'
                    CHECK (review_status IN ('not_submitted', 'in_review', 'approved', 'rejected')),
    approved_scopes TEXT[] NOT NULL DEFAULT '{}',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- one row per Platform App per quota kind
CREATE TABLE IF NOT EXISTS quota_pools (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    platform_app_id UUID NOT NULL REFERENCES platform_apps (id),
    quota_kind      TEXT NOT NULL,               -- e.g. videos.insert, generic_units
    limit_value     BIGINT NOT NULL,
    used_value      BIGINT NOT NULL DEFAULT 0 CHECK (used_value <= limit_value),
    reset_cycle     TEXT NOT NULL DEFAULT 'daily',  -- daily | hourly | rolling
    reset_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (platform_app_id, quota_kind)
);

-- reserve / confirm / release; balance updates MUST happen in the same
-- transaction as the ledger row (Guide 6: 防止超发)
CREATE TABLE IF NOT EXISTS quota_ledger (
    reservation_id  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agency_id       UUID NOT NULL REFERENCES agencies (id),
    quota_pool_id   UUID NOT NULL REFERENCES quota_pools (id),
    amount          BIGINT NOT NULL CHECK (amount > 0),
    state           TEXT NOT NULL DEFAULT 'reserved'
                    CHECK (state IN ('reserved', 'confirmed', 'released')),
    task_id         UUID,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at     TIMESTAMPTZ,
    CONSTRAINT quota_ledger_tenant_key UNIQUE (agency_id, reservation_id)
);
ALTER TABLE quota_ledger ENABLE ROW LEVEL SECURITY;
ALTER TABLE quota_ledger FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON quota_ledger;
CREATE POLICY tenant_isolation ON quota_ledger
    USING (agency_id::text = current_setting('app.current_agency_id', true))
    WITH CHECK (agency_id::text = current_setting('app.current_agency_id', true));

-- per-agency guaranteed share + borrow limit of a pool (FR-020 allocation)
CREATE TABLE IF NOT EXISTS quota_allocations (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agency_id       UUID NOT NULL REFERENCES agencies (id),
    quota_pool_id   UUID NOT NULL REFERENCES quota_pools (id),
    guaranteed      BIGINT NOT NULL DEFAULT 0 CHECK (guaranteed >= 0),
    borrow_limit    BIGINT NOT NULL DEFAULT 0 CHECK (borrow_limit >= 0),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT quota_allocations_tenant_key UNIQUE (agency_id, id),
    UNIQUE (agency_id, quota_pool_id)
);
ALTER TABLE quota_allocations ENABLE ROW LEVEL SECURITY;
ALTER TABLE quota_allocations FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON quota_allocations;
CREATE POLICY tenant_isolation ON quota_allocations
    USING (agency_id::text = current_setting('app.current_agency_id', true))
    WITH CHECK (agency_id::text = current_setting('app.current_agency_id', true));

CREATE INDEX IF NOT EXISTS idx_quota_ledger_pool ON quota_ledger (quota_pool_id, state);

COMMIT;

-- idempotent reservations: one active reservation per (agency, task)
CREATE UNIQUE INDEX IF NOT EXISTS idx_quota_ledger_task_active
    ON quota_ledger (agency_id, task_id)
    WHERE task_id IS NOT NULL AND state = 'reserved';
