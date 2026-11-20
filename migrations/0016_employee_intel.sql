-- 0016_employee_intel: Digital Employee (SINCY-ARCH-DE-001) + World Intelligence
-- Station (WI guide) core tables. Tenant-scoped where agency-owned (RLS),
-- platform-level for the global source/provider registry.

BEGIN;

-- ============ Digital Employee (DE guide §17 data model) ============

CREATE TABLE IF NOT EXISTS digital_employees (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agency_id       UUID NOT NULL REFERENCES agencies (id),
    name            TEXT NOT NULL,
    role            TEXT NOT NULL DEFAULT 'content_operator',
    -- CREATED -> CONFIGURING -> READY -> RUNNING -> PAUSED -> STOPPED
    -- exceptional: WAITING_APPROVAL, DEGRADED, ERROR, ARCHIVED
    status          TEXT NOT NULL DEFAULT 'CREATED'
                    CHECK (status IN ('CREATED','CONFIGURING','READY','RUNNING','PAUSED','STOPPED','WAITING_APPROVAL','DEGRADED','ERROR','ARCHIVED')),
    config_version  INTEGER NOT NULL DEFAULT 1,
    brand_id        UUID REFERENCES brands (id),
    channel         TEXT NOT NULL DEFAULT 'x',   -- primary platform (MVP: x)
    auto_publish    BOOLEAN NOT NULL DEFAULT false, -- false = packages need human approval
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT digital_employees_tenant_key UNIQUE (agency_id, id)
);
ALTER TABLE digital_employees ENABLE ROW LEVEL SECURITY;
ALTER TABLE digital_employees FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON digital_employees;
CREATE POLICY tenant_isolation ON digital_employees
    USING (agency_id::text = current_setting('app.current_agency_id', true))
    WITH CHECK (agency_id::text = current_setting('app.current_agency_id', true));

-- versioned identity (DE §6: identity changes require versioning)
CREATE TABLE IF NOT EXISTS employee_identity_versions (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    employee_id     UUID NOT NULL REFERENCES digital_employees (id),
    version         INTEGER NOT NULL,
    identity        JSONB NOT NULL,      -- name/role/personality/voice/audience/expertise/values/boundaries
    status          TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('draft','active','superseded')),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (employee_id, version)
);
ALTER TABLE employee_identity_versions ENABLE ROW LEVEL SECURITY;
ALTER TABLE employee_identity_versions FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON employee_identity_versions;
CREATE POLICY tenant_isolation ON employee_identity_versions
    USING (EXISTS (SELECT 1 FROM digital_employees e WHERE e.id = employee_id AND e.agency_id::text = current_setting('app.current_agency_id', true)))
    WITH CHECK (EXISTS (SELECT 1 FROM digital_employees e WHERE e.id = employee_id AND e.agency_id::text = current_setting('app.current_agency_id', true)));

CREATE TABLE IF NOT EXISTS employee_missions (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    employee_id     UUID NOT NULL REFERENCES digital_employees (id),
    version         INTEGER NOT NULL DEFAULT 1,
    mission         JSONB NOT NULL,      -- statement/domain/audience/value_proposition/constraints/success_definition
    active          BOOLEAN NOT NULL DEFAULT true,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS employee_goals (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    employee_id     UUID NOT NULL REFERENCES digital_employees (id),
    kind            TEXT NOT NULL,       -- growth/engagement/authority/reach/conversion/quality/consistency
    target          JSONB NOT NULL,      -- {metric, value, horizon}
    deadline        TIMESTAMPTZ,
    status          TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active','met','missed','retired')),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- strategy is versioned: DRAFT -> REVIEW -> ACTIVE -> SUPERSEDED (DE §7)
CREATE TABLE IF NOT EXISTS employee_strategies (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    employee_id     UUID NOT NULL REFERENCES digital_employees (id),
    version         INTEGER NOT NULL,
    strategy        JSONB NOT NULL,      -- pillars/cadence/formats/channel/risk/exploration_budget
    status          TEXT NOT NULL DEFAULT 'DRAFT' CHECK (status IN ('DRAFT','REVIEW','ACTIVE','SUPERSEDED')),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (employee_id, version)
);

-- operating cycle + task graph (DE §5.2, §9)
CREATE TABLE IF NOT EXISTS operating_cycles (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    employee_id     UUID NOT NULL REFERENCES digital_employees (id),
    state           TEXT NOT NULL DEFAULT 'PENDING'
                    CHECK (state IN ('PENDING','RUNNING','SUCCEEDED','FAILED','CANCELLED')),
    trigger         TEXT NOT NULL DEFAULT 'manual',  -- scheduled/event/manual
    idempotency_key TEXT NOT NULL,
    summary         JSONB,
    started_at      TIMESTAMPTZ,
    finished_at     TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (employee_id, idempotency_key)
);

CREATE TABLE IF NOT EXISTS cycle_tasks (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    cycle_id        UUID NOT NULL REFERENCES operating_cycles (id),
    agent           TEXT NOT NULL,       -- executive/research/strategy/creative/content/critic_compliance/analytics_learning
    step            TEXT NOT NULL,       -- observe/understand/decide/plan/create/review/execute/measure/learn/adapt
    state           TEXT NOT NULL DEFAULT 'PENDING'
                    CHECK (state IN ('PENDING','READY','RUNNING','SUCCEEDED','RETRYING','WAITING_APPROVAL','WAITING_EXTERNAL','FAILED','CANCELLED','BLOCKED')),
    input           JSONB,
    output          JSONB,
    idempotency_key TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (cycle_id, idempotency_key)
);

-- structured, evidence-linked memory (DE §13)
CREATE TABLE IF NOT EXISTS memory_items (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    employee_id     UUID NOT NULL REFERENCES digital_employees (id),
    type            TEXT NOT NULL CHECK (type IN ('episodic','semantic','strategic','audience','creative','performance','experiment','operational')),
    content         TEXT NOT NULL,
    evidence        JSONB NOT NULL DEFAULT '[]'::jsonb,
    confidence      NUMERIC NOT NULL DEFAULT 0.5 CHECK (confidence BETWEEN 0 AND 1),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_validated  TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS employee_insights (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    employee_id     UUID NOT NULL REFERENCES digital_employees (id),
    cycle_id        UUID REFERENCES operating_cycles (id),
    content         TEXT NOT NULL,
    evidence        JSONB NOT NULL DEFAULT '[]'::jsonb,
    confidence      NUMERIC NOT NULL DEFAULT 0.5,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS employee_experiments (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    employee_id     UUID NOT NULL REFERENCES digital_employees (id),
    hypothesis      TEXT NOT NULL,
    variable        TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'PROPOSED'
                    CHECK (status IN ('PROPOSED','APPROVED','RUNNING','ANALYZING','VALIDATED','REJECTED','INCONCLUSIVE')),
    result          JSONB,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ============ World Intelligence Station (WI guide) ============
-- Platform-level registries (no tenant RLS by design, like app_adapters).

CREATE TABLE IF NOT EXISTS intelligence_sources (
    id              TEXT PRIMARY KEY,    -- e.g. rsshub, searxng, trendradar
    name            TEXT NOT NULL,
    category        TEXT NOT NULL,       -- social/search/news/rss/rank/trend/provider
    access_method   TEXT NOT NULL CHECK (access_method IN ('NativeConnector','RESTProvider','MCPServer','CLIWorker','ExternalService','RSSSource','Aggregator')),
    capabilities    JSONB NOT NULL DEFAULT '[]'::jsonb,  -- social_search/trend_search/news_research/...
    platforms       JSONB NOT NULL DEFAULT '[]'::jsonb,
    auth_ref        TEXT,                -- vault:// reference only
    terms           TEXT,
    enabled         BOOLEAN NOT NULL DEFAULT true,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS intel_providers (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    source_id       TEXT NOT NULL REFERENCES intelligence_sources (id),
    name            TEXT NOT NULL,
    endpoint        TEXT,
    cost_model      JSONB NOT NULL DEFAULT '{}'::jsonb,
    rate_limit      JSONB NOT NULL DEFAULT '{}'::jsonb,
    health          TEXT NOT NULL DEFAULT 'unknown' CHECK (health IN ('healthy','degraded','down','unknown')),
    reliability     NUMERIC NOT NULL DEFAULT 0.9,
    enabled         BOOLEAN NOT NULL DEFAULT true,
    last_health_at  TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_intel_providers_source ON intel_providers (source_id);

CREATE TABLE IF NOT EXISTS intel_collection_jobs (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    source_id       TEXT NOT NULL REFERENCES intelligence_sources (id),
    schedule        TEXT NOT NULL DEFAULT 'hourly',
    status          TEXT NOT NULL DEFAULT 'idle' CHECK (status IN ('idle','running','paused','failed')),
    last_run_at     TIMESTAMPTZ,
    last_error      TEXT
);

-- normalized intelligence items (WI §11 unified data model), dedup by key
CREATE TABLE IF NOT EXISTS intel_items (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    source_id       TEXT NOT NULL REFERENCES intelligence_sources (id),
    provider_id     UUID REFERENCES intel_providers (id),
    kind            TEXT NOT NULL CHECK (kind IN ('social_content','trend','event','topic','creator','news','knowledge')),
    dedup_key       TEXT NOT NULL UNIQUE,
    payload         JSONB NOT NULL,      -- normalized SocialContent/Trend/Event/... schema
    collected_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_intel_items_kind ON intel_items (kind, collected_at DESC);

-- opportunity engine output (WI §19), consumed by employee adapter (§20)
CREATE TABLE IF NOT EXISTS intel_opportunities (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    topic           TEXT NOT NULL,
    score           NUMERIC NOT NULL DEFAULT 0,
    rationale       JSONB NOT NULL DEFAULT '{}'::jsonb,
    evidence        JSONB NOT NULL DEFAULT '[]'::jsonb,  -- refs to intel_items
    status          TEXT NOT NULL DEFAULT 'new' CHECK (status IN ('new','selected','dismissed','published')),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT intel_opportunities_topic_key UNIQUE (topic)
);
CREATE INDEX IF NOT EXISTS idx_intel_opps_score ON intel_opportunities (status, score DESC);

COMMIT;
