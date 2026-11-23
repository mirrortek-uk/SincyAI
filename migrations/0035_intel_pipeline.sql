-- 0035_intel_pipeline: Sincy Intelligence Ingestion & Processing Pipeline 存储 (四层, Sincy-05)
-- Layer 1 原始记录 / Layer 2 类型化实体 + 通用元数据 / 专用时序库 / Layer 4 信号。
-- 现有 intel_items 保持不变 (legacy article 层), 新管线与之并行。

-- Layer 1: Raw Records (所有 connector 的原始输出, 永久保留, 可重放)
CREATE TABLE IF NOT EXISTS intelligence_raw_records (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    provider_id     TEXT NOT NULL,
    source_id       TEXT NOT NULL,
    dataset_id      TEXT NOT NULL,
    record_type     TEXT NOT NULL,
    external_id     TEXT,
    url             TEXT,
    dedup_key       TEXT NOT NULL UNIQUE,
    payload         JSONB NOT NULL,
    metadata        JSONB NOT NULL DEFAULT '{}'::jsonb,
    fetched_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    observed_at     TIMESTAMPTZ,
    processing_status TEXT NOT NULL DEFAULT 'pending' CHECK (processing_status IN ('pending','processed','failed'))
);
CREATE INDEX IF NOT EXISTS idx_raw_records_type ON intelligence_raw_records (record_type, fetched_at DESC);
CREATE INDEX IF NOT EXISTS idx_raw_records_source ON intelligence_raw_records (source_id, dataset_id);

-- Layer 2: 统一实体 (类型化数据 + universal metadata 一体, 通过 entity_type 区分形状)
CREATE TABLE IF NOT EXISTS intelligence_entities (
    entity_id       TEXT PRIMARY KEY,
    entity_type     TEXT NOT NULL,          -- article / trend / ranking / social_post / social_account ...
    source_id       TEXT NOT NULL,
    dataset_id      TEXT NOT NULL,
    external_id     TEXT,
    url             TEXT,
    title           TEXT NOT NULL DEFAULT '',
    payload         JSONB NOT NULL DEFAULT '{}'::jsonb,
    industry_tags   JSONB NOT NULL DEFAULT '[]'::jsonb,
    topic_tags      JSONB NOT NULL DEFAULT '[]'::jsonb,
    entities        JSONB NOT NULL DEFAULT '[]'::jsonb,
    geography       JSONB NOT NULL DEFAULT '[]'::jsonb,
    language        TEXT NOT NULL DEFAULT '',
    provenance      JSONB NOT NULL DEFAULT '{}'::jsonb,
    confidence      DOUBLE PRECISION NOT NULL DEFAULT 0.5,
    quality_score   DOUBLE PRECISION NOT NULL DEFAULT 0.5,
    relevance_score DOUBLE PRECISION NOT NULL DEFAULT 0,
    observed_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_intel_entities_type ON intelligence_entities (entity_type, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_intel_entities_industry ON intelligence_entities USING gin (industry_tags);

-- 专用时序库: 趋势序列 / 榜单快照 (RankingBoard, TrendSignal 不再塞进一张表)
CREATE TABLE IF NOT EXISTS trend_series (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    series_key  TEXT NOT NULL,              -- dataset:topic:geo
    value       DOUBLE PRECISION NOT NULL,
    source_id   TEXT NOT NULL,
    observed_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_trend_series_key ON trend_series (series_key, observed_at DESC);

CREATE TABLE IF NOT EXISTS ranking_snapshots (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    dataset_id  TEXT NOT NULL,
    external_id TEXT,
    title       TEXT NOT NULL DEFAULT '',
    rank        DOUBLE PRECISION,
    metrics     JSONB NOT NULL DEFAULT '{}'::jsonb,
    observed_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_ranking_snapshots_ds ON ranking_snapshots (dataset_id, observed_at DESC);

-- Layer 4: Intelligence Signals (Opportunity Engine 只消费这个)
CREATE TABLE IF NOT EXISTS intelligence_signals (
    signal_id    TEXT PRIMARY KEY,
    signal_type  TEXT NOT NULL,
    entity_id    TEXT,
    source_id    TEXT NOT NULL,
    industry_id  TEXT,
    topic_id     TEXT,
    strength     DOUBLE PRECISION NOT NULL DEFAULT 0,
    confidence   DOUBLE PRECISION NOT NULL DEFAULT 0,
    importance   DOUBLE PRECISION NOT NULL DEFAULT 0,
    observed_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    payload      JSONB NOT NULL DEFAULT '{}'::jsonb
);
CREATE INDEX IF NOT EXISTS idx_signals_type ON intelligence_signals (signal_type, observed_at DESC);
CREATE INDEX IF NOT EXISTS idx_signals_importance ON intelligence_signals (importance DESC);

-- 处理游标 (连接器幂等轮询)
CREATE TABLE IF NOT EXISTS connector_cursors (
    connector_id TEXT NOT NULL,
    dataset_id   TEXT NOT NULL,
    cursor_value TEXT NOT NULL DEFAULT '',
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (connector_id, dataset_id)
);
