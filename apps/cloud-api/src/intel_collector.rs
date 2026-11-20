//! World Intelligence collector (WI §9 scheduler): periodically pulls from
//! REGISTERED read-only public sources, normalizes into the WI §11 unified
//! model and upserts into intel_items with dedup_key (WI §12).
//!
//! Red line (WI §29): only public, read-only endpoints of registered sources;
//! modest rate (one run per interval, per-feed item cap, polite User-Agent,
//! short timeout). Never bypasses platform auth, CAPTCHA or rate limits —
//! a failing source is recorded as failed, never retried aggressively.

use serde_json::{Value, json};
use sha2::Digest;
use std::time::Duration;
use uuid::Uuid;

/// One fetch target. `parser` picks the normalizer: rss | atom | github_trending | searxng_json.
#[derive(Debug, Clone)]
pub struct FeedConfig {
    pub source_id: String,
    pub label: String,
    pub url: String,
    pub kind: String,
    pub parser: Parser,
    pub max_items: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parser {
    Rss, // RSS 2.0 + Atom (arxiv uses Atom <entry>, handled by the same parser)
    SearxngJson,
    GithubTrending,
    HnJson,     // HN Algolia: hits -> social_content/trend
    DevToJson,  // dev.to articles API
    WikiRcJson, // Wikipedia recentchanges
}

#[derive(Debug, Clone)]
pub struct CollectorConfig {
    pub feeds: Vec<FeedConfig>,
    pub interval_secs: u64,
}

impl CollectorConfig {
    /// Built-in default feeds (public read-only endpoints) + env overrides:
    /// - SINCY_INTEL_RSSHUB_BASE replaces the RSS base host (self-hosted RSSHub)
    /// - SINCY_INTEL_SEARXNG_BASE replaces the SearXNG instance
    /// - SINCY_INTEL_COLLECT_INTERVAL_SECS sets the loop interval
    pub fn from_env() -> Self {
        // Self-hosted overrides (WI §26). Public SearXNG instances sit behind
        // bot checks — per the red line (WI §29) we do NOT bypass them, so
        // searxng feeds only run when pointed at a self-hosted instance.
        let rsshub_base = std::env::var("SINCY_INTEL_RSSHUB_BASE").ok();
        let searx_base = std::env::var("SINCY_INTEL_SEARXNG_BASE").ok();
        let interval_secs = std::env::var("SINCY_INTEL_COLLECT_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1800); // default: one pass every 30 min
        let feed =
            |source: &str, label: &str, url: String, kind: &str, parser: Parser| FeedConfig {
                source_id: source.into(),
                label: label.into(),
                url,
                kind: kind.into(),
                parser,
                max_items: 100,
            };
        let mut feeds = vec![
            // ===== 高吞吐公开 API =====
            // arXiv 论文 (Atom) — AI 研究, 每查询 100 篇
            feed("arxiv", "arxiv: ai agent",        "https://export.arxiv.org/api/query?search_query=all:%22ai%20agent%22&sortBy=submittedDate&sortOrder=descending&max_results=100".into(), "knowledge", Parser::Rss),
            feed("arxiv", "arxiv: large language model", "https://export.arxiv.org/api/query?search_query=all:%22large%20language%20model%22&sortBy=submittedDate&sortOrder=descending&max_results=100".into(), "knowledge", Parser::Rss),
            feed("arxiv", "arxiv: multi-agent",     "https://export.arxiv.org/api/query?search_query=all:%22multi-agent%20system%22&sortBy=submittedDate&sortOrder=descending&max_results=100".into(), "knowledge", Parser::Rss),
            feed("arxiv", "arxiv: RAG",             "https://export.arxiv.org/api/query?search_query=all:%22retrieval%20augmented%20generation%22&sortBy=submittedDate&sortOrder=descending&max_results=100".into(), "knowledge", Parser::Rss),
            feed("arxiv", "arxiv: llm eval",        "https://export.arxiv.org/api/query?search_query=all:%22llm%20evaluation%22&sortBy=submittedDate&sortOrder=descending&max_results=100".into(), "knowledge", Parser::Rss),
            // Hacker News Algolia — 最近故事 100 + 首页热榜 + AI 搜索
            feed("hn-algolia", "hn: recent stories", "https://hn.algolia.com/api/v1/search_by_date?tags=story&hitsPerPage=100".into(), "social_content", Parser::HnJson),
            feed("hn-algolia", "hn: front page",      "https://hn.algolia.com/api/v1/search?tags=front_page&hitsPerPage=100".into(), "trend", Parser::HnJson),
            feed("hn-algolia", "hn: search AI",       "https://hn.algolia.com/api/v1/search_by_date?query=AI%20agent&tags=story&hitsPerPage=100".into(), "social_content", Parser::HnJson),
            // dev.to 文章 API — 每 tag 100 篇
            feed("devto-api", "devto: #ai",     "https://dev.to/api/articles?tag=ai&per_page=100".into(), "news", Parser::DevToJson),
            feed("devto-api", "devto: #llm",    "https://dev.to/api/articles?tag=llm&per_page=100".into(), "news", Parser::DevToJson),
            feed("devto-api", "devto: #agents", "https://dev.to/api/articles?tag=agents&per_page=100".into(), "news", Parser::DevToJson),
            feed("devto-api", "devto: #opensource", "https://dev.to/api/articles?tag=opensource&per_page=100".into(), "news", Parser::DevToJson),
            // Wikipedia 最近变更事件流 (500/次)
            feed("wikipedia-rc", "wiki: recentchanges", "https://en.wikipedia.org/w/api.php?action=query&list=recentchanges&rcnamespace=0&rclimit=500&format=json".into(), "event", Parser::WikiRcJson),
            // ===== 趋势榜 =====
            feed("hotrank-hub", "github trending (ai agent)",
                 "https://api.github.com/search/repositories?q=ai+agent+in:description&sort=stars&order=desc&per_page=30".into(),
                 "trend", Parser::GithubTrending),
            feed("hotrank-hub", "github trending (llm)",
                 "https://api.github.com/search/repositories?q=llm+in:description&sort=stars&order=desc&per_page=30".into(),
                 "trend", Parser::GithubTrending),
            feed("hotrank-hub", "github trending (agent framework)",
                 "https://api.github.com/search/repositories?q=agent+framework+in:name,description&sort=stars&order=desc&per_page=30".into(),
                 "trend", Parser::GithubTrending),
            // ===== RSS / 社区 =====
            feed("rsshub", "ithome", "https://www.ithome.com/rss/".into(), "news", Parser::Rss),
            feed("sspai", "sspai", "https://sspai.com/feed".into(), "news", Parser::Rss),
            feed("juejin", "juejin", "https://juejin.cn/rss".into(), "news", Parser::Rss),
            feed("v2ex", "v2ex hot", "https://www.v2ex.com/feed/tab/hot.xml".into(), "social_content", Parser::Rss),
            // YouTube 频道更新 (科技/AI 向频道)
            feed("youtube-rss", "yt: 3Blue1Brown", "https://www.youtube.com/feeds/videos.xml?channel_id=UCYO_jab_esuFRV4b17AJtAw".into(), "social_content", Parser::Rss),
            feed("youtube-rss", "yt: Fireship",          "https://www.youtube.com/feeds/videos.xml?channel_id=UCsBjURrPoezykLs9EqgamOA".into(), "social_content", Parser::Rss),
            feed("youtube-rss", "yt: Yannic Kilcher",    "https://www.youtube.com/feeds/videos.xml?channel_id=UCZHmQk67mSJgfCCTn7xBfew".into(), "social_content", Parser::Rss),
        ];
        if let Some(base) = rsshub_base {
            feeds.push(feed(
                "rsshub",
                "rsshub: 36kr 快讯",
                format!("{base}/36kr/newsflash"),
                "news",
                Parser::Rss,
            ));
        }
        if let Some(base) = searx_base {
            feeds.push(feed(
                "searxng",
                "searxng: AI agents 2026",
                format!("{base}/search?q=AI+agents+2026&format=json&count=100"),
                "knowledge",
                Parser::SearxngJson,
            ));
            feeds.push(feed("searxng", "searxng: 多智能体 协作",
                 format!("{base}/search?q=%E5%A4%9A%E6%99%BA%E8%83%BD%E4%BD%93+%E5%8D%8F%E4%BD%9C&format=json&count=100"), "knowledge", Parser::SearxngJson));
        }
        Self {
            interval_secs,
            feeds,
        }
    }
}

/// Stable dedup key: source | kind | sha256(url or title).
pub fn dedup_key(source_id: &str, kind: &str, url: &str, title: &str) -> String {
    let h = format!(
        "{:x}",
        sha2::Sha256::digest(format!("{source_id}|{kind}|{url}|{title}"))
    );
    format!("{source_id}:{kind}:{:.16}", &h)
}

// ---------- minimal RSS/Atom parsing (no extra deps) ----------

fn xml_unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&#34;", "\"")
}

fn tag_value(block: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}");
    let start = block.find(&open)?;
    let rest = &block[start..];
    let end_open = rest.find('>')?;
    let after_open = &rest[end_open + 1..];
    let close = format!("</{tag}>");
    let end = after_open.find(&close)?;
    let raw = &after_open[..end];
    // strip CDATA
    let raw = if let Some(c0) = raw.find("<![CDATA[") {
        let c1 = raw.find("]]>").map(|i| i + 3).unwrap_or(raw.len());
        &raw[c0 + 9..c1.saturating_sub(3)]
    } else {
        raw
    };
    let v = xml_unescape(raw.trim());
    if v.is_empty() { None } else { Some(v) }
}

/// Parse RSS 2.0 <item> and Atom <entry> blocks into normalized news payloads.
pub fn parse_rss(xml: &str) -> Vec<(String, String, String, Option<String>)> {
    // (title, url, summary, published_at)
    let mut out = Vec::new();
    for block in xml
        .split("<item>")
        .skip(1)
        .chain(xml.split("<entry>").skip(1))
    {
        let block = &block[..block
            .find("</item>")
            .or(block.find("</entry>"))
            .unwrap_or(block.len())];
        let title = tag_value(block, "title").unwrap_or_default();
        let url = tag_value(block, "link").unwrap_or_else(|| {
            // Atom: <link href="..."/>
            block
                .find("<link")
                .and_then(|i| {
                    block[i..].find("href=\"").map(|j| {
                        let s = &block[i + j + 6..];
                        s.split('"').next().unwrap_or("").to_string()
                    })
                })
                .unwrap_or_default()
        });
        let summary = tag_value(block, "description")
            .or_else(|| tag_value(block, "summary"))
            .or_else(|| tag_value(block, "content"))
            .unwrap_or_default();
        let published = tag_value(block, "pubDate")
            .or_else(|| tag_value(block, "published"))
            .or_else(|| tag_value(block, "updated"));
        if !title.is_empty() {
            out.push((title, url, summary, published));
        }
    }
    out
}

/// Normalize GitHub search API response into Trend payloads (WI §11 Trend).
pub fn parse_github_trending(body: &str) -> Vec<(String, Value)> {
    // (topic, payload)
    let Ok(v) = serde_json::from_str::<Value>(body) else {
        return vec![];
    };
    let Some(items) = v["items"].as_array() else {
        return vec![];
    };
    let now = chrono::Utc::now();
    let mut out = Vec::new();
    for it in items {
        let name = it["full_name"].as_str().unwrap_or("").to_string();
        if name.is_empty() {
            continue;
        }
        let stars = it["stargazers_count"].as_i64().unwrap_or(0);
        let created = it["created_at"].as_str().unwrap_or("");
        let age_days = chrono::DateTime::parse_from_rfc3339(created)
            .map(|t| (now - t.with_timezone(&chrono::Utc)).num_hours() as f64 / 24.0)
            .unwrap_or(30.0)
            .max(0.5);
        // velocity: new stars per day (trend momentum)
        let velocity = stars as f64 / age_days;
        let desc = it["description"].as_str().unwrap_or("");
        out.push((
            name.clone(),
            json!({
                "topic": name,
                "platforms": ["github"],
                "volume": stars,
                "velocity": (velocity * 100.0).round() / 100.0,
                "sentiment": 0.5,
                "window_hours": 168,
                "url": it["html_url"].as_str().unwrap_or(""),
                "description": desc,
            }),
        ));
    }
    out
}

/// Normalize SearXNG JSON results into knowledge payloads.
pub fn parse_searxng(body: &str, query: &str) -> Vec<(String, Value)> {
    let Ok(v) = serde_json::from_str::<Value>(body) else {
        return vec![];
    };
    let Some(results) = v["results"].as_array() else {
        return vec![];
    };
    results
        .iter()
        .filter_map(|r| {
            let title = r["title"].as_str()?.to_string();
            let url = r["url"].as_str().unwrap_or("").to_string();
            Some((
                title.clone(),
                json!({
                    "query": query, "title": title, "url": url,
                    "content": r["content"].as_str().unwrap_or(""),
                    "engine": r["engine"].as_str().unwrap_or(""),
                }),
            ))
        })
        .collect()
}

/// HN Algolia response -> (title, payload). kind hint comes from the feed.
pub fn parse_hn(body: &str, kind: &str) -> Vec<(String, Value)> {
    let Ok(v) = serde_json::from_str::<Value>(body) else {
        return vec![];
    };
    let Some(hits) = v["hits"].as_array() else {
        return vec![];
    };
    let now = chrono::Utc::now();
    hits.iter()
        .filter_map(|h| {
            let title = h["title"].as_str()?.to_string();
            let url = h["url"].as_str().unwrap_or("").to_string();
            let points = h["points"].as_i64().unwrap_or(0);
            let comments = h["num_comments"].as_i64().unwrap_or(0);
            let created = h["created_at"].as_str().unwrap_or("");
            let age_h = chrono::DateTime::parse_from_rfc3339(created)
                .map(|t| (now - t.with_timezone(&chrono::Utc)).num_minutes() as f64 / 60.0)
                .unwrap_or(24.0)
                .max(0.1);
            let payload = if kind == "trend" {
                json!({"topic": title, "platforms": ["hackernews"], "volume": points,
                       "velocity": (points as f64 / age_h * 10.0).round() / 10.0,
                       "sentiment": 0.5, "window_hours": 24,
                       "url": url, "comments": comments})
            } else {
                json!({"title": title, "url": url, "platform": "hackernews",
                       "points": points, "comments": comments, "published_at": created})
            };
            Some((title, payload))
        })
        .collect()
}

/// dev.to articles API -> news payloads.
pub fn parse_devto(body: &str) -> Vec<(String, Value)> {
    let Ok(v) = serde_json::from_str::<Value>(body) else {
        return vec![];
    };
    let Some(arr) = v.as_array() else {
        return vec![];
    };
    arr.iter()
        .filter_map(|a| {
            let title = a["title"].as_str()?.to_string();
            Some((
                title.clone(),
                json!({
                    "title": title, "url": a["url"].as_str().unwrap_or(""),
                    "summary": a["description"].as_str().unwrap_or(""),
                    "tags": a["tag_list"].as_array().cloned().unwrap_or_default(),
                    "published_at": a["published_at"].as_str(),
                    "platform": "devto",
                    "author": a["user"]["name"].as_str().unwrap_or(""),
                    "reactions": a["public_reactions_count"].as_i64().unwrap_or(0),
                }),
            ))
        })
        .collect()
}

/// Wikipedia recentchanges -> event payloads.
pub fn parse_wiki_rc(body: &str) -> Vec<(String, Value)> {
    let Ok(v) = serde_json::from_str::<Value>(body) else {
        return vec![];
    };
    let Some(rc) = v["query"]["recentchanges"].as_array() else {
        return vec![];
    };
    rc.iter()
        .filter_map(|r| {
            let title = r["title"].as_str()?.to_string();
            Some((
                title.clone(),
                json!({
                    "title": title, "type": r["type"].as_str().unwrap_or(""),
                    "timestamp": r["timestamp"].as_str(),
                    "user": r["user"].as_str().unwrap_or(""),
                    "platform": "wikipedia",
                    "url": format!("https://en.wikipedia.org/wiki/{}", title.replace(' ', "_")),
                }),
            ))
        })
        .collect()
}

/// One collection run over all configured feeds. Returns (inserted, duplicates, failures).
pub async fn collect_once(
    pool: &sqlx::PgPool,
    cfg: &CollectorConfig,
) -> (usize, usize, Vec<String>) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .user_agent("SincyIntelBot/1.0 (+read-only public feeds; contact: admin@sincy.dev)")
        .build()
        .unwrap();
    let mut inserted = 0usize;
    let mut dups = 0usize;
    let mut failures = Vec::new();
    for (i, f) in cfg.feeds.iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(Duration::from_millis(800)).await;
        }
        match collect_feed(pool, &client, f).await {
            Ok((ins, dup)) => {
                inserted += ins;
                dups += dup;
                mark_job(
                    pool,
                    &f.source_id,
                    &f.label,
                    "idle",
                    None,
                    Some((ins + dup) as i32),
                )
                .await;
                tracing::info!(source = %f.source_id, label = %f.label, inserted = ins, dups = dup, "intel feed collected");
            }
            Err(e) => {
                failures.push(format!("{}: {e}", f.label));
                mark_job(pool, &f.source_id, &f.label, "failed", Some(&e), None).await;
                tracing::warn!(source = %f.source_id, label = %f.label, error = %e, "intel feed failed (recorded, not retried aggressively)");
            }
        }
    }
    (inserted, dups, failures)
}

async fn collect_feed(
    pool: &sqlx::PgPool,
    client: &reqwest::Client,
    f: &FeedConfig,
) -> Result<(usize, usize), String> {
    // source must be registered (registry check, WI §6)
    let registered = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM intelligence_sources WHERE id = $1 AND enabled)",
    )
    .bind(&f.source_id)
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;
    if !registered {
        return Err(format!("source {} not registered/enabled", f.source_id));
    }
    let resp = client.get(&f.url).send().await.map_err(|e| e.to_string())?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HTTP {status}"));
    }
    let body = resp.text().await.map_err(|e| e.to_string())?;
    let items: Vec<(String, String, Value)> = match f.parser {
        Parser::Rss => parse_rss(&body)
            .into_iter()
            .take(f.max_items)
            .map(|(title, url, summary, published)| {
                let payload = json!({
                    "title": title, "url": url, "summary": summary, "published_at": published,
                    "source": f.label, "platform": "web",
                });
                let dk = dedup_key(&f.source_id, &f.kind, &url, &title);
                (dk, title, payload)
            })
            .collect(),
        Parser::GithubTrending => parse_github_trending(&body)
            .into_iter()
            .take(f.max_items)
            .map(|(topic, payload)| {
                let url = payload["url"].as_str().unwrap_or("").to_string();
                (
                    dedup_key(&f.source_id, &f.kind, &url, &topic),
                    topic,
                    payload,
                )
            })
            .collect(),
        Parser::HnJson => parse_hn(&body, &f.kind)
            .into_iter()
            .take(f.max_items)
            .map(|(title, payload)| {
                let url = payload["url"].as_str().unwrap_or("").to_string();
                (
                    dedup_key(&f.source_id, &f.kind, &url, &title),
                    title,
                    payload,
                )
            })
            .collect(),
        Parser::DevToJson => parse_devto(&body)
            .into_iter()
            .take(f.max_items)
            .map(|(title, payload)| {
                let url = payload["url"].as_str().unwrap_or("").to_string();
                (
                    dedup_key(&f.source_id, &f.kind, &url, &title),
                    title,
                    payload,
                )
            })
            .collect(),
        Parser::WikiRcJson => parse_wiki_rc(&body)
            .into_iter()
            .take(f.max_items)
            .map(|(title, payload)| {
                let url = payload["url"].as_str().unwrap_or("").to_string();
                (
                    dedup_key(&f.source_id, &f.kind, &url, &title),
                    title,
                    payload,
                )
            })
            .collect(),
        Parser::SearxngJson => {
            let q = f
                .url
                .split("q=")
                .nth(1)
                .unwrap_or("")
                .split('&')
                .next()
                .unwrap_or("")
                .to_string();
            parse_searxng(&body, &q)
                .into_iter()
                .take(f.max_items)
                .map(|(title, payload)| {
                    let url = payload["url"].as_str().unwrap_or("").to_string();
                    (
                        dedup_key(&f.source_id, &f.kind, &url, &title),
                        title,
                        payload,
                    )
                })
                .collect()
        }
    };
    let mut ins = 0usize;
    let mut dup = 0usize;
    for (dk, title, payload) in items {
        let r = sqlx::query(
            "INSERT INTO intel_items (id, source_id, kind, dedup_key, payload)
             VALUES ($1, $2, $3, $4, $5) ON CONFLICT (dedup_key) DO NOTHING RETURNING id",
        )
        .bind(Uuid::new_v4())
        .bind(&f.source_id)
        .bind(&f.kind)
        .bind(&dk)
        .bind({
            let mut p = payload;
            p["title"] = json!(title);
            p
        })
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;
        if r.is_some() {
            ins += 1;
        } else {
            dup += 1;
        }
    }
    Ok((ins, dup))
}

async fn mark_job(
    pool: &sqlx::PgPool,
    source_id: &str,
    label: &str,
    status: &str,
    err: Option<&str>,
    inserted: Option<i32>,
) {
    let _ = sqlx::query(
        "INSERT INTO intel_collection_jobs (source_id, schedule, status, last_run_at, last_error, last_inserted)
         VALUES ($1, $2, $3, now(), $4, $5)
         ON CONFLICT (source_id, schedule) DO UPDATE SET
           status = EXCLUDED.status, last_run_at = now(), last_error = EXCLUDED.last_error,
           last_inserted = COALESCE(EXCLUDED.last_inserted, intel_collection_jobs.last_inserted)",
    )
    .bind(source_id)
    .bind(format!("feed:{label}"))
    .bind(status)
    .bind(err)
    .bind(inserted)
    .execute(pool)
    .await;
}

/// Background loop started by main.rs when a database is configured.
pub async fn run_loop(pool: sqlx::PgPool, cfg: CollectorConfig) {
    let mut ticker = tokio::time::interval(Duration::from_secs(cfg.interval_secs.max(30)));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        let (ins, _dup, fails) = collect_once(&pool, &cfg).await;
        if ins > 0 {
            // new signals in: re-run the opportunity engine (WI §19)
            if let Ok((n, up)) = crate::intel::refresh_opportunities_db(&pool).await {
                tracing::info!(
                    new_items = ins,
                    opportunities = n,
                    upserted = up,
                    "opportunity engine refreshed"
                );
            }
        }
        if !fails.is_empty() {
            tracing::warn!(?fails, "some intel feeds failed this run");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RSS: &str = r#"<?xml version="1.0"?>
<rss version="2.0"><channel><title>t</title>
<item><title>AI agents break records</title><link>https://example.com/1</link><description>news &amp; more</description><pubDate>Wed, 20 Nov 2026 10:00:00 GMT</pubDate></item>
<item><title><![CDATA[LLM ops 101]]></title><link>https://example.com/2</link><description><![CDATA[<p>guide</p>]]></description></item>
</channel></rss>"#;

    const ATOM: &str = r#"<?xml version="1.0"?>
<feed xmlns="http://www.w3.org/2005/Atom"><title>dev.to</title>
<entry><title>Why agents matter</title><link href="https://dev.to/a"/><summary>essay</summary><published>2026-11-20T09:00:00Z</published></entry>
</feed>"#;

    #[test]
    fn rss_items_parsed() {
        let items = parse_rss(RSS);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].0, "AI agents break records");
        assert_eq!(items[0].1, "https://example.com/1");
        assert_eq!(items[0].2, "news & more");
        assert_eq!(items[1].0, "LLM ops 101");
    }

    #[test]
    fn atom_entries_parsed() {
        let items = parse_rss(ATOM);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].0, "Why agents matter");
        assert_eq!(items[0].1, "https://dev.to/a");
    }

    #[test]
    fn github_trending_normalized() {
        let body = r#"{"items":[{"full_name":"a/b","stargazers_count":5000,"created_at":"2026-11-01T00:00:00Z","html_url":"https://github.com/a/b","description":"agent kit"}]}"#;
        let t = parse_github_trending(body);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].0, "a/b");
        assert_eq!(t[0].1["volume"], 5000);
        assert!(t[0].1["velocity"].as_f64().unwrap() > 0.0);
    }

    #[test]
    fn searxng_normalized() {
        let body =
            r#"{"results":[{"title":"t1","url":"https://x","content":"c1","engine":"bing"}]}"#;
        let k = parse_searxng(body, "AI agents");
        assert_eq!(k.len(), 1);
        assert_eq!(k[0].1["query"], "AI agents");
    }

    #[test]
    fn hn_parsed() {
        let body = r#"{"hits":[{"title":"Show HN: agent","url":"https://x","points":321,"num_comments":88,"created_at":"2026-11-20T10:00:00Z"}]}"#;
        let items = parse_hn(body, "social_content");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].1["points"], 321);
        let t = parse_hn(body, "trend");
        assert_eq!(t[0].1["topic"], "Show HN: agent");
        assert!(t[0].1["velocity"].as_f64().unwrap() > 0.0);
    }

    #[test]
    fn devto_parsed() {
        let body = r#"[{"title":"t","url":"https://dev.to/a","description":"d","tag_list":["ai"],"published_at":"2026-11-20T00:00:00Z","user":{"name":"n"},"public_reactions_count":7}]"#;
        let items = parse_devto(body);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].1["reactions"], 7);
    }

    #[test]
    fn wiki_rc_parsed() {
        let body = r#"{"query":{"recentchanges":[{"title":"AI agents","type":"edit","timestamp":"2026-11-20T10:00:00Z","user":"u"}]}}"#;
        let items = parse_wiki_rc(body);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].1["platform"], "wikipedia");
    }

    #[test]
    fn dedup_key_stable_and_distinct() {
        let a = dedup_key("rsshub", "news", "u1", "t1");
        let b = dedup_key("rsshub", "news", "u1", "t1");
        let c = dedup_key("rsshub", "news", "u2", "t1");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!(a.starts_with("rsshub:news:"));
    }
}
