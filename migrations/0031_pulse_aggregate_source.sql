-- 0031_pulse_aggregate_source: Sincy Pulse 定位为世界情报 (/intel) 的一个聚合源。
--
-- 1) 下线 3 个内置 feed 包装型 provider (pulse-hotrank / pulse-hn / pulse-gnews):
--    它们只是 intel_collector 里 hotrank-hub / hn-algolia / google-news 信源的重复,
--    这些数据本就属于世界情报的普通信源, 不再占用 Pulse 的 provider 位。
-- 2) 在 intelligence_sources 注册聚合源 sincy-pulse:
--    category='aggregate' 标识它是"聚合情报源"而非普通采集信源,
--    其条目由 pulse_entities 镜像写入 intel_items (source_id='sincy-pulse')。
-- 3) 7 个 X 生态站 (xbangdan / xpaiming / shadowban-radar / getxanalytics / xranks /
--    trends24 / tide-now) 由应用启动时 seed_providers() 写入 pulse_providers,
--    legal_status='unknown' (待法务审查), enabled=false —— 调度器红线不会跑它们。

-- 1) 清理内置 provider 及其产物 (先子表后父表)
DELETE FROM pulse_observations WHERE provider_id IN ('pulse-hotrank', 'pulse-hn', 'pulse-gnews');
DELETE FROM pulse_entities e WHERE NOT EXISTS (SELECT 1 FROM pulse_observations o WHERE o.entity_id = e.id);
DELETE FROM pulse_provider_runs WHERE provider_id IN ('pulse-hotrank', 'pulse-hn', 'pulse-gnews');
DELETE FROM pulse_providers WHERE id IN ('pulse-hotrank', 'pulse-hn', 'pulse-gnews');

-- 2) 聚合源注册到世界情报信源表 (幂等)
INSERT INTO intelligence_sources (id, name, category, access_method, capabilities, platforms, terms, enabled)
VALUES (
    'sincy-pulse',
    'Sincy Pulse (聚合情报源)',
    'aggregate',
    'Aggregator',
    '["trend_intelligence","viral_content","creator_intelligence","opportunity_detection","cross_source_verification"]'::jsonb,
    '["x","web"]'::jsonb,
    'Sincy 聚合情报产品: 由可扩展的第三方情报 Provider 经 Adapter 采集、统一为实体并交叉验证后输出, 用户只看到 Sincy 品牌。底层原始观测保留在 pulse_observations。',
    true
)
ON CONFLICT (id) DO UPDATE SET
    name = EXCLUDED.name,
    category = EXCLUDED.category,
    terms = EXCLUDED.terms;
