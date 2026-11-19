-- 0005_content: content factory + publishing package + approvals
-- (FR-006/007/008/009/024, TDD 11.1/13, WP-04). All tenant-scoped with RLS.

BEGIN;

CREATE TABLE IF NOT EXISTS content_items (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agency_id   UUID NOT NULL REFERENCES agencies (id),
    brand_id    UUID NOT NULL REFERENCES brands (id),
    kind        TEXT NOT NULL CHECK (kind IN ('text', 'image', 'video', 'audio', 'doc')),
    status      TEXT NOT NULL DEFAULT 'draft'
                CHECK (status IN ('draft', 'quality_check', 'policy_check', 'approved', 'packaged', 'blocked')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT content_items_tenant_key UNIQUE (agency_id, id)
);
ALTER TABLE content_items ENABLE ROW LEVEL SECURITY;
ALTER TABLE content_items FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON content_items;
CREATE POLICY tenant_isolation ON content_items
    USING (agency_id::text = current_setting('app.current_agency_id', true))
    WITH CHECK (agency_id::text = current_setting('app.current_agency_id', true));

-- append-only version history; the CURRENT version is a projection
CREATE TABLE IF NOT EXISTS content_versions (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agency_id       UUID NOT NULL REFERENCES agencies (id),
    content_item_id UUID NOT NULL,
    version         INTEGER NOT NULL CHECK (version > 0),
    body            JSONB NOT NULL,           -- text payload / render spec
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT content_versions_tenant_key UNIQUE (agency_id, id),
    UNIQUE (agency_id, content_item_id, version),
    FOREIGN KEY (agency_id, content_item_id) REFERENCES content_items (agency_id, id)
);
ALTER TABLE content_versions ENABLE ROW LEVEL SECURITY;
ALTER TABLE content_versions FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON content_versions;
CREATE POLICY tenant_isolation ON content_versions
    USING (agency_id::text = current_setting('app.current_agency_id', true))
    WITH CHECK (agency_id::text = current_setting('app.current_agency_id', true));

CREATE TABLE IF NOT EXISTS assets (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agency_id   UUID NOT NULL REFERENCES agencies (id),
    sha256      TEXT NOT NULL,                -- content hash: drives duplicate detection
    mime        TEXT NOT NULL,
    bytes       BIGINT NOT NULL,
    storage_ref TEXT NOT NULL,                -- S3 key; bytes never in DB
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT assets_tenant_key UNIQUE (agency_id, id),
    UNIQUE (agency_id, sha256)                -- duplicate content detection (FR-007)
);
ALTER TABLE assets ENABLE ROW LEVEL SECURITY;
ALTER TABLE assets FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON assets;
CREATE POLICY tenant_isolation ON assets
    USING (agency_id::text = current_setting('app.current_agency_id', true))
    WITH CHECK (agency_id::text = current_setting('app.current_agency_id', true));

-- rights / provenance per asset (FR-007: 不默认移除第三方版权标识)
CREATE TABLE IF NOT EXISTS content_rights (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agency_id   UUID NOT NULL REFERENCES agencies (id),
    asset_id    UUID NOT NULL,
    source      TEXT,                          -- url | client_upload | generated
    license     TEXT,                          -- license id or 'proprietary'
    restrictions TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT content_rights_tenant_key UNIQUE (agency_id, id),
    FOREIGN KEY (agency_id, asset_id) REFERENCES assets (agency_id, id)
);
ALTER TABLE content_rights ENABLE ROW LEVEL SECURITY;
ALTER TABLE content_rights FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON content_rights;
CREATE POLICY tenant_isolation ON content_rights
    USING (agency_id::text = current_setting('app.current_agency_id', true))
    WITH CHECK (agency_id::text = current_setting('app.current_agency_id', true));

-- AI content label (FR-024: AIGC 标识 / EU AI Act 透明度 / C2PA 预留)
CREATE TABLE IF NOT EXISTS ai_labels (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agency_id       UUID NOT NULL REFERENCES agencies (id),
    content_item_id UUID NOT NULL,
    is_ai_generated BOOLEAN NOT NULL DEFAULT true,
    tools           JSONB NOT NULL DEFAULT '[]'::jsonb,   -- model/tool provenance
    c2pa_manifest_ref TEXT,                                 -- C2PA 凭证引用 (v1.1 预留)
    platform_requirements JSONB NOT NULL DEFAULT '{}'::jsonb, -- per-platform标签要求
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT ai_labels_tenant_key UNIQUE (agency_id, id),
    FOREIGN KEY (agency_id, content_item_id) REFERENCES content_items (agency_id, id)
);
ALTER TABLE ai_labels ENABLE ROW LEVEL SECURITY;
ALTER TABLE ai_labels FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON ai_labels;
CREATE POLICY tenant_isolation ON ai_labels
    USING (agency_id::text = current_setting('app.current_agency_id', true))
    WITH CHECK (agency_id::text = current_setting('app.current_agency_id', true));

-- immutable publishing package (FR-009, TDD 13): versions never mutated in place
CREATE TABLE IF NOT EXISTS publishing_packages (
    id              UUID NOT NULL,             -- package_id
    version         INTEGER NOT NULL CHECK (version > 0),
    agency_id       UUID NOT NULL REFERENCES agencies (id),
    brand_id        UUID NOT NULL REFERENCES brands (id),
    platform        TEXT NOT NULL,
    account_id      UUID NOT NULL,
    action          TEXT NOT NULL,
    scheduled_at    TIMESTAMPTZ,
    timezone        TEXT NOT NULL DEFAULT 'UTC',
    text_content    TEXT NOT NULL DEFAULT '',
    constraints     JSONB NOT NULL DEFAULT '{}'::jsonb,
    approval_status TEXT NOT NULL DEFAULT 'pending'
                    CHECK (approval_status IN ('pending', 'approved', 'rejected', 'auto_approved', 'withdrawn')),
    ai_label_id     UUID REFERENCES ai_labels (id),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (id, version),
    CONSTRAINT publishing_packages_tenant_key UNIQUE (agency_id, id, version)
);
ALTER TABLE publishing_packages ENABLE ROW LEVEL SECURITY;
ALTER TABLE publishing_packages FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON publishing_packages;
CREATE POLICY tenant_isolation ON publishing_packages
    USING (agency_id::text = current_setting('app.current_agency_id', true))
    WITH CHECK (agency_id::text = current_setting('app.current_agency_id', true));

CREATE TABLE IF NOT EXISTS package_assets (
    package_id      UUID NOT NULL,
    version         INTEGER NOT NULL,
    agency_id       UUID NOT NULL,
    asset_id        UUID NOT NULL,
    sha256          TEXT NOT NULL,             -- denormalized for integrity check
    PRIMARY KEY (agency_id, package_id, version, asset_id),
    FOREIGN KEY (agency_id, package_id, version) REFERENCES publishing_packages (agency_id, id, version),
    FOREIGN KEY (agency_id, asset_id) REFERENCES assets (agency_id, id)
);
ALTER TABLE package_assets ENABLE ROW LEVEL SECURITY;
ALTER TABLE package_assets FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON package_assets;
CREATE POLICY tenant_isolation ON package_assets
    USING (agency_id::text = current_setting('app.current_agency_id', true))
    WITH CHECK (agency_id::text = current_setting('app.current_agency_id', true));

-- approval records (FR-008): append-only decision log
CREATE TABLE IF NOT EXISTS approvals (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agency_id   UUID NOT NULL REFERENCES agencies (id),
    package_id  UUID NOT NULL,
    package_version INTEGER NOT NULL,
    approver    TEXT NOT NULL,
    decision    TEXT NOT NULL CHECK (decision IN ('approved', 'rejected', 'withdrawn')),
    comment     TEXT,
    decided_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT approvals_tenant_key UNIQUE (agency_id, id)
);
ALTER TABLE approvals ENABLE ROW LEVEL SECURITY;
ALTER TABLE approvals FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON approvals;
CREATE POLICY tenant_isolation ON approvals
    USING (agency_id::text = current_setting('app.current_agency_id', true))
    WITH CHECK (agency_id::text = current_setting('app.current_agency_id', true));

-- model cost records (FR-026)
CREATE TABLE IF NOT EXISTS model_cost_records (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agency_id   UUID NOT NULL REFERENCES agencies (id),
    brand_id    UUID,
    content_item_id UUID,
    provider    TEXT NOT NULL,
    model       TEXT NOT NULL,
    kind        TEXT NOT NULL,               -- llm | image | video | speech
    tokens_in   BIGINT NOT NULL DEFAULT 0,
    tokens_out  BIGINT NOT NULL DEFAULT 0,
    cost_usd    NUMERIC(12, 6) NOT NULL DEFAULT 0,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT model_cost_records_tenant_key UNIQUE (agency_id, id)
);
ALTER TABLE model_cost_records ENABLE ROW LEVEL SECURITY;
ALTER TABLE model_cost_records FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON model_cost_records;
CREATE POLICY tenant_isolation ON model_cost_records
    USING (agency_id::text = current_setting('app.current_agency_id', true))
    WITH CHECK (agency_id::text = current_setting('app.current_agency_id', true));

COMMIT;
