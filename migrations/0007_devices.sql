-- 0007_devices: device registry, consents, sessions (FR-003/010/023, TDD 11.1, WP-07).
-- Tenant-scoped with RLS. Device identity is independent per device (TDD 15):
-- registration uses challenge-response (sincy-security::challenge).

BEGIN;

CREATE TABLE IF NOT EXISTS devices (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agency_id       UUID NOT NULL REFERENCES agencies (id),
    name            TEXT,
    platform        TEXT NOT NULL CHECK (platform IN ('android', 'windows')),
    os_version      TEXT,
    agent_version   TEXT,
    -- per-device secret HASH (never plaintext); challenge-response proof
    secret_hash     TEXT NOT NULL,
    state           TEXT NOT NULL DEFAULT 'pending'
                    CHECK (state IN ('pending', 'active', 'revoked')),
    last_seen       TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT devices_tenant_key UNIQUE (agency_id, id)
);
ALTER TABLE devices ENABLE ROW LEVEL SECURITY;
ALTER TABLE devices FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON devices;
CREATE POLICY tenant_isolation ON devices
    USING (agency_id::text = current_setting('app.current_agency_id', true))
    WITH CHECK (agency_id::text = current_setting('app.current_agency_id', true));

CREATE TABLE IF NOT EXISTS device_capabilities (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agency_id   UUID NOT NULL REFERENCES agencies (id),
    device_id   UUID NOT NULL,
    name        TEXT NOT NULL,             -- accessibility, media_projection, notifications, background_run
    available   BOOLEAN NOT NULL,
    detail      TEXT,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT device_capabilities_tenant_key UNIQUE (agency_id, id),
    FOREIGN KEY (agency_id, device_id) REFERENCES devices (agency_id, id)
);
ALTER TABLE device_capabilities ENABLE ROW LEVEL SECURITY;
ALTER TABLE device_capabilities FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON device_capabilities;
CREATE POLICY tenant_isolation ON device_capabilities
    USING (agency_id::text = current_setting('app.current_agency_id', true))
    WITH CHECK (agency_id::text = current_setting('app.current_agency_id', true));

CREATE TABLE IF NOT EXISTS device_app_versions (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agency_id   UUID NOT NULL REFERENCES agencies (id),
    device_id   UUID NOT NULL,
    package     TEXT NOT NULL,
    version_name TEXT NOT NULL,
    version_code INTEGER NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT device_app_versions_tenant_key UNIQUE (agency_id, id),
    UNIQUE (agency_id, device_id, package),
    FOREIGN KEY (agency_id, device_id) REFERENCES devices (agency_id, id)
);
ALTER TABLE device_app_versions ENABLE ROW LEVEL SECURITY;
ALTER TABLE device_app_versions FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON device_app_versions;
CREATE POLICY tenant_isolation ON device_app_versions
    USING (agency_id::text = current_setting('app.current_agency_id', true))
    WITH CHECK (agency_id::text = current_setting('app.current_agency_id', true));

-- sessions for the WSS gateway (WP-08); revoked sessions reject connections
CREATE TABLE IF NOT EXISTS device_sessions (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agency_id   UUID NOT NULL REFERENCES agencies (id),
    device_id   UUID NOT NULL,
    token_hash  TEXT NOT NULL,
    state       TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('pending', 'active', 'revoked')),
    connected_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen   TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT device_sessions_tenant_key UNIQUE (agency_id, id),
    FOREIGN KEY (agency_id, device_id) REFERENCES devices (agency_id, id)
);
ALTER TABLE device_sessions ENABLE ROW LEVEL SECURITY;
ALTER TABLE device_sessions FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON device_sessions;
CREATE POLICY tenant_isolation ON device_sessions
    USING (agency_id::text = current_setting('app.current_agency_id', true))
    WITH CHECK (agency_id::text = current_setting('app.current_agency_id', true));

-- automation consent (FR-023): per account/platform/action, revocable
CREATE TABLE IF NOT EXISTS automation_consents (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agency_id   UUID NOT NULL REFERENCES agencies (id),
    account_id  UUID NOT NULL,
    platform    TEXT NOT NULL,
    action      TEXT NOT NULL,
    granted     BOOLEAN NOT NULL DEFAULT true,
    risk_ack    TEXT NOT NULL,            -- 风险告知文本版本
    granted_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    revoked_at  TIMESTAMPTZ,
    CONSTRAINT automation_consents_tenant_key UNIQUE (agency_id, id),
    UNIQUE (agency_id, account_id, platform, action)
);
ALTER TABLE automation_consents ENABLE ROW LEVEL SECURITY;
ALTER TABLE automation_consents FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON automation_consents;
CREATE POLICY tenant_isolation ON automation_consents
    USING (agency_id::text = current_setting('app.current_agency_id', true))
    WITH CHECK (agency_id::text = current_setting('app.current_agency_id', true));

-- social accounts (FR-003): credentials are Vault refs only
CREATE TABLE IF NOT EXISTS social_accounts (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agency_id   UUID NOT NULL REFERENCES agencies (id),
    brand_id    UUID NOT NULL REFERENCES brands (id),
    platform    TEXT NOT NULL,
    handle      TEXT NOT NULL,
    oauth_ref   TEXT,                      -- vault://... when API route authorized
    oauth_scopes TEXT[] NOT NULL DEFAULT '{}',
    oauth_expires_at TIMESTAMPTZ,
    state       TEXT NOT NULL DEFAULT 'connected' CHECK (state IN ('connected', 'expired', 'revoked')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT social_accounts_tenant_key UNIQUE (agency_id, id)
);
ALTER TABLE social_accounts ENABLE ROW LEVEL SECURITY;
ALTER TABLE social_accounts FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON social_accounts;
CREATE POLICY tenant_isolation ON social_accounts
    USING (agency_id::text = current_setting('app.current_agency_id', true))
    WITH CHECK (agency_id::text = current_setting('app.current_agency_id', true));

-- device <-> social account binding (device route)
CREATE TABLE IF NOT EXISTS account_device_bindings (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agency_id   UUID NOT NULL REFERENCES agencies (id),
    account_id  UUID NOT NULL,
    device_id   UUID NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT account_device_bindings_tenant_key UNIQUE (agency_id, id),
    UNIQUE (agency_id, account_id, device_id),
    FOREIGN KEY (agency_id, account_id) REFERENCES social_accounts (agency_id, id),
    FOREIGN KEY (agency_id, device_id) REFERENCES devices (agency_id, id)
);
ALTER TABLE account_device_bindings ENABLE ROW LEVEL SECURITY;
ALTER TABLE account_device_bindings FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON account_device_bindings;
CREATE POLICY tenant_isolation ON account_device_bindings
    USING (agency_id::text = current_setting('app.current_agency_id', true))
    WITH CHECK (agency_id::text = current_setting('app.current_agency_id', true));

COMMIT;
