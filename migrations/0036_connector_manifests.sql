-- 0036: Connector 清单 (架构图点击 Connector 展开查看; 与代码 default_connector_manifests 幂等同步)
CREATE TABLE IF NOT EXISTS connector_manifests (
    id              TEXT PRIMARY KEY,           -- connector id, e.g. 'arxiv-atom'
    name            TEXT NOT NULL,
    connector_type  TEXT NOT NULL,              -- rss / api / website / search / aggregator / custom
    capabilities    JSONB NOT NULL DEFAULT '[]',
    datasets        JSONB NOT NULL DEFAULT '[]',-- [{id, record_type, url}]
    sources         JSONB NOT NULL DEFAULT '[]',-- [{source_id, label, url, kind, record_type, enabled}]
    fetch_impl      TEXT NOT NULL DEFAULT '',   -- 现役采集实现 collector:<Parser> / pulse:<provider>
    default_interval_secs BIGINT NOT NULL DEFAULT 1800,
    enabled         BOOLEAN NOT NULL DEFAULT TRUE,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_connector_manifests_type ON connector_manifests (connector_type);
