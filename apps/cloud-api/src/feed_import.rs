//! 清单页导入: 把 awesome-rss 这类"源列表"页面 (GitHub markdown / HTML) 里的
//! RSS feed URL 抽取出来, 注册成自定义信源 (intel_custom_feeds), 由 rss-custom
//! 连接器统一抓取。红线不变: 只抓公开只读页面与 feed, 礼貌 UA, 不绕限流。

use axum::{
    Json,
    extract::{Json as ExtractJson, State},
};
use serde_json::{Value, json};
use sha2::Digest;
use sqlx::Row;

use crate::state::AppState;

fn err(e: impl ToString) -> axum::response::Response {
    crate::api::app_error_response(sincy_common::AppError::retryable("bad", e.to_string()))
}

/// 从列表页文本中抽取疑似 feed 的 URL (markdown 链接 / 行内反引号 URL / HTML href)
pub fn extract_feed_urls(text: &str) -> Vec<String> {
    let url_re = regex::Regex::new(r#"https?://[^\s\)\]\`<>"']+"#).unwrap();
    let feedish = regex::Regex::new(r"(?i)(feed|atom|rss|\.xml)").unwrap();
    let mut out: Vec<String> = Vec::new();
    for m in url_re.find_iter(text) {
        let mut u = m
            .as_str()
            .trim_end_matches(['.', ',', ';', '!', '?'])
            .to_string();
        // 去掉 markdown/html 尾巴
        while let Some(stripped) = u.strip_suffix(')') {
            u = stripped.to_string();
        }
        if feedish.is_match(&u) && !out.contains(&u) {
            out.push(u);
        }
    }
    out.truncate(500);
    out
}

/// 由 feed URL 生成稳定的 source id: host + url 短哈希 (幂等)
fn feed_id(url: &str) -> String {
    let host = url
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or("feed")
        .replace(['.', ':'], "-");
    let digest = sha2::Sha256::digest(url.as_bytes());
    let short: String = digest.iter().take(4).map(|b| format!("{b:02x}")).collect();
    format!("feed-{host}-{short}")
}

/// GET /intel/feeds — 当前全部自定义 RSS 源
pub async fn list_feeds(
    State(state): State<AppState>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(err)?;
    let rows = sqlx::query(
        "SELECT f.id, f.label, f.url, f.origin, f.enabled, f.created_at,
                (SELECT count(*)::int8 FROM intel_items i WHERE i.source_id = f.id) AS items_total
         FROM intel_custom_feeds f ORDER BY f.enabled DESC, f.id",
    )
    .fetch_all(pool)
    .await
    .map_err(err)?;
    let feeds: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<String, _>("id"),
                "label": r.get::<String, _>("label"),
                "url": r.get::<String, _>("url"),
                "origin": r.get::<Option<String>, _>("origin"),
                "enabled": r.get::<bool, _>("enabled"),
                "created_at": r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
                "items_total": r.get::<i64, _>("items_total"),
            })
        })
        .collect();
    Ok(ExtractJson(json!({ "feeds": feeds })))
}

/// POST /intel/feeds/import — 抓取一个清单页 (如 awesome-rss-feeds 的 LIST.md),
/// 抽取 feed URL; apply=true 时注册为自定义信源 (幂等, 不重复添加)。
pub async fn import_feed_list(
    State(state): State<AppState>,
    ExtractJson(body): ExtractJson<Value>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(err)?;
    let url = body.get("url").and_then(|v| v.as_str()).ok_or_else(|| {
        crate::api::app_error_response(sincy_common::AppError::retryable(
            "bad_request",
            "missing url",
        ))
    })?;
    let apply = body.get("apply").and_then(|v| v.as_bool()).unwrap_or(false);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("SincyIntelBot/1.0 (+read-only public feed lists; contact: admin@sincy.dev)")
        .build()
        .map_err(|e| {
            crate::api::app_error_response(sincy_common::AppError::retryable("http", e.to_string()))
        })?;
    let resp = client.get(url).send().await.map_err(|e| {
        crate::api::app_error_response(sincy_common::AppError::retryable("http", e.to_string()))
    })?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| {
        crate::api::app_error_response(sincy_common::AppError::retryable("http", e.to_string()))
    })?;
    if !status.is_success() {
        return Err(crate::api::app_error_response(
            sincy_common::AppError::retryable("http", format!("list page HTTP {status}")),
        ));
    }

    let found = extract_feed_urls(&text);
    let mut added = 0usize;
    let mut skipped = 0usize;
    if apply {
        for u in &found {
            let id = feed_id(u);
            let host = u
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .split('/')
                .next()
                .unwrap_or("feed")
                .to_string();
            let _r = sqlx::query(
                "INSERT INTO intelligence_sources (id, name, category, access_method, capabilities, platforms, terms, enabled)
                 VALUES ($1, $2, 'rss', 'RSSSource', '[\"news_research\"]', '[]', $3, true)
                 ON CONFLICT (id) DO NOTHING",
            )
            .bind(&id)
            .bind(&host)
            .bind(format!("imported from {url}"))
            .execute(pool)
            .await
            .map_err(err)?;
            let r2 = sqlx::query(
                "INSERT INTO intel_custom_feeds (id, label, url, origin, enabled) VALUES ($1, $2, $3, $4, true)
                 ON CONFLICT (id) DO NOTHING",
            )
            .bind(&id)
            .bind(&host)
            .bind(u)
            .bind(url)
            .execute(pool)
            .await
            .map_err(err)?;
            if r2.rows_affected() > 0 {
                added += 1;
            } else {
                skipped += 1;
            }
        }
    }
    Ok(ExtractJson(json!({
        "list_url": url,
        "apply": apply,
        "found": found.len(),
        "added": added,
        "skipped_existing": skipped,
        "feeds": found,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_markdown_links_and_inline_urls() {
        let md = r#"# list
- [Kaichao You](https://youkaichao.github.io/feed.xml) — `https://youkaichao.github.io/feed.xml`
- [HN](https://news.ycombinator.com/rss) — `https://news.ycombinator.com/rss`
- [repo](https://github.com/foo/bar) — not a feed
- plain https://example.com/atom.xml in text
"#;
        let urls = extract_feed_urls(md);
        assert!(urls.contains(&"https://youkaichao.github.io/feed.xml".to_string()));
        assert!(urls.contains(&"https://news.ycombinator.com/rss".to_string()));
        assert!(urls.contains(&"https://example.com/atom.xml".to_string()));
        assert!(!urls.iter().any(|u| u.contains("github.com/foo/bar")));
        // 去重
        let n = urls.len();
        let dedup: std::collections::HashSet<_> = urls.iter().collect();
        assert_eq!(n, dedup.len());
    }

    #[test]
    fn feed_id_stable_and_unique() {
        let a = feed_id("https://a.com/feed.xml");
        let b = feed_id("https://a.com/feed.xml");
        let c = feed_id("https://a.com/other.xml");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!(a.starts_with("feed-a-com-"));
    }
}
