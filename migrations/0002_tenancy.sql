-- 0002_tenancy: multi-tenant base schema with RLS (FR-001, TDD 11.2, ADR-008, WP-03).
--
-- Rules implemented here (Guide 6):
--  * every tenant table carries agency_id; the tenant root (agencies) carries
--    agency_id = id so composite FK (agency_id, id) works uniformly;
--  * parent tables get UNIQUE(agency_id, id) before child composite FKs;
--  * PostgreSQL RLS is ENFORCED (FORCE) on all tenant tables; the policy reads
--    app.current_agency_id which is set per transaction via SET LOCAL;
--  * platform-level tables (platform_apps, quota_pools, channel_capabilities,
--    app_adapters) are added by later migrations WITHOUT tenant RLS and with
--    writes restricted to the Platform Admin role.

BEGIN;

-- ---------------------------------------------------------------------------
-- Tenant root
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS agencies (
    id          UUID PRIMARY KEY,
    agency_id   UUID NOT NULL,               -- self-referencing tenant key
    name        TEXT NOT NULL,
    plan        TEXT NOT NULL DEFAULT 'free',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT agencies_tenant_key UNIQUE (agency_id, id)
);

-- root row: agency_id mirrors id
CREATE OR REPLACE FUNCTION sincy_agencies_agency_id() RETURNS TRIGGER AS $$
BEGIN
    NEW.agency_id := NEW.id;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_agencies_agency_id ON agencies;
CREATE TRIGGER trg_agencies_agency_id
    BEFORE INSERT ON agencies
    FOR EACH ROW EXECUTE FUNCTION sincy_agencies_agency_id();

-- ---------------------------------------------------------------------------
-- Tenant tables
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS agency_members (
    id          UUID PRIMARY KEY,
    agency_id   UUID NOT NULL,
    email       TEXT NOT NULL,
    role        TEXT NOT NULL DEFAULT 'operator',   -- owner|operator|approver|device_owner
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT agency_members_tenant_key UNIQUE (agency_id, id),
    -- root-level link: single-column FK + RLS; agency_id is never client-supplied
    CONSTRAINT fk_members_agency FOREIGN KEY (agency_id)
        REFERENCES agencies (id)
);

CREATE TABLE IF NOT EXISTS clients (
    id          UUID PRIMARY KEY,
    agency_id   UUID NOT NULL,
    name        TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'active',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT clients_tenant_key UNIQUE (agency_id, id),
    CONSTRAINT fk_clients_agency FOREIGN KEY (agency_id)
        REFERENCES agencies (id)
);

CREATE TABLE IF NOT EXISTS brands (
    id          UUID PRIMARY KEY,
    agency_id   UUID NOT NULL,
    client_id   UUID NOT NULL,
    name        TEXT NOT NULL,
    -- brand voice, audience, forbidden topics, visual rules, compliance (FR-002)
    config      JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT brands_tenant_key UNIQUE (agency_id, id),
    CONSTRAINT fk_brands_agency FOREIGN KEY (agency_id)
        REFERENCES agencies (id),
    -- in-tenant hierarchy uses the composite key (agency_id, parent_id) -> (agency_id, id)
    CONSTRAINT fk_brands_client FOREIGN KEY (agency_id, client_id)
        REFERENCES clients (agency_id, id)
);

CREATE TABLE IF NOT EXISTS roles (
    id          UUID PRIMARY KEY,
    agency_id   UUID NOT NULL,
    name        TEXT NOT NULL,
    permissions JSONB NOT NULL DEFAULT '[]'::jsonb,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT roles_tenant_key UNIQUE (agency_id, id),
    CONSTRAINT fk_roles_agency FOREIGN KEY (agency_id)
        REFERENCES agencies (id)
);

-- ---------------------------------------------------------------------------
-- Row Level Security (ADR-008): tenant context comes from
-- current_setting('app.current_agency_id', true) — set per transaction by
-- sincy-database (SET LOCAL), never from client input.
-- ---------------------------------------------------------------------------
DO $$
DECLARE t TEXT;
BEGIN
    FOREACH t IN ARRAY ARRAY['agencies', 'agency_members', 'clients', 'brands', 'roles']
    LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', t);
        EXECUTE format(
            'DROP POLICY IF EXISTS tenant_isolation ON %I', t);
        EXECUTE format(
            'CREATE POLICY tenant_isolation ON %I
               USING (agency_id::text = current_setting(''app.current_agency_id'', true))
               WITH CHECK (agency_id::text = current_setting(''app.current_agency_id'', true))',
            t);
    END LOOP;
END $$;

-- ---------------------------------------------------------------------------
-- Application role: no bypass, no superuser. Workers connect as this role
-- after SET LOCAL app.current_agency_id; without the setting, RLS yields
-- zero rows (fail closed).
-- ---------------------------------------------------------------------------
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sincy_app') THEN
        CREATE ROLE sincy_app LOGIN PASSWORD 'sincy_app_dev_only';
    END IF;
END $$;

GRANT USAGE ON SCHEMA public TO sincy_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO sincy_app;
ALTER DEFAULT PRIVILEGES IN SCHEMA public
    GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO sincy_app;

COMMIT;
