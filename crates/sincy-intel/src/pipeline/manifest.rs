//! Connector Manifest (设计 §12): 把现役数据源以"连接器清单"注册,
//! 供架构图点击 Connector 节点展开查看: 每个 Connector 的名称 / 功能(capabilities) /
//! 覆盖的内容源(sources, 含 url 与识别出的 record_type) / 现役采集实现。
//!
//! 注意: 清单是"声明"(descriptor), 现役抓取仍由 intel_collector / pulse 双写产出 RawRecord;
//! 迁移到独立 `impl Connector` 时按此清单逐个替换 fetch_impl。

use serde::{Deserialize, Serialize};

use super::connector::{AuthDecl, ConnectorDescriptor, DatasetDecl};
use super::record::RecordType;

/// 一个具体内容源 (某条 feed / 某个 provider dataset)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceRef {
    pub source_id: String,
    pub label: String,
    pub url: String,
    /// collector feed kind (news/trend/social_content/event/...)
    pub kind: String,
    /// 该源内容被识别成的 record_type (进入哪条 pipeline)
    pub record_type: RecordType,
    /// 是否启用 (自托管扩展类默认 false, 配置后才启用)
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// Connector 清单 = descriptor + 覆盖的内容源
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorManifest {
    #[serde(flatten)]
    pub descriptor: ConnectorDescriptor,
    pub sources: Vec<SourceRef>,
    /// 现役采集实现: collector:<Parser> 或 pulse:<provider>
    pub fetch_impl: String,
}

fn kind_rt(kind: &str) -> RecordType {
    match kind {
        "news" | "blog" => RecordType::Article,
        "trend" | "hot" => RecordType::Trend,
        "rank" => RecordType::Ranking,
        "social_content" => RecordType::SocialPost,
        "event" => RecordType::Event,
        "knowledge" => RecordType::ResearchPaper,
        "video" => RecordType::Video,
        "product" => RecordType::Product,
        _ => RecordType::Article,
    }
}

fn src(source_id: &str, label: &str, url: &str, kind: &str) -> SourceRef {
    SourceRef {
        source_id: source_id.into(),
        label: label.into(),
        url: url.into(),
        record_type: kind_rt(kind),
        kind: kind.into(),
        enabled: true,
    }
}

fn src_off(source_id: &str, label: &str, url: &str, kind: &str) -> SourceRef {
    SourceRef {
        enabled: false,
        ..src(source_id, label, url, kind)
    }
}

#[allow(clippy::too_many_arguments)]
fn manifest(
    id: &str,
    name: &str,
    connector_type: &str,
    capabilities: &[&str],
    datasets: Vec<(&str, RecordType, Option<&str>)>,
    interval: u64,
    fetch_impl: &str,
    sources: Vec<SourceRef>,
) -> ConnectorManifest {
    ConnectorManifest {
        descriptor: ConnectorDescriptor {
            id: id.into(),
            name: name.into(),
            connector_type: connector_type.into(),
            capabilities: capabilities.iter().map(|s| s.to_string()).collect(),
            datasets: datasets
                .into_iter()
                .map(|(id, rt, url)| DatasetDecl {
                    id: id.into(),
                    record_type: rt,
                    url: url.map(|u| u.to_string()),
                })
                .collect(),
            default_interval_secs: interval,
            rate_limit_per_min: 30,
            auth: AuthDecl::default(),
        },
        sources,
        fetch_impl: fetch_impl.into(),
    }
}

/// 全部内置 Connector 清单 (与 intel_collector 默认 feed + pulse provider 对齐)
pub fn default_connector_manifests() -> Vec<ConnectorManifest> {
    vec![
        manifest(
            "arxiv-atom",
            "arXiv 论文 (Atom)",
            "rss",
            &["research_discovery", "paper_feed"],
            vec![("arxiv_feeds", RecordType::ResearchPaper, None)],
            1800,
            "collector:Rss",
            vec![
                src(
                    "arxiv",
                    "arxiv: ai agent",
                    "https://export.arxiv.org/api/query?search_query=all:%22ai%20agent%22&max_results=100",
                    "knowledge",
                ),
                src(
                    "arxiv",
                    "arxiv: large language model",
                    "https://export.arxiv.org/api/query?search_query=all:%22large%20language%20model%22&max_results=100",
                    "knowledge",
                ),
                src(
                    "arxiv",
                    "arxiv: multi-agent",
                    "https://export.arxiv.org/api/query?search_query=all:%22multi-agent%20system%22&max_results=100",
                    "knowledge",
                ),
                src(
                    "arxiv",
                    "arxiv: RAG",
                    "https://export.arxiv.org/api/query?search_query=all:%22retrieval%20augmented%20generation%22&max_results=100",
                    "knowledge",
                ),
                src(
                    "arxiv",
                    "arxiv: llm eval",
                    "https://export.arxiv.org/api/query?search_query=all:%22llm%20evaluation%22&max_results=100",
                    "knowledge",
                ),
            ],
        ),
        manifest(
            "hn-algolia",
            "Hacker News Algolia API",
            "api",
            &["story_discovery", "front_page", "tech_search"],
            vec![
                ("hn_stories", RecordType::SocialPost, None),
                ("hn_front", RecordType::Trend, None),
            ],
            1800,
            "collector:HnJson",
            vec![
                src(
                    "hn-algolia",
                    "hn: recent stories",
                    "https://hn.algolia.com/api/v1/search_by_date?tags=story&hitsPerPage=100",
                    "social_content",
                ),
                src(
                    "hn-algolia",
                    "hn: front page",
                    "https://hn.algolia.com/api/v1/search?tags=front_page&hitsPerPage=100",
                    "trend",
                ),
                src(
                    "hn-algolia",
                    "hn: search AI",
                    "https://hn.algolia.com/api/v1/search_by_date?query=AI%20agent&tags=story&hitsPerPage=100",
                    "social_content",
                ),
            ],
        ),
        manifest(
            "devto-api",
            "dev.to 文章 API",
            "api",
            &["developer_blog", "tag_feed"],
            vec![("devto_articles", RecordType::Article, None)],
            1800,
            "collector:DevToJson",
            vec![
                src(
                    "devto-api",
                    "devto: #ai",
                    "https://dev.to/api/articles?tag=ai&per_page=100",
                    "news",
                ),
                src(
                    "devto-api",
                    "devto: #llm",
                    "https://dev.to/api/articles?tag=llm&per_page=100",
                    "news",
                ),
                src(
                    "devto-api",
                    "devto: #agents",
                    "https://dev.to/api/articles?tag=agents&per_page=100",
                    "news",
                ),
                src(
                    "devto-api",
                    "devto: #opensource",
                    "https://dev.to/api/articles?tag=opensource&per_page=100",
                    "news",
                ),
            ],
        ),
        manifest(
            "wikipedia-rc",
            "Wikipedia 最近变更",
            "api",
            &["event_stream", "encyclopedia_events"],
            vec![("wiki_recentchanges", RecordType::Event, None)],
            900,
            "collector:WikiRcJson",
            vec![src(
                "wikipedia-rc",
                "wiki: recentchanges",
                "https://en.wikipedia.org/w/api.php?action=query&list=recentchanges&rcnamespace=0&rclimit=500&format=json",
                "event",
            )],
        ),
        manifest(
            "github-trending",
            "GitHub 仓库趋势 (Search API)",
            "api",
            &["repo_ranking", "topic_trending"],
            vec![("github_trending", RecordType::Ranking, None)],
            3600,
            "collector:GithubTrending",
            vec![
                src(
                    "hotrank-hub",
                    "github trending (ai agent)",
                    "https://api.github.com/search/repositories?q=ai+agent+in:description&sort=stars&order=desc&per_page=30",
                    "trend",
                ),
                src(
                    "hotrank-hub",
                    "github trending (llm)",
                    "https://api.github.com/search/repositories?q=llm+in:description&sort=stars&order=desc&per_page=30",
                    "trend",
                ),
                src(
                    "hotrank-hub",
                    "github trending (agent framework)",
                    "https://api.github.com/search/repositories?q=agent+framework+in:name,description&sort=stars&order=desc&per_page=30",
                    "trend",
                ),
            ],
        ),
        manifest(
            "bilibili-rank",
            "B站排行榜 API",
            "api",
            &["video_ranking", "tech_ranking"],
            vec![("bili_ranking", RecordType::Ranking, None)],
            3600,
            "collector:BiliRank",
            vec![
                src(
                    "bilibili",
                    "bili: rank all",
                    "https://api.bilibili.com/x/web-interface/ranking/v2?rid=0&type=all",
                    "trend",
                ),
                src(
                    "bilibili",
                    "bili: rank tech",
                    "https://api.bilibili.com/x/web-interface/ranking/v2?rid=36&type=all",
                    "trend",
                ),
            ],
        ),
        manifest(
            "rss-news",
            "中文科技新闻 RSS",
            "rss",
            &["news_aggregation"],
            vec![("zh_tech_news", RecordType::Article, None)],
            1800,
            "collector:Rss",
            vec![
                src("rsshub", "ithome", "https://www.ithome.com/rss/", "news"),
                src("sspai", "sspai", "https://sspai.com/feed", "news"),
                src("juejin", "juejin", "https://juejin.cn/rss", "news"),
            ],
        ),
        manifest(
            "v2ex-rss",
            "V2EX 热帖 RSS",
            "rss",
            &["community_hot"],
            vec![("v2ex_hot", RecordType::SocialPost, None)],
            1800,
            "collector:Rss",
            vec![src(
                "v2ex",
                "v2ex hot",
                "https://www.v2ex.com/feed/tab/hot.xml",
                "social_content",
            )],
        ),
        manifest(
            "youtube-rss",
            "YouTube 频道更新 RSS",
            "rss",
            &["video_feed", "creator_content"],
            vec![("yt_channels", RecordType::Video, None)],
            3600,
            "collector:Rss",
            vec![
                src(
                    "youtube-rss",
                    "yt: 3Blue1Brown",
                    "https://www.youtube.com/feeds/videos.xml?channel_id=UCYO_jab_esuFRV4b17AJtAw",
                    "social_content",
                ),
                src(
                    "youtube-rss",
                    "yt: Fireship",
                    "https://www.youtube.com/feeds/videos.xml?channel_id=UCsBjURrPoezykLs9EqgamOA",
                    "social_content",
                ),
                src(
                    "youtube-rss",
                    "yt: Yannic Kilcher",
                    "https://www.youtube.com/feeds/videos.xml?channel_id=UCZHmQk67mSJgfCCTn7xBfew",
                    "social_content",
                ),
            ],
        ),
        manifest(
            "google-news-rss",
            "Google News RSS 搜索",
            "rss",
            &["news_search", "zh_en_coverage"],
            vec![("gnews", RecordType::Article, None)],
            1800,
            "collector:Rss",
            vec![
                src(
                    "google-news",
                    "gnews: AI agent (en)",
                    "https://news.google.com/rss/search?q=AI+agent&hl=en-US&gl=US&ceid=US:en",
                    "news",
                ),
                src(
                    "google-news",
                    "gnews: large language model (en)",
                    "https://news.google.com/rss/search?q=large+language+model&hl=en-US&gl=US&ceid=US:en",
                    "news",
                ),
                src(
                    "google-news",
                    "gnews: AI startup funding (en)",
                    "https://news.google.com/rss/search?q=AI+startup+funding&hl=en-US&gl=US&ceid=US:en",
                    "news",
                ),
                src(
                    "google-news",
                    "gnews: AI 智能体 (zh)",
                    "https://news.google.com/rss/search?q=AI%20%E6%99%BA%E8%83%BD%E4%BD%93&hl=zh-CN&gl=CN&ceid=CN:zh-Hans",
                    "news",
                ),
                src(
                    "google-news",
                    "gnews: 大模型 (zh)",
                    "https://news.google.com/rss/search?q=%E5%A4%A7%E6%A8%A1%E5%9E%8B&hl=zh-CN&gl=CN&ceid=CN:zh-Hans",
                    "news",
                ),
            ],
        ),
        manifest(
            "producthunt-atom",
            "Product Hunt 新品",
            "rss",
            &["product_launch", "tech_products"],
            vec![("ph_products", RecordType::Product, None)],
            3600,
            "collector:Rss",
            vec![
                src(
                    "producthunt",
                    "ph: tech",
                    "https://www.producthunt.com/feed?category=tech",
                    "trend",
                ),
                src(
                    "producthunt",
                    "ph: latest",
                    "https://www.producthunt.com/feed",
                    "trend",
                ),
            ],
        ),
        manifest(
            "gdelt-api",
            "GDELT DOC 2.0 (需 SINCY_INTEL_GDELT=1 + 专用出口 IP)",
            "api",
            &["global_news", "event_detection"],
            vec![("gdelt_artlist", RecordType::Article, None)],
            3600,
            "collector:GdeltJson",
            vec![src_off(
                "gdelt",
                "gdelt: ai agent",
                "https://api.gdeltproject.org/api/v2/doc/doc?query=%22ai%20agent%22&mode=artlist&maxrecords=100&format=json",
                "news",
            )],
        ),
        manifest(
            "rsshub-ext",
            "自托管 RSSHub 扩展 (中文热榜/Reddit, 需 SINCY_INTEL_RSSHUB_BASE)",
            "rss",
            &["zh_hotlists", "reddit_hot"],
            vec![("rsshub_routes", RecordType::Trend, None)],
            1800,
            "collector:Rss",
            vec![
                src_off(
                    "rsshub",
                    "rsshub: 36kr 快讯",
                    "{SINCY_INTEL_RSSHUB_BASE}/36kr/newsflash",
                    "news",
                ),
                src_off(
                    "zhihu-hot",
                    "zhihu hot",
                    "{SINCY_INTEL_RSSHUB_BASE}/zhihu/hot",
                    "trend",
                ),
                src_off(
                    "douyin-hot",
                    "douyin hot",
                    "{SINCY_INTEL_RSSHUB_BASE}/douyin/hot",
                    "trend",
                ),
                src_off(
                    "weibo-hot",
                    "weibo hot",
                    "{SINCY_INTEL_RSSHUB_BASE}/weibo/hot",
                    "trend",
                ),
                src_off(
                    "baidu-hot",
                    "baidu hot",
                    "{SINCY_INTEL_RSSHUB_BASE}/baidu/hot",
                    "trend",
                ),
                src_off(
                    "reddit",
                    "reddit: r/MachineLearning",
                    "{SINCY_INTEL_RSSHUB_BASE}/reddit/r/MachineLearning/hot",
                    "social_content",
                ),
                src_off(
                    "reddit",
                    "reddit: r/artificial",
                    "{SINCY_INTEL_RSSHUB_BASE}/reddit/r/artificial/hot",
                    "social_content",
                ),
                src_off(
                    "reddit",
                    "reddit: r/LocalLLaMA",
                    "{SINCY_INTEL_RSSHUB_BASE}/reddit/r/LocalLLaMA/hot",
                    "social_content",
                ),
            ],
        ),
        manifest(
            "searxng-ext",
            "自托管 SearXNG 搜索 (需 SINCY_INTEL_SEARXNG_BASE)",
            "search",
            &["web_search", "serp"],
            vec![("searxng_search", RecordType::SearchResult, None)],
            3600,
            "collector:SearxngJson",
            vec![
                src_off(
                    "searxng",
                    "searxng: AI agents 2026",
                    "{SINCY_INTEL_SEARXNG_BASE}/search?q=AI+agents+2026&format=json",
                    "knowledge",
                ),
                src_off(
                    "searxng",
                    "searxng: 多智能体 协作",
                    "{SINCY_INTEL_SEARXNG_BASE}/search?q=%E5%A4%9A%E6%99%BA%E8%83%BD%E4%BD%93+%E5%8D%8F%E4%BD%9C&format=json",
                    "knowledge",
                ),
            ],
        ),
        manifest(
            "pulse-x-ecology",
            "Sincy Pulse · X 生态聚合站 (7 个公开站)",
            "aggregator",
            &[
                "creator_ranking",
                "viral_content",
                "account_growth",
                "trending_topics",
                "account_health",
            ],
            vec![
                ("creator_rankings", RecordType::Ranking, None),
                ("viral_posts", RecordType::SocialPost, None),
                ("account_growth", RecordType::Metric, None),
                ("trending_topics", RecordType::Trend, None),
            ],
            1800,
            "pulse:WebScrapeProvider",
            vec![
                src(
                    "xbangdan",
                    "xbangdan 排行榜/爆帖",
                    "https://xbangdan.com/",
                    "rank",
                ),
                src("xpaiming", "xpaiming 排名", "https://xpaiming.com/", "rank"),
                src("xranks", "xranks 排行", "https://xranks.com/", "rank"),
                src("trends24", "trends24 趋势", "https://trends24.in/", "trend"),
                src(
                    "tide-now",
                    "tide-now 潮汐趋势",
                    "https://tide-now.com/",
                    "trend",
                ),
                src(
                    "shadowban-radar",
                    "shadowban-radar 账号健康",
                    "https://shadowban-radar.com/",
                    "rank",
                ),
                src(
                    "getxanalytics",
                    "getxanalytics 帖子分析",
                    "https://getxanalytics.com/",
                    "social_content",
                ),
            ],
        ),
    ]
}
