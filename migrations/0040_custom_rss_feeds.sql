-- 0040_custom_rss_feeds: 自定义 RSS 源 (零代码接入)
--
-- 新增信源不再改代码: 往 intel_custom_feeds 加一行即可, rss-custom 连接器
-- (apps/cloud-api/src/pipeline/connectors.rs::custom_rss_manifest) 每轮调度时
-- 从该表读取启用中的源, 走统一 HttpConnector + parse_rss 进管线。
-- 也可以 POST /api/v1/intel/feeds/import 从 awesome-rss 清单页批量导入。
-- 红线不变: 只抓公开只读 feed, 礼貌 UA + 429 退避。

BEGIN;

CREATE TABLE IF NOT EXISTS intel_custom_feeds (
    id          TEXT PRIMARY KEY REFERENCES intelligence_sources (id),
    label       TEXT NOT NULL,
    url         TEXT NOT NULL,
    origin      TEXT,                       -- 来源清单页 URL / 手工
    enabled     BOOLEAN NOT NULL DEFAULT true,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_custom_feeds_enabled ON intel_custom_feeds (enabled);

-- ============ 注册信源 (精选 awesome-rss-feeds 清单, 均为站点自有公开 feed) ============
INSERT INTO intelligence_sources (id, name, category, access_method, capabilities, platforms, terms, enabled) VALUES
  ('openai-news',     'OpenAI News',          'news', 'RSSSource', '["news_research","trend_search"]', '["openai"]',   'public RSS, no key', true),
  ('deepmind-blog',   'Google DeepMind Blog', 'news', 'RSSSource', '["news_research","research"]',       '["deepmind"]', 'public RSS, no key', true),
  ('google-ai-blog',  'Google AI Blog',       'news', 'RSSSource', '["news_research"]',                   '["google"]',   'public RSS, no key', true),
  ('google-research', 'Google Research Blog', 'news', 'RSSSource', '["research","news_research"]',       '["google"]',   'public Atom, no key', true),
  ('google-cloud',    'Google Cloud Blog',    'news', 'RSSSource', '["news_research"]',                   '["gcp"]',      'public RSS, no key', true),
  ('ms-azure-blog',   'Azure Blog',           'news', 'RSSSource', '["news_research"]',                   '["azure"]',    'public RSS, no key', true),
  ('mit-ai-news',     'MIT News · AI',        'news', 'RSSSource', '["news_research"]',                   '["mit"]',      'public RSS, no key', true),
  ('berkeley-bair',   'Berkeley AI Research', 'news', 'RSSSource', '["research","news_research"]',       '["berkeley"]', 'public feed.xml, no key', true),
  ('arstechnica',     'Ars Technica',         'news', 'RSSSource', '["news_research"]',                   '["ars"]',      'public RSS, no key', true),
  ('theverge',        'The Verge',            'news', 'RSSSource', '["news_research","trend_search"]',   '["verge"]',    'public RSS, no key', true),
  ('techcrunch',      'TechCrunch',           'news', 'RSSSource', '["news_research","trend_search"]',   '["techcrunch"]','public feedburner RSS, no key', true),
  ('simonwillison',   'Simon Willison Blog',  'blog', 'RSSSource', '["news_research"]',                   '["blog"]',     'public Atom, no key', true),
  ('latent-space',    'Latent.Space',         'blog', 'RSSSource', '["news_research","trend_search"]',   '["ai"]',       'public feed, no key', true),
  ('github-blog',     'GitHub Blog',          'blog', 'RSSSource', '["news_research"]',                   '["github"]',   'public RSS, no key', true),
  ('cloudflare-blog', 'Cloudflare Blog',      'blog', 'RSSSource', '["news_research"]',                   '["cloudflare"]','public RSS, no key', true),
  ('stripe-blog',     'Stripe Blog',          'blog', 'RSSSource', '["news_research"]',                   '["stripe"]',   'public RSS, no key', true),
  ('mozilla-hacks',    'Mozilla Hacks',        'blog', 'RSSSource', '["news_research"]',                   '["mozilla"]',  'public RSS, no key', true),
  ('baoyu-blog',      '宝玉的分享',            'blog', 'RSSSource', '["news_research","trend_search"]',   '["blog"]',     'public feed.xml, no key', true),
  ('qbitai',          '量子位',               'news', 'RSSSource', '["news_research","trend_search"]',   '["qbitai"]',   'public feed, no key', true),
  ('36kr',            '36氪',                 'news', 'RSSSource', '["news_research","trend_search"]',   '["36kr"]',     'public feed, no key', true),
  ('geekpark',        '极客公园',              'news', 'RSSSource', '["news_research","trend_search"]',   '["geekpark"]', 'public RSS, no key', true),
  ('segmentfault',    'SegmentFault',         'news', 'RSSSource', '["news_research"]',                   '["segmentfault"]','public feed, no key', true),
  ('stormraiser',     '火星计算研究所',        'blog', 'RSSSource', '["research","news_research"]',       '["blog"]',     'public feed, no key', true),
  ('youkaichao',      'Kaichao You Blog',     'blog', 'RSSSource', '["research"]',                        '["blog"]',     'public feed.xml, no key', true)
ON CONFLICT (id) DO NOTHING;

-- ============ 自定义 feed 清单 (首批来自 awesome-rss-feeds 精选) ============
INSERT INTO intel_custom_feeds (id, label, url, origin, enabled) VALUES
  ('openai-news',     'openai news',        'https://openai.com/news/rss.xml',                    'awesome-rss-feeds', true),
  ('deepmind-blog',   'deepmind blog',      'https://deepmind.google/blog/rss.xml',              'awesome-rss-feeds', true),
  ('google-ai-blog',  'google ai blog',     'https://blog.google/technology/ai/rss',             'awesome-rss-feeds', true),
  ('google-research', 'google research',    'https://googleresearch.blogspot.com/atom.xml',      'awesome-rss-feeds', true),
  ('google-cloud',    'google cloud blog',  'https://cloudblog.withgoogle.com/rss',              'awesome-rss-feeds', true),
  ('ms-azure-blog',   'azure blog',         'https://azure.microsoft.com/en-us/blog/feed',       'awesome-rss-feeds', true),
  ('mit-ai-news',     'mit ai news',        'http://news.mit.edu/rss/topic/artificial-intelligence2', 'awesome-rss-feeds', true),
  ('berkeley-bair',   'berkeley bair',      'http://bair.berkeley.edu/blog/feed.xml',            'awesome-rss-feeds', true),
  ('arstechnica',     'arstechnica',        'https://feeds.arstechnica.com/arstechnica/index',   'awesome-rss-feeds', true),
  ('theverge',        'theverge',           'https://www.theverge.com/rss/full.xml',             'awesome-rss-feeds', true),
  ('techcrunch',      'techcrunch',         'http://feeds.feedburner.com/TechCrunch',            'awesome-rss-feeds', true),
  ('simonwillison',   'simonwillison',      'https://simonwillison.net/atom/everything',         'awesome-rss-feeds', true),
  ('latent-space',    'latent.space',       'https://www.latent.space/feed',                     'awesome-rss-feeds', true),
  ('github-blog',     'github blog',        'https://github.blog/feed',                          'awesome-rss-feeds', true),
  ('cloudflare-blog', 'cloudflare blog',    'https://blog.cloudflare.com/rss',                   'awesome-rss-feeds', true),
  ('stripe-blog',     'stripe blog',        'https://stripe.com/blog/feed.rss',                  'awesome-rss-feeds', true),
  ('mozilla-hacks',    'mozilla hacks',      'https://hacks.mozilla.org/feed',                    'awesome-rss-feeds', true),
  ('baoyu-blog',      '宝玉的分享',          'https://baoyu.io/feed.xml',                         'awesome-rss-feeds', true),
  ('qbitai',          '量子位',             'https://www.qbitai.com/feed',                       'awesome-rss-feeds', true),
  ('36kr',            '36氪',               'https://36kr.com/feed',                             'awesome-rss-feeds', true),
  ('geekpark',        '极客公园',            'https://www.geekpark.net/rss',                      'awesome-rss-feeds', true),
  ('segmentfault',    'segmentfault',       'https://segmentfault.com/feeds/blogs',              'awesome-rss-feeds', true),
  ('stormraiser',     '火星计算研究所',      'https://stormraiser.me/feed',                       'awesome-rss-feeds', true),
  ('youkaichao',      'youkaichao blog',    'https://youkaichao.github.io/feed.xml',             'awesome-rss-feeds', true)
ON CONFLICT (id) DO NOTHING;

COMMIT;
