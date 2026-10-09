//! URL-query data sources (design doc §2.4, §3.4.11): large tables addressed by
//! "source URL + query parameters" instead of content ObjectIds. Phase one is
//! read-only and only talks to *registered* adapters: a URL in a document can
//! never make the service issue an arbitrary network request.

use aiworkspace_core::canonical::{canonical_json, sha256_hex};
use aiworkspace_core::{Code, WsError, WsResult};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Default)]
pub struct SourceQuery {
    /// View-level filter combined with the base query; must be pushed down or refused.
    pub filter: Option<Value>,
    pub sorts: Vec<Value>,
    pub fields: Option<Vec<String>>,
    pub limit: usize,
    pub cursor: Option<String>,
    /// The caller needs one consistent source view across pages.
    pub require_snapshot: bool,
    pub snapshot_token: Option<String>,
}

pub trait SourceAdapter: Send + Sync {
    /// What this source can actually do. Nothing is assumed from the URL being resolvable.
    fn capabilities(&self, reference: &Value) -> WsResult<Value>;
    /// One page. `scope` is the trusted authorization scope the read runs under.
    fn query(&self, reference: &Value, q: &SourceQuery, scope: &str) -> WsResult<Value>;
}

#[derive(Default)]
pub struct SourceRegistry {
    adapters: HashMap<String, Arc<dyn SourceAdapter>>,
    cache: Mutex<HashMap<String, Value>>,
    pub cache_hits: AtomicU64,
}

impl SourceRegistry {
    pub fn register(&mut self, scheme: &str, adapter: Arc<dyn SourceAdapter>) {
        self.adapters.insert(scheme.to_string(), adapter);
    }

    fn adapter(&self, reference: &Value) -> WsResult<&Arc<dyn SourceAdapter>> {
        let url = reference["source_url"].as_str().unwrap_or("");
        let scheme = url.split("://").next().unwrap_or("");
        self.adapters
            .get(scheme)
            .ok_or_else(|| WsError::new(Code::DependencyUnavailable, format!("no source adapter is registered for {scheme}://")))
    }

    pub fn capabilities(&self, reference: &Value) -> WsResult<Value> {
        self.adapter(reference)?.capabilities(reference)
    }

    /// Query through the adapter. The cache key covers source, the *whole*
    /// query, the authorization scope and the pinned source version — never
    /// just the base URL.
    pub fn query(&self, reference: &Value, q: &SourceQuery, scope: &str) -> WsResult<Value> {
        let adapter = self.adapter(reference)?;
        let caps = adapter.capabilities(reference)?;
        if q.require_snapshot && caps["consistent_paging"] != json!(true) {
            return Err(WsError::new(Code::DependencyUnavailable, "the source cannot provide a consistent snapshot")
                .with_data(json!({ "capabilities": caps })));
        }
        let key = sha256_hex(
            canonical_json(&json!({ "ref": reference, "filter": q.filter, "sorts": q.sorts, "fields": q.fields, "limit": q.limit,
                                    "cursor": q.cursor, "snapshot": q.snapshot_token, "scope": scope }))?
            .as_bytes(),
        );
        // only pages pinned to a source snapshot are safe to reuse
        let cacheable = q.snapshot_token.is_some();
        if cacheable {
            if let Some(hit) = self.cache.lock().unwrap().get(&key) {
                self.cache_hits.fetch_add(1, Ordering::Relaxed);
                let mut v = hit.clone();
                v["from_cache"] = json!(true);
                return Ok(v);
            }
        }
        let mut page = adapter.query(reference, q, scope)?;
        page["capabilities"] = caps;
        if cacheable {
            self.cache.lock().unwrap().insert(key, page.clone());
        }
        Ok(page)
    }
}

/// Deterministic generated big table for tests and demos:
/// `fixture://events?rows=1000000&row_key=1&snapshot=1&deny=bob`.
/// Rows are produced page by page; `rows_generated` proves nothing fetched the whole set.
#[derive(Default)]
pub struct GeneratedSource {
    pub rows_generated: AtomicU64,
    pub queries: AtomicU64,
    pub offline: std::sync::atomic::AtomicBool,
}

fn url_params(url: &str) -> HashMap<String, String> {
    url.split_once('?')
        .map(|(_, q)| q.split('&').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect())
        .unwrap_or_default()
}

impl GeneratedSource {
    fn row(i: u64, with_key: bool) -> Value {
        let mut r = json!({ "values": {
            "event_id": format!("ev-{i:09}"),
            "created_at": aiworkspace_core::value::format_utc_ms(1_750_000_000_000 + i as i64 * 60_000),
            "amount": format!("{}.{:02}", i % 1000, i % 100),
            "project_id": format!("project-{}", i % 50),
        } });
        if with_key {
            r["record_id"] = json!(format!("ev-{i:09}"));
        }
        r
    }
}

impl SourceAdapter for GeneratedSource {
    fn capabilities(&self, reference: &Value) -> WsResult<Value> {
        let p = url_params(reference["source_url"].as_str().unwrap_or(""));
        let snapshot = p.get("snapshot").map(String::as_str) == Some("1");
        Ok(json!({
            "fixed_version": snapshot, "consistent_paging": snapshot,
            "stable_row_key": p.get("row_key").map(String::as_str) != Some("0"),
            "offline_slice": false, "write": false,
            "filters": [{ "field_id": "project_id", "operators": ["eq"] }],
            "sorts": [{ "field_id": "event_id", "directions": ["asc"] }],
        }))
    }

    fn query(&self, reference: &Value, q: &SourceQuery, scope: &str) -> WsResult<Value> {
        self.queries.fetch_add(1, Ordering::Relaxed);
        if self.offline.load(Ordering::Relaxed) {
            return Err(WsError::new(Code::DependencyUnavailable, "the data source is unreachable"));
        }
        let p = url_params(reference["source_url"].as_str().unwrap_or(""));
        if p.get("deny").is_some_and(|d| d.split(',').any(|x| x == scope)) {
            return Err(WsError::denied("the data source refused this principal"));
        }
        let rows: u64 = p.get("rows").and_then(|r| r.parse().ok()).unwrap_or(1000);
        let with_key = p.get("row_key").map(String::as_str) != Some("0");
        let snapshot = p.get("snapshot").map(String::as_str) == Some("1");
        // base query and view filter combine; anything this source cannot push down is refused, not ignored
        let mut project: Option<String> = None;
        for f in [reference["query"].get("filter"), q.filter.as_ref()].into_iter().flatten().filter(|f| !f.is_null()) {
            let ok = f["field_id"] == json!("project_id") && (f["operator"] == json!("eq")) && f["value"].is_string();
            if !ok {
                return Err(WsError::invalid_op("the data source cannot evaluate this filter").with_data(json!({ "filter": f })));
            }
            if project.as_deref().is_some_and(|p| Some(p) != f["value"].as_str()) {
                return Ok(json!({ "rows": [], "consistency": if snapshot { "snapshot" } else { "best_effort" } }));
            }
            project = f["value"].as_str().map(str::to_string);
        }
        for s in &q.sorts {
            if s["field_id"] != json!("event_id") || s.get("direction").is_some_and(|d| d != "asc") {
                return Err(WsError::invalid_op("the data source cannot sort this way"));
            }
        }
        let token = snapshot.then(|| format!("snap-{rows}"));
        if let (Some(want), Some(have)) = (&q.snapshot_token, &token) {
            if want != have {
                return Err(WsError::new(Code::SnapshotExpired, "source snapshot is no longer available"));
            }
        }
        let start: u64 = match &q.cursor {
            Some(c) => c.parse().map_err(|_| WsError::invalid_op("bad cursor"))?,
            None => 0,
        };
        let (mut out, mut i) = (Vec::new(), start);
        let limit = q.limit.clamp(1, 1000);
        while i < rows && out.len() < limit {
            let matches = project.as_ref().map_or(true, |p| *p == format!("project-{}", i % 50));
            if matches {
                self.rows_generated.fetch_add(1, Ordering::Relaxed);
                let mut r = Self::row(i, with_key);
                if let Some(fields) = q.fields.as_ref().or(reference["query"]["fields"].as_array().map(|a| {
                    a.iter().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>()
                }).as_ref()) {
                    r["values"].as_object_mut().unwrap().retain(|k, _| fields.contains(k));
                }
                out.push(r);
            }
            i += 1;
        }
        let mut page = json!({ "rows": out, "consistency": if snapshot { "snapshot" } else { "best_effort" } });
        if i < rows {
            page["next_cursor"] = json!(i.to_string());
        }
        if let Some(t) = token {
            page["source_revision"] = json!(t);
        }
        Ok(page)
    }
}
