-- 0022_intel_expansion: World Intelligence source expansion (WI Phase 1 + Phase 3 注册)
--
-- Phase 1 (P0, 无需密钥、当前即可跑): Google News RSS / Product Hunt / B站排行榜
-- Phase 3 (中文热榜) + Reddit/GDELT/Google Trends/X 社媒: 注册为 source + paused job,
--   由 Source Health Audit 标记为 not_configured, 待自托管 RSSHub / 专用出口 IP / 付费
--   provider key (vault://) 配置后再启用。红线不变: 不绕认证/验证码/限流。

BEGIN;

-- jobs: 区分"抓到多少"与"真正新入库多少", 供健康审计判断 degraded
ALTER TABLE intel_collection_jobs ADD COLUMN IF NOT EXISTS last_fetched INTEGER;
ALTER TABLE intel_collection_jobs ADD COLUMN IF NOT EXISTS last_new INTEGER;

-- ============ 新信源注册 ============
INSERT INTO intelligence_sources (id, name, category, access_method, capabilities, platforms, auth_ref, terms, enabled) VALUES
  ('google-news',   'Google News RSS', 'news',    'RSSSource',     '["news_research","trend_search"]', '["google_news"]', NULL, 'public Google News RSS search, no key, polite UA', true),
  ('producthunt',   'Product Hunt',    'product', 'RSSSource',     '["product_discovery","trend_search"]', '["producthunt"]', NULL, 'public PH Atom feed, no key', true),
  ('bilibili',      'B站排行榜 API',   'rank',    'RESTProvider',  '["trend_search","social_search"]', '["bilibili"]', NULL, 'public ranking API v2, UA required, no login', true),
  ('gdelt',         'GDELT Project',   'event',   'RESTProvider',  '["event_stream","news_research"]', '[]', NULL, 'free DOC 2.0 API but rate-limits shared IPs (1 req/5s); enable on dedicated egress IP', false),
  ('google-trends', 'Google Trends',   'trend',   'ExternalService','["trend_search"]', '["google_trends"]', NULL, 'public RSS endpoint retired (404); needs self-hosted scraper/provider', false),
  ('reddit',        'Reddit',          'social',  'ExternalService','["social_search","reddit_research"]', '["reddit"]', NULL, 'direct .json is bot-blocked; use self-hosted RSSHub (SINCY_INTEL_RSSHUB_BASE)', true),
  ('zhihu-hot',     '知乎热榜',         'rank',    'RSSSource',     '["trend_search"]', '["zhihu"]', NULL, 'via self-hosted RSSHub /zhihu/hot', true),
  ('weibo-hot',     '微博热搜',         'rank',    'RSSSource',     '["trend_search"]', '["weibo"]', NULL, 'via self-hosted RSSHub; ajax API is cookie-gated', true),
  ('douyin-hot',    '抖音热点',         'rank',    'RSSSource',     '["trend_search"]', '["douyin"]', NULL, 'via self-hosted RSSHub /douyin/hot', true),
  ('baidu-hot',     '百度热搜',         'rank',    'RSSSource',     '["trend_search"]', '["baidu"]', NULL, 'via self-hosted RSSHub /baidu/hot', true),
  ('x-social',      'X/Twitter 情报',   'social',  'MCPServer',     '["x_research","social_search","trend_search"]', '["x"]', 'vault://x-provider-key', 'keyed provider (SocialDataX/TikHub/reach-mcp) or self-hosted mirror; no-key path is device automation', false),
  ('socialdatax',   'SocialDataX',     'social',  'RESTProvider',  '["x_research","social_search","creator_research"]', '["x","instagram"]', 'vault://socialdatax-api-key', 'paid provider; key in vault only', false),
  ('crawl4ai',      'Crawl4AI',        'web',     'CLIWorker',     '["web_extraction"]', '[]', NULL, 'self-hosted LLM-friendly crawler', false)
ON CONFLICT (id) DO NOTHING;

-- 防御: 0017 应已注册这些源, 但老库可能被清过, 先补齐再挂 provider FK
INSERT INTO intelligence_sources (id, name, category, access_method, capabilities, platforms, auth_ref, terms, enabled) VALUES
  ('reach-mcp', 'reach-mcp', 'social', 'MCPServer', '["social_search","x_research","reddit_research"]', '["x","reddit","youtube","bilibili"]', NULL, 'MCP server over public pages; no login bypass', true)
ON CONFLICT (id) DO NOTHING;

-- ============ Provider 注册 (keyed/self-host, 未配置 key 前 health=unknown, 审计标 not_configured) ============
INSERT INTO intel_providers (source_id, name, endpoint, cost_model, rate_limit, health, reliability, enabled)
SELECT v.source_id, v.name, v.endpoint, v.cost_model::jsonb, v.rate_limit::jsonb, v.health, v.reliability, v.enabled
FROM (VALUES
  ('socialdatax',  'SocialDataX API',              'https://api.socialdatax.com', '{"per_1k_usd": 5.0}', '{"window_max": 600, "window_left": 600}', 'unknown', 0.90::numeric, false),
  ('x-social',     'X provider (SocialDataX/reach-mcp)', NULL,                   '{"per_1k_usd": 5.0}', '{"window_max": 600, "window_left": 600}', 'unknown', 0.80::numeric, false),
  ('reach-mcp',    'reach-mcp server',             NULL,                          '{}',                 '{"window_max": 120, "window_left": 120}', 'unknown', 0.85::numeric, false),
  ('crawl4ai',     'Crawl4AI self-host',           NULL,                          '{}',                 '{}',                                        'unknown', 0.80::numeric, false),
  ('gdelt',        'GDELT DOC 2.0 API',            'https://api.gdeltproject.org/api/v2/doc/doc', '{}', '{"window_max": 12, "window_left": 12}', 'unknown', 0.90::numeric, false),
  ('google-trends','Google Trends scraper',        NULL,                          '{}',                 '{}',                                        'unknown', 0.70::numeric, false)
) AS v(source_id, name, endpoint, cost_model, rate_limit, health, reliability, enabled)
WHERE NOT EXISTS (SELECT 1 FROM intel_providers p WHERE p.source_id = v.source_id AND p.name = v.name);

-- ============ 新采集任务种子 ============
-- 立即启用 (P0 免费公开源)
INSERT INTO intel_collection_jobs (source_id, schedule, status) VALUES
  ('google-news',  'feed:gnews: AI agent (en)',            'idle'),
  ('google-news',  'feed:gnews: large language model (en)', 'idle'),
  ('google-news',  'feed:gnews: AI startup funding (en)',   'idle'),
  ('google-news',  'feed:gnews: AI 智能体 (zh)',            'idle'),
  ('google-news',  'feed:gnews: 大模型 (zh)',               'idle'),
  ('producthunt',  'feed:ph: tech',                        'idle'),
  ('producthunt',  'feed:ph: latest',                      'idle'),
  ('bilibili',     'feed:bili: rank all',                  'idle'),
  ('bilibili',     'feed:bili: rank tech',                 'idle')
ON CONFLICT (source_id, schedule) DO NOTHING;

-- 已注册但待配置 (审计显示 not_configured)
INSERT INTO intel_collection_jobs (source_id, schedule, status) VALUES
  ('gdelt',         'feed:gdelt: ai agent',                 'paused'),
  ('google-trends', 'daily US',                             'paused'),
  ('reddit',        'feed:reddit: r/MachineLearning',       'paused'),
  ('reddit',        'feed:reddit: r/artificial',            'paused'),
  ('reddit',        'feed:reddit: r/LocalLLaMA',            'paused'),
  ('zhihu-hot',     'feed:zhihu hot',                       'paused'),
  ('weibo-hot',     'feed:weibo hot',                       'paused'),
  ('douyin-hot',    'feed:douyin hot',                      'paused'),
  ('baidu-hot',     'feed:baidu hot',                       'paused'),
  ('x-social',      'hourly',                               'paused')
ON CONFLICT (source_id, schedule) DO NOTHING;

COMMIT;
