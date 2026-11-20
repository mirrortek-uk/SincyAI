-- 0017_intel_seed: seed the World Intelligence source registry (WI §6) with
-- the guide's evaluated integrations as metadata-only registrations, plus a
-- few providers and demo trend items so the opportunity engine has signal.
-- Red line (WI §29): these are read-only public sources/tools; Sincy never
-- bypasses platform auth, CAPTCHA, or rate limits.

BEGIN;

INSERT INTO intelligence_sources (id, name, category, access_method, capabilities, platforms, auth_ref, terms, enabled) VALUES
  ('rsshub',       'RSSHub',       'rss',    'ExternalService', '["rss_research","news_research"]', '[]', NULL, 'public RSS aggregator; respect each feed ToS', true),
  ('searxng',      'SearXNG',      'search', 'ExternalService', '["web_search"]', '[]', NULL, 'self-hosted metasearch; no API key needed', true),
  ('trendradar',   'TrendRadar',   'trend',  'ExternalService', '["trend_search"]', '["weibo","zhihu","douyin","xiaohongshu","bilibili"]', NULL, 'public hot-search aggregator', true),
  ('hotrank-hub',  'HotRank Hub',  'rank',   'ExternalService', '["trend_search","creator_research"]', '["weibo","zhihu","bilibili","douyin"]', NULL, 'public rank lists', true),
  ('reach-mcp',    'reach-mcp',    'social', 'MCPServer',       '["social_search","x_research","reddit_research"]', '["x","reddit","youtube","bilibili"]', NULL, 'MCP server over public pages; no login bypass', true),
  ('tikhub',       'TikHub',       'social', 'RESTProvider',    '["social_search","douyin_research","xiaohongshu_research","creator_research"]', '["douyin","xiaohongshu","tiktok"]', 'vault://tikhub-api-key', 'paid provider; key in vault only', true),
  ('firecrawl',    'Firecrawl',    'web',    'RESTProvider',    '["web_extraction"]', '[]', 'vault://firecrawl-api-key', 'paid provider; key in vault only', true)
ON CONFLICT (id) DO NOTHING;

-- providers: one per paid/external source that needs an endpoint + budget
INSERT INTO intel_providers (source_id, name, endpoint, cost_model, rate_limit, health, reliability, enabled) VALUES
  ('tikhub',    'TikHub API',    'https://api.tikhub.io', '{"per_1k_usd": 1.0}',  '{"window_max": 1000, "window_left": 1000}', 'unknown', 0.95, true),
  ('firecrawl', 'Firecrawl API', 'https://api.firecrawl.dev/v1', '{"per_1k_usd": 3.0}', '{"window_max": 500, "window_left": 500}', 'unknown', 0.90, true),
  ('reach-mcp', 'reach-mcp',     NULL, '{"per_1k_usd": 0.0}', '{"window_max": 10000, "window_left": 10000}', 'healthy', 0.85, true)
ON CONFLICT DO NOTHING;

-- demo trend signals so POST /intel/opportunities has something to rank
INSERT INTO intel_items (source_id, kind, dedup_key, payload) VALUES
  ('trendradar', 'trend', 'trend:ai-agents-2026', '{"topic":"AI agents","platforms":["weibo","zhihu"],"volume":48000,"velocity":6.2,"sentiment":0.72,"window_hours":24}'),
  ('trendradar', 'trend', 'trend:llm-ops-2026', '{"topic":"LLM ops","platforms":["weibo","zhihu"],"volume":21000,"velocity":3.1,"sentiment":0.66,"window_hours":24}'),
  ('hotrank-hub', 'trend', 'trend:local-ai-2026', '{"topic":"local AI models","platforms":["bilibili","zhihu"],"volume":9500,"velocity":2.4,"sentiment":0.61,"window_hours":24}'),
  ('trendradar', 'trend', 'trend:multi-agent-2026', '{"topic":"multi-agent systems","platforms":["zhihu","weibo"],"volume":12000,"velocity":4.0,"sentiment":0.7,"window_hours":24}')
ON CONFLICT (dedup_key) DO NOTHING;

COMMIT;
