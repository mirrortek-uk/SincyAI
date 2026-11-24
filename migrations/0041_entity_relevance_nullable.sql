-- 0041: 修正两个通用处理器弱点
--
-- 1) intelligence_entities.relevance_score 之前在入库路径恒写 0 (误导性死字段):
--    情报是全局共享入库, 入库时没有"单一客户"可绑, 相关性属于 per-brand 下游计算。
--    因此该列改为可空且入库不再写入; per-brand 相关性落到新表
--    intel_entity_brand_relevance, 由 GET /brands/{id}/intel-feed 服务时按品牌回填。
-- 2) topic_tags 改为真话题抽取 (sincy-intel::topic 词表 + 关键词回退), 不再截取正文前 60 字。

BEGIN;

ALTER TABLE intelligence_entities ALTER COLUMN relevance_score DROP NOT NULL;
ALTER TABLE intelligence_entities ALTER COLUMN relevance_score DROP DEFAULT;

CREATE TABLE IF NOT EXISTS intel_entity_brand_relevance (
    dedup_key   TEXT NOT NULL,               -- 跨 intel_items / intelligence_entities 的稳定键
    brand_id    UUID NOT NULL REFERENCES brands(id) ON DELETE CASCADE,
    agency_id   UUID NOT NULL REFERENCES agencies(id),
    score       DOUBLE PRECISION NOT NULL,   -- 0..100, relevance_score() 的 per-brand 结果
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (dedup_key, brand_id)
);
CREATE INDEX IF NOT EXISTS idx_entity_brand_rel_brand ON intel_entity_brand_relevance (brand_id, score DESC);

ALTER TABLE intel_entity_brand_relevance ENABLE ROW LEVEL SECURITY;
ALTER TABLE intel_entity_brand_relevance FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON intel_entity_brand_relevance
    USING (agency_id::text = current_setting('app.current_agency_id', true))
    WITH CHECK (agency_id::text = current_setting('app.current_agency_id', true));

COMMIT;
