-- 0001_bootstrap: initial schema placeholder.
-- WP-03 replaces this with the real tenancy schema (agencies, clients, brands,
-- RLS policies, composite UNIQUE(id, agency_id) keys). Kept runnable on an
-- empty database so `cargo run -p cloud-api` works from WP-01 onward.

CREATE TABLE IF NOT EXISTS schema_bootstrap (
    id          BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    applied_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    note        TEXT NOT NULL DEFAULT 'sincy schema bootstrap'
);

INSERT INTO schema_bootstrap DEFAULT VALUES;
