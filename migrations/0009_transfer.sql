-- 0009_transfer: transfer sessions + chunk metadata (FR-012, TDD 11.1, WP-10).
-- Chunk BYTES go to object storage / LAN; only metadata + hashes live here.
-- Tenant-scoped with RLS; cross-tenant download must be rejected (WP-10 验收).

BEGIN;

CREATE TABLE IF NOT EXISTS transfer_sessions (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agency_id       UUID NOT NULL REFERENCES agencies (id),
    task_id         UUID,
    direction       TEXT NOT NULL CHECK (direction IN ('upload', 'download')),
    object_key      TEXT NOT NULL,
    total_bytes     BIGINT NOT NULL CHECK (total_bytes >= 0),
    chunk_size      INTEGER NOT NULL CHECK (chunk_size > 0),
    sha256          TEXT NOT NULL,             -- whole-file hash
    state           TEXT NOT NULL DEFAULT 'active'
                    CHECK (state IN ('active', 'completed', 'failed', 'expired')),
    expires_at      TIMESTAMPTZ NOT NULL DEFAULT (now() + interval '24 hours'),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT transfer_sessions_tenant_key UNIQUE (agency_id, id)
);
ALTER TABLE transfer_sessions ENABLE ROW LEVEL SECURITY;
ALTER TABLE transfer_sessions FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON transfer_sessions;
CREATE POLICY tenant_isolation ON transfer_sessions
    USING (agency_id::text = current_setting('app.current_agency_id', true))
    WITH CHECK (agency_id::text = current_setting('app.current_agency_id', true));

CREATE TABLE IF NOT EXISTS transfer_chunks (
    session_id      UUID NOT NULL,
    agency_id       UUID NOT NULL,
    chunk_index     INTEGER NOT NULL,
    bytes           BIGINT NOT NULL,
    sha256          TEXT NOT NULL,
    state           TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending', 'done', 'failed')),
    PRIMARY KEY (agency_id, session_id, chunk_index),
    CONSTRAINT transfer_chunks_tenant_key UNIQUE (agency_id, session_id, chunk_index),
    FOREIGN KEY (agency_id, session_id) REFERENCES transfer_sessions (agency_id, id)
);
ALTER TABLE transfer_chunks ENABLE ROW LEVEL SECURITY;
ALTER TABLE transfer_chunks FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON transfer_chunks;
CREATE POLICY tenant_isolation ON transfer_chunks
    USING (agency_id::text = current_setting('app.current_agency_id', true))
    WITH CHECK (agency_id::text = current_setting('app.current_agency_id', true));

COMMIT;
