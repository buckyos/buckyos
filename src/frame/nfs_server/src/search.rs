//! `search` (NFSP §5.4): v1 implements only the `name` mode, but the response
//! structure is complete (`match_source`, `sources[]`) so adding modes later
//! does not change the shape (nfs_server.md §4.1).
//!
//! Implementation: pre-order DFS over the scoped subtree with byte-ordered
//! siblings, descending into each directory at its position in the sibling
//! order — so hits are emitted in path-component lexicographic order and the
//! cursor (the last hit's canonical path) makes pagination stable without any
//! server state. The walk is capped; hitting the cap surfaces as a degraded
//! source, never silently.

use crate::error::*;
use crate::fsutil::*;
use crate::namespace::{canonical_path, current_zone_cyfs_path, Node};
use crate::state::AppState;
use crate::types::{Locator, WantMask};
use serde_json::{json, Value};
use std::time::Instant;

const MAX_VISITED: usize = 200_000;

struct SearchCtx<'a> {
    needle: String,
    q: &'a str,
    limit: usize,
    want: &'a WantMask,
    /// components of the cursor path within the cursor's root
    cursor: Option<Vec<String>>,
    hits: Vec<Value>,
    visited: usize,
    capped: bool,
    truncated: bool,
}

impl AppState {
    pub fn op_search(&self, args: &Value, want: &WantMask) -> NfsResult<Value> {
        let q = args["q"].as_str().unwrap_or("").trim().to_string();
        if q.is_empty() {
            return Err(invalid("search requires q"));
        }
        let limit = args["limit"].as_u64().unwrap_or(50).clamp(1, 500) as usize;
        if let Some(modes) = args["modes"].as_array() {
            for m in modes {
                match m.as_str() {
                    Some("name") => {}
                    Some(other) => {
                        log::debug!("search mode '{}' not implemented in v1", other)
                    }
                    None => return Err(invalid("bad search mode")),
                }
            }
        }

        // Scope: an export root subtree, or all roots.
        let scope_roots: Vec<(String, String)> = match args.get("scope").filter(|s| !s.is_null()) {
            Some(scope) => {
                let loc: Locator = serde_json::from_value(scope.clone())
                    .map_err(|_| invalid("bad search scope"))?;
                match self.resolve_locator(&loc)? {
                    Node::Root => {
                        self.config.exports.iter().map(|r| (r.id.clone(), String::new())).collect()
                    }
                    Node::Native { root, rel, meta, .. } if meta.kind == "dir" => {
                        vec![(root, rel)]
                    }
                    _ => return Err(invalid("search scope must be a directory")),
                }
            }
            None => self.config.exports.iter().map(|r| (r.id.clone(), String::new())).collect(),
        };

        let cursor_parsed: Option<(usize, Vec<String>)> = match args["cursor"].as_str() {
            Some(cp) => Some(self.parse_search_cursor(cp)?),
            None => None,
        };

        let started = Instant::now();
        let mut ctx = SearchCtx {
            needle: q.to_lowercase(),
            q: &q,
            limit,
            want,
            cursor: None,
            hits: Vec::new(),
            visited: 0,
            capped: false,
            truncated: false,
        };

        for (root_id, base_rel) in &scope_roots {
            let cfg = match self.config.root(root_id) {
                Some(c) => c,
                None => continue,
            };
            let root_idx = self.root_index(root_id);
            ctx.cursor = match &cursor_parsed {
                Some((cur_root, _)) if root_idx < *cur_root => continue,
                Some((cur_root, parts)) if root_idx == *cur_root => Some(parts.clone()),
                _ => None,
            };
            self.search_dir(&mut ctx, cfg, root_id, base_rel);
            if ctx.truncated || ctx.capped {
                break;
            }
        }

        let next_cursor = if ctx.truncated {
            ctx.hits.last().and_then(|h| h["canonical_path"].as_str()).map(String::from)
        } else {
            None
        };
        let mut sources = vec![json!({
            "mode": "name",
            "state": if ctx.capped { "degraded" } else { "ok" },
            "took_ms": started.elapsed().as_millis() as u64,
        })];
        if ctx.capped {
            sources[0]["reason"] = json!("scan_capped");
        }
        let mut out = json!({
            "hits": ctx.hits,
            "partial": false,
            "sources": sources,
        });
        if let Some(c) = next_cursor {
            out["next_cursor"] = json!(c);
        }
        Ok(out)
    }

    /// Pre-order walk of one directory. Returns false to stop the whole search.
    fn search_dir(
        &self,
        ctx: &mut SearchCtx<'_>,
        cfg: &crate::config::ExportRoot,
        root_id: &str,
        rel: &str,
    ) -> bool {
        ctx.visited += 1;
        if ctx.visited > MAX_VISITED {
            ctx.capped = true;
            return false;
        }
        let full = join_root(&cfg.path, rel);
        let mut children: Vec<(String, NodeMeta)> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&full) {
            for dent in rd.flatten() {
                if let Ok(name) = dent.file_name().into_string() {
                    if let Ok(m) = lstat(&dent.path()) {
                        children.push((name, m));
                    }
                }
            }
        }
        children.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
        for (name, meta) in &children {
            let child_rel = rel_join(rel, name);
            let parts: Vec<&str> = child_rel.split('/').collect();
            let after_cursor = match &ctx.cursor {
                Some(cur) => {
                    let cur: Vec<&str> = cur.iter().map(|s| s.as_str()).collect();
                    parts > cur
                }
                None => true,
            };
            if after_cursor && name.to_lowercase().contains(&ctx.needle) {
                if ctx.hits.len() >= ctx.limit {
                    ctx.truncated = true;
                    return false;
                }
                let node = Node::Native {
                    root: root_id.to_string(),
                    rel: child_rel.clone(),
                    meta: meta.clone(),
                    anchored: None,
                };
                let mut hit = json!({
                    "ref": self.node_ref(&node),
                    "kind": meta.kind,
                    "name": name,
                    "canonical_path": canonical_path(root_id, &child_rel),
                    "match_source": "name",
                    "score": 1.0,
                    "explain": {
                        "matcher": "name.substring",
                        "evidence": format!("文件名包含 '{}'", ctx.q),
                    },
                });
                if ctx.want.has("base") {
                    hit["size"] = json!(meta.size);
                    hit["mtime"] = json!(meta.mtime);
                }
                ctx.hits.push(hit);
            }
            if meta.kind == "dir" {
                // Prune subtrees that lie entirely before the cursor: the dir's
                // components are less than the cursor's and not a prefix of it.
                let descend = match &ctx.cursor {
                    Some(cur) => {
                        let cur: Vec<&str> = cur.iter().map(|s| s.as_str()).collect();
                        let is_prefix = cur.len() >= parts.len() && cur[..parts.len()] == parts[..];
                        !(parts < cur && !is_prefix)
                    }
                    None => true,
                };
                if descend && !self.search_dir(ctx, cfg, root_id, &child_rel) {
                    return false;
                }
            }
        }
        true
    }

    fn root_index(&self, root_id: &str) -> usize {
        self.config.exports.iter().position(|r| r.id == root_id).unwrap_or(usize::MAX)
    }

    /// Parses a search cursor ("cyfs:///root/rel") into (root index, components).
    fn parse_search_cursor(&self, cursor: &str) -> NfsResult<(usize, Vec<String>)> {
        let rest = current_zone_cyfs_path(cursor)?
            .ok_or_else(|| invalid("bad search cursor"))?;
        let (root, rel) = rest.split_once('/').unwrap_or((&rest, ""));
        let idx = self.root_index(root);
        if idx == usize::MAX {
            return Err(invalid("bad search cursor: unknown root"));
        }
        Ok((idx, rel.split('/').map(String::from).collect()))
    }
}
