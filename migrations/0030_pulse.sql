-- 0030_pulse: Sincy Pulse — Extensible Intelligence Provider Plugin Framework 数据域
-- 与 intel_items 分开 (设计 §九): 原始 observation 保留, 统一实体另存。
BEGIN;

-- Provider 注册表 (manifest 驱动, 含法务状态 — 红线: 未标配合规的 provider 默认不启用)
CREATE TABLE IF NOT EXISTS pulse_providers (
  id             TEXT PRIMARY KEY,
  name           TEXT NOT NULL,
  platform       TEXT NOT NULL DEFAULT '',
  category       TEXT NOT NULL DEFAULT 'social_intelligence',
  manifest       JSONB NOT NULL,
  legal_status   TEXT NOT NULL DEFAULT 'unknown'
                 CHECK (legal_status IN ('public','commercial_ok','no_redistribute','attribution_required','unknown','blocked')),
  reliability    NUMERIC NOT NULL DEFAULT 0.5 CHECK (reliability BETWEEN 0 AND 1),
  enabled        BOOLEAN NOT NULL DEFAULT false,
  created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- 采集运行记录 (健康审计用)
CREATE TABLE IF NOT EXISTS pulse_provider_runs (
  id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  provider_id   TEXT NOT NULL REFERENCES pulse_providers(id),
  dataset_id    TEXT NOT NULL DEFAULT '',
  started_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  finished_at   TIMESTAMPTZ,
  status        TEXT NOT NULL DEFAULT 'running' CHECK (status IN ('running','ok','failed')),
  items_fetched INTEGER NOT NULL DEFAULT 0,
  items_new     INTEGER NOT NULL DEFAULT 0,
  error         TEXT
);
CREATE INDEX IF NOT EXISTS idx_pulse_runs_provider ON pulse_provider_runs (provider_id, started_at DESC);

-- 统一 Intelligence Entity (跨源聚合后的产品层数据)
CREATE TABLE IF NOT EXISTS pulse_entities (
  id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  entity_type    TEXT NOT NULL,                -- social_post / social_account / ranking / trend / topic ...
  platform       TEXT NOT NULL DEFAULT '',
  canonical_key  TEXT NOT NULL,                -- 跨 provider 消解键 (如 x:post:123456)
  title          TEXT NOT NULL DEFAULT '',
  payload        JSONB NOT NULL DEFAULT '{}',  -- 原始字段(作者/内容/链接等)
  metrics        JSONB NOT NULL DEFAULT '{}',  -- 聚合指标: {views:{estimated,range}, cross_source_score, provider_count}
  trend          JSONB NOT NULL DEFAULT '{}',  -- {velocity, acceleration, classification}
  confidence     JSONB NOT NULL DEFAULT '{}',  -- {score, level}
  created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (entity_type, canonical_key)
);
CREATE INDEX IF NOT EXISTS idx_pulse_entities_type ON pulse_entities (entity_type, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_pulse_entities_trend ON pulse_entities ((trend->>'classification'));

-- 第三方原始 Observation (可审计/可回溯 — 不丢弃 raw provider data)
CREATE TABLE IF NOT EXISTS pulse_observations (
  id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  entity_id   UUID NOT NULL REFERENCES pulse_entities(id) ON DELETE CASCADE,
  provider_id TEXT NOT NULL REFERENCES pulse_providers(id),
  dataset_id  TEXT NOT NULL DEFAULT '',
  metric      TEXT NOT NULL,                   -- views / rank / followers / velocity ...
  value       JSONB NOT NULL,                  -- 原始值 (数字或字符串)
  observed_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_pulse_obs_entity ON pulse_observations (entity_id, metric);
CREATE INDEX IF NOT EXISTS idx_pulse_obs_provider ON pulse_observations (provider_id, observed_at DESC);

COMMIT;
