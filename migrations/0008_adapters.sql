-- 0008_adapters: App Adapter registry, versions, test runs, UI drift events
-- (FR-022, TDD 9.3 / 11.1, WP-13). PLATFORM-LEVEL tables: no agency_id, no
-- tenant RLS; writes restricted to Platform Admin (app layer).

BEGIN;

CREATE TABLE IF NOT EXISTS app_adapters (
    id          TEXT PRIMARY KEY,           -- e.g. xiaohongshu.publish_video
    app_package TEXT NOT NULL,
    description TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS app_adapter_versions (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    adapter_id      TEXT NOT NULL REFERENCES app_adapters (id),
    version         INTEGER NOT NULL,
    version_range   TEXT NOT NULL,          -- target App version range, e.g. ">=8.10 <8.20"
    definition      JSONB NOT NULL,         -- the declarative adapter (validated against schema)
    signature       TEXT NOT NULL,          -- platform signing key signature over definition
    sha256          TEXT NOT NULL,
    state           TEXT NOT NULL DEFAULT 'active'
                    CHECK (state IN ('active', 'degraded', 'withdrawn')),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (adapter_id, version)
);

-- device-reported drift events (WP-13)
CREATE TABLE IF NOT EXISTS ui_drift_events (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    adapter_id      TEXT NOT NULL,
    adapter_version INTEGER NOT NULL,
    app_package     TEXT NOT NULL,
    app_version     TEXT NOT NULL,
    device_id       UUID,
    reason          TEXT NOT NULL,          -- version_out_of_range | anchor_match_low | step_timeout
    detail          JSONB NOT NULL DEFAULT '{}'::jsonb,
    at              TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_ui_drift_adapter ON ui_drift_events (adapter_id, adapter_version);

--真机回归测试记录
CREATE TABLE IF NOT EXISTS adapter_test_runs (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    adapter_id      TEXT NOT NULL,
    adapter_version INTEGER NOT NULL,
    device_model    TEXT NOT NULL,
    app_version     TEXT NOT NULL,
    result          TEXT NOT NULL CHECK (result IN ('pass', 'fail', 'blocked')),
    steps_log       JSONB NOT NULL DEFAULT '[]'::jsonb,
    ran_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

COMMIT;
