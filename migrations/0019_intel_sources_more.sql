-- 0019_intel_sources_more: register the high-volume READ-ONLY public APIs the
-- collector actually pulls from (all no-auth, UA-identified, rate-limit
-- friendly). Red line (WI §29) unchanged: no platform login/CAPTCHA bypass —
-- X/Weibo hot lists are intentionally NOT scraped; we use their public
-- read-only APIs / RSS only.

BEGIN;

INSERT INTO intelligence_sources (id, name, category, access_method, capabilities, platforms, terms, enabled) VALUES
  ('arxiv',        'arXiv API',        'knowledge', 'RESTProvider',    '["paper_research","knowledge"]', '[]', 'export.arxiv.org API, no key, polite rate', true),
  ('hn-algolia',   'HN Algolia API',   'rank',      'RESTProvider',    '["social_search","trend_search"]', '["hackernews"]', 'hn.algolia.com free API', true),
  ('devto-api',    'dev.to API',       'blog',      'RESTProvider',    '["news_research","social_search"]', '["devto"]', 'dev.to public articles API, per_page<=100', true),
  ('wikipedia-rc', 'Wikipedia RC API', 'event',     'RESTProvider',    '["event_stream","news_research"]', '["wikipedia"]', 'wikimedia API with UA, rclimit<=500', true),
  ('v2ex',         'V2EX hot',         'social',    'RSSSource',       '["social_search","trend_search"]', '["v2ex"]', 'public RSS feed', true),
  ('juejin',       '掘金 RSS',          'blog',      'RSSSource',       '["news_research"]', '["juejin"]', 'public RSS feed', true),
  ('sspai',        '少数派 RSS',        'blog',      'RSSSource',       '["news_research"]', '["sspai"]', 'public RSS feed', true),
  ('youtube-rss',  'YouTube RSS',      'video',     'RSSSource',       '["social_search","creator_research"]', '["youtube"]', 'public channel video feeds', true)
ON CONFLICT (id) DO NOTHING;

ALTER TABLE intel_collection_jobs ADD COLUMN IF NOT EXISTS last_inserted INTEGER;

INSERT INTO intel_collection_jobs (source_id, schedule, status) VALUES
  ('arxiv', 'feed:arxiv x5', 'idle'),
  ('hn-algolia', 'feed:hn x3', 'idle'),
  ('devto-api', 'feed:devto x4', 'idle'),
  ('wikipedia-rc', 'feed:wiki-rc', 'idle'),
  ('v2ex', 'feed:v2ex-hot', 'idle'),
  ('juejin', 'feed:juejin', 'idle'),
  ('sspai', 'feed:sspai', 'idle'),
  ('youtube-rss', 'feed:yt x3', 'idle')
ON CONFLICT (source_id, schedule) DO NOTHING;

COMMIT;
