//! Spider (§5.6, §7.5): RSS/Atom and web pages become private captures — local candidate
//! objects without an entry, never in the stream. Selected link cards get an HTML snapshot
//! wrapped by a new local object (§18.1 step 4). Session credentials never enter objects.

use crate::error::{bad, HsError, HsResult};
use crate::ingress::add_candidate;
use crate::objects::{get_object, index_feed, put_object, StoredObject};
use crate::protocol::*;
use crate::{new_id, now_ms, now_s, Station};
use regex::Regex;
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::OnceLock;

const MAX_PAGE_BYTES: usize = 2 * 1024 * 1024;
const MAX_ITEMS_PER_CRAWL: usize = 20;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FeedItem {
    pub title: String,
    pub link: String,
    pub summary: String,
    pub author: Option<String>,
}

fn re(pattern: &'static str, cell: &'static OnceLock<Regex>) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("valid regex"))
}

pub fn decode_entities(s: &str) -> String {
    static NUM: OnceLock<Regex> = OnceLock::new();
    let s = s
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ");
    let s = re(r"&#(x?[0-9a-fA-F]+);", &NUM)
        .replace_all(&s, |caps: &regex::Captures| {
            let v = &caps[1];
            let code = if let Some(hex) = v.strip_prefix('x') { u32::from_str_radix(hex, 16).ok() } else { v.parse().ok() };
            code.and_then(char::from_u32).map(|c| c.to_string()).unwrap_or_default()
        })
        .to_string();
    s.replace("&amp;", "&")
}

fn strip_cdata(s: &str) -> &str {
    let t = s.trim();
    t.strip_prefix("<![CDATA[").and_then(|x| x.strip_suffix("]]>")).unwrap_or(t)
}

pub fn html_to_text(html: &str) -> String {
    static SCRIPT: OnceLock<Regex> = OnceLock::new();
    static TAG: OnceLock<Regex> = OnceLock::new();
    static WS: OnceLock<Regex> = OnceLock::new();
    let s = re(r"(?is)<(script|style|noscript)[^>]*>.*?</(script|style|noscript)>", &SCRIPT).replace_all(html, " ");
    let s = re(r"(?s)<[^>]*>", &TAG).replace_all(&s, " ");
    let s = decode_entities(&s);
    re(r"\s+", &WS).replace_all(&s, " ").trim().to_string()
}

fn tag_text(block: &str, names: &[&str]) -> Option<String> {
    for name in names {
        let pattern = format!(r"(?is)<{0}(?:\s[^>]*)?>(.*?)</{0}>", regex::escape(name));
        if let Ok(r) = Regex::new(&pattern) {
            if let Some(c) = r.captures(block) {
                let v = html_to_text(&decode_entities(strip_cdata(&c[1])));
                if !v.is_empty() {
                    return Some(v);
                }
            }
        }
    }
    None
}

fn truncate(s: &str, n: usize) -> String {
    let mut out: String = s.chars().take(n).collect();
    if s.chars().count() > n {
        out.push('…');
    }
    out
}

/// RSS 2.0 `<item>` and Atom `<entry>`; resolves relative links against `base`.
pub fn parse_feed(xml: &str, base: &str) -> Vec<FeedItem> {
    static ITEM: OnceLock<Regex> = OnceLock::new();
    static ATOM_LINK: OnceLock<Regex> = OnceLock::new();
    let items = re(r"(?is)<(item|entry)(?:\s[^>]*)?>(.*?)</(item|entry)>", &ITEM);
    let atom_link = re(r#"(?is)<link\b([^>]*)/?>"#, &ATOM_LINK);
    let base_url = url::Url::parse(base).ok();
    let mut out = Vec::new();
    for caps in items.captures_iter(xml) {
        let block = &caps[2];
        let title = tag_text(block, &["title"]).unwrap_or_default();
        let mut link = tag_text(block, &["link"]).unwrap_or_default();
        if link.is_empty() {
            for l in atom_link.captures_iter(block) {
                let attrs = &l[1];
                let rel_ok = !attrs.contains("rel=") || attrs.contains("rel=\"alternate\"") || attrs.contains("rel='alternate'");
                if let Some(href) = attr(attrs, "href") {
                    if rel_ok {
                        link = href;
                        break;
                    }
                }
            }
        }
        if let (Some(base), false) = (&base_url, link.is_empty()) {
            if let Ok(abs) = base.join(&link) {
                link = abs.to_string();
            }
        }
        if title.is_empty() || !is_http_url(&link) {
            continue;
        }
        let summary = tag_text(block, &["description", "summary", "content", "content:encoded"]).unwrap_or_default();
        let author = tag_text(block, &["dc:creator", "author"]).map(|a| truncate(&a, 80));
        out.push(FeedItem { title: truncate(&title, 120), link, summary: truncate(&summary, 280), author });
    }
    out
}

fn attr(attrs: &str, name: &str) -> Option<String> {
    let pattern = format!(r#"(?is)\b{}\s*=\s*["']([^"']*)["']"#, regex::escape(name));
    Regex::new(&pattern).ok()?.captures(attrs).map(|c| decode_entities(&c[1]))
}

pub fn is_feed_document(body: &str) -> bool {
    let head: String = body.chars().take(2048).collect::<String>().to_lowercase();
    head.contains("<rss") || head.contains("<feed") || head.contains("<rdf:rdf")
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct PagePreview {
    pub title: String,
    pub summary: String,
    pub feed_url: Option<String>,
    pub site_name: Option<String>,
}

pub fn preview_of_html(html: &str, base: &str) -> PagePreview {
    static META: OnceLock<Regex> = OnceLock::new();
    static LINK: OnceLock<Regex> = OnceLock::new();
    let mut preview = PagePreview::default();
    for m in re(r"(?is)<meta\b([^>]*)>", &META).captures_iter(html) {
        let attrs = &m[1];
        let key = attr(attrs, "property").or_else(|| attr(attrs, "name")).unwrap_or_default().to_lowercase();
        let Some(content) = attr(attrs, "content") else { continue };
        match key.as_str() {
            "og:title" | "twitter:title" if preview.title.is_empty() => preview.title = truncate(&content, 120),
            "og:description" | "description" | "twitter:description" if preview.summary.is_empty() => preview.summary = truncate(&content, 280),
            "og:site_name" => preview.site_name = Some(content),
            _ => {}
        }
    }
    if preview.title.is_empty() {
        preview.title = tag_text(html, &["title"]).map(|t| truncate(&t, 120)).unwrap_or_default();
    }
    for l in re(r"(?is)<link\b([^>]*)>", &LINK).captures_iter(html) {
        let attrs = &l[1];
        let t = attr(attrs, "type").unwrap_or_default().to_lowercase();
        if attr(attrs, "rel").is_some_and(|r| r.to_lowercase().contains("alternate")) && (t.contains("rss") || t.contains("atom")) {
            if let Some(href) = attr(attrs, "href") {
                preview.feed_url = url::Url::parse(base).ok().and_then(|b| b.join(&href).ok()).map(|u| u.to_string());
                break;
            }
        }
    }
    preview
}

impl Station {
    pub async fn fetch_text(&self, url: &str) -> HsResult<(String, String)> {
        if !is_http_url(url) {
            return Err(bad("only http(s) URLs can be fetched"));
        }
        let response = self.http.get(url).send().await.map_err(|e| HsError::Unavailable(format!("fetch {url}: {e}")))?;
        if !response.status().is_success() {
            return Err(HsError::Unavailable(format!("fetch {url}: HTTP {}", response.status())));
        }
        let content_type = response.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
        if response.content_length().unwrap_or(0) as usize > MAX_PAGE_BYTES {
            return Err(HsError::Unavailable("page too large".into()));
        }
        let bytes = response.bytes().await.map_err(|e| HsError::Unavailable(e.to_string()))?;
        if bytes.len() > MAX_PAGE_BYTES {
            return Err(HsError::Unavailable("page too large".into()));
        }
        Ok((String::from_utf8_lossy(&bytes).to_string(), content_type))
    }

    pub async fn link_preview(&self, url: &str) -> HsResult<PagePreview> {
        let (body, _) = self.fetch_text(url).await?;
        let mut preview = preview_of_html(&body, url);
        if preview.title.is_empty() {
            preview.title = url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_else(|| url.to_string());
        }
        Ok(preview)
    }

    /// Crawl an RSS or website source into private captures.
    pub async fn crawl_source(&self, source_id: &str) -> HsResult<usize> {
        let id = source_id.to_string();
        let (kind, url, name) = self
            .db
            .call(move |c| Ok(c.query_row("SELECT kind, url, name FROM sources WHERE id=?1", [id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, String>(2)?)))?))
            .await?;
        let url = url.ok_or_else(|| bad("source has no URL"))?;
        let (body, _) = self.fetch_text(&url).await?;
        let items = if is_feed_document(&body) {
            parse_feed(&body, &url)
        } else {
            let preview = preview_of_html(&body, &url);
            vec![FeedItem { title: if preview.title.is_empty() { url.clone() } else { preview.title }, link: url.clone(), summary: preview.summary, author: preview.site_name }]
        };
        let source_kind = if kind == "rss" { "rss" } else { "web" };
        let mut added = 0;
        for item in items.into_iter().take(MAX_ITEMS_PER_CRAWL) {
            if self.capture_link(&item, source_kind, source_id, &name, kind == "website").await? {
                added += 1;
            }
        }
        let id = source_id.to_string();
        let now = now_ms();
        self.db
            .call(move |c| {
                c.execute("UPDATE sources SET last_success_at=?2, last_fetch_at=?2, last_error=NULL WHERE id=?1", params![id, now])?;
                Ok(())
            })
            .await?;
        if added > 0 {
            self.bump(&["sources", "candidates"]);
        }
        Ok(added)
    }

    /// A link card capture (E06's precursor): local only, no entry. Websites are re-captured
    /// when their card changes; feed items once per URL.
    pub async fn capture_link(&self, item: &FeedItem, source_kind: &str, source_id: &str, label: &str, allow_update: bool) -> HsResult<bool> {
        let link = item.link.clone();
        let link2 = link.clone();
        let existing: Option<(String, String)> = self
            .db
            .call(move |c| {
                Ok(c.query_row(
                    "SELECT obj_id, body FROM objects WHERE local_only=1 AND obj_type=?1 AND json_extract(body, '$.source.original_url')=?2 ORDER BY received_at DESC LIMIT 1",
                    params![OBJ_TYPE_FEED, link2],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?)
            })
            .await?;
        if let Some((_, body)) = &existing {
            let same = serde_json::from_str::<FeedObject>(body)
                .ok()
                .and_then(|f| f.content)
                .is_some_and(|c| c.title.as_deref() == Some(item.title.as_str()) && c.summary.as_deref().unwrap_or("") == item.summary);
            if !allow_update || same {
                return Ok(false);
            }
        }
        let obj = FeedObject {
            kind: FeedKind::Post,
            comment_type: None,
            publisher: self.cfg.owner.clone(),
            iat: now_s(),
            nonce: Some(new_id("n")),
            entry: None,
            content: Some(FeedContent {
                content_type: ContentType::Link,
                text: None,
                title: Some(item.title.clone()),
                summary: (!item.summary.is_empty()).then(|| item.summary.clone()),
                cover: None,
                media: vec![],
            }),
            wraps: None,
            references: vec![],
            tags: vec![],
            source: Some(FeedSource { kind: source_kind.into(), original_url: link, original_author: item.author.clone(), captured_at_ms: now_ms() as u64 }),
            link: Some(item.link.clone()),
            publication_category: None,
            base_on: None,
        };
        validate_feed_object(&obj).map_err(bad)?;
        self.store_capture(obj, json!({ "transport": "pull", "subscription_id": source_id, "label": label }), existing.map(|e| e.0)).await?;
        Ok(true)
    }

    async fn store_capture(&self, obj: FeedObject, path: Value, replaces: Option<String>) -> HsResult<String> {
        let body = obj.to_value();
        let (obj_id, _) = obj_id_of(OBJ_TYPE_FEED, &body).map_err(bad)?;
        let stored = StoredObject {
            obj_id: obj_id.clone(),
            obj_type: OBJ_TYPE_FEED.into(),
            body,
            jwt: None,
            signer: None,
            publisher: Some(self.cfg.owner.clone()),
            verified: true,
            local_only: true,
        };
        let owner = self.cfg.owner.clone();
        let now = now_ms();
        let id = obj_id.clone();
        self.db
            .call(move |c| {
                let tx = c.transaction()?;
                put_object(&tx, &stored, Some("spider"), now)?;
                index_feed(&tx, &id, &obj, false)?;
                add_candidate(&tx, &id, &owner, &path, true, now)?;
                if let Some(old) = replaces {
                    tx.execute("UPDATE candidates SET selection='dropped', reason='replaced', replaced_by=?2 WHERE obj_id=?1 AND selection!='admitted'", params![old, id])?;
                }
                tx.commit()?;
                Ok(())
            })
            .await?;
        self.wake.select.notify_one();
        Ok(obj_id)
    }

    /// Selected link-card captures get a snapshot: the page stored as a FileObject, wrapped by
    /// a new local object that replaces the card (§18.1). Returns the object to admit.
    pub async fn maybe_snapshot(&self, obj_id: &str) -> HsResult<String> {
        let id = obj_id.to_string();
        let stored = self.db.call(move |c| get_object(c, &id)).await?;
        let Some(stored) = stored else { return Ok(obj_id.to_string()) };
        if !stored.local_only || !self.cfg.spider_enabled {
            return Ok(obj_id.to_string());
        }
        let feed: FeedObject = serde_json::from_value(stored.body.clone())?;
        let (Some(link), None, Some(source)) = (&feed.link, &feed.wraps, &feed.source) else { return Ok(obj_id.to_string()) };
        let Ok((html, content_type)) = self.fetch_text(link).await else { return Ok(obj_id.to_string()) };
        if !content_type.contains("html") && !html.trim_start().starts_with('<') {
            return Ok(obj_id.to_string());
        }
        let snapshot = self.store_text_file("snapshot.html", "text/html", html.as_bytes()).await?;
        let mut wrapped = feed.clone();
        wrapped.nonce = Some(new_id("n"));
        wrapped.iat = now_s();
        wrapped.wraps = Some(snapshot);
        if let Some(content) = wrapped.content.as_mut() {
            content.content_type = ContentType::Article;
        }
        wrapped.source = Some(FeedSource { captured_at_ms: now_ms() as u64, ..source.clone() });
        let id = obj_id.to_string();
        let paths: String = self.db.call(move |c| Ok(c.query_row("SELECT source_paths FROM candidates WHERE obj_id=?1", [id], |r| r.get(0))?)).await?;
        let path = serde_json::from_str::<Vec<Value>>(&paths).ok().and_then(|p| p.into_iter().next()).unwrap_or(json!({ "transport": "pull" }));
        let new_id_ = self.store_capture(wrapped, path, Some(obj_id.to_string())).await?;
        let id = new_id_.clone();
        self.db.call(move |c| Ok(c.execute("UPDATE candidates SET selection='preparing' WHERE obj_id=?1", [id])?)).await?;
        Ok(new_id_)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rss_and_atom() {
        let rss = r#"<?xml version="1.0"?><rss><channel><title>T</title>
            <item><title><![CDATA[Hello &amp; welcome]]></title><link>https://example.org/a</link>
            <description>&lt;p&gt;First &amp;amp; best&lt;/p&gt;</description><dc:creator>Ann</dc:creator></item>
            <item><title>No link</title></item></channel></rss>"#;
        let items = parse_feed(rss, "https://example.org/feed.xml");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Hello & welcome");
        assert_eq!(items[0].link, "https://example.org/a");
        assert_eq!(items[0].summary, "First & best");
        assert_eq!(items[0].author.as_deref(), Some("Ann"));
        let atom = r#"<feed xmlns="http://www.w3.org/2005/Atom"><entry><title>A</title>
            <link rel="alternate" href="/posts/1"/><summary>S</summary></entry></feed>"#;
        let items = parse_feed(atom, "https://blog.example/atom.xml");
        assert_eq!(items[0].link, "https://blog.example/posts/1");
        assert!(is_feed_document(atom));
    }

    #[test]
    fn previews_html() {
        let html = r#"<html><head><title>Fallback</title><meta property="og:title" content="Garden guide">
            <meta name="description" content="How to start"><link rel="alternate" type="application/rss+xml" href="/rss"></head>
            <body><script>var x = "<b>";</script><p>Body &amp; more</p></body></html>"#;
        let p = preview_of_html(html, "https://example.org/garden");
        assert_eq!(p.title, "Garden guide");
        assert_eq!(p.summary, "How to start");
        assert_eq!(p.feed_url.as_deref(), Some("https://example.org/rss"));
        assert_eq!(html_to_text(html), "Fallback Body & more");
    }
}
