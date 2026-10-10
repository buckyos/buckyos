//! Derived state: canonical JSON files, graph snapshots, path indexes and
//! the SQLite query cache. Everything here can be deleted and rebuilt from
//! the log (M-24); each piece is rebuilt aside and swapped in, so a reader
//! never sees a half-built index.

use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
use rusqlite::{params, Connection, OpenFlags};
use serde::Serialize;
use serde_json::Value;

use super::model::*;
use super::state::GraphView;
use super::text::{fts_tokens, normalize_alias};
use super::Result;

pub const GRAPH_DIR: &str = "graph";
pub const OCCASION_DIR: &str = "occasion";
pub const OBJECT_DIR: &str = "object";
pub const OBSERVATION_DIR: &str = "observation";
pub const ITEM_DIR: &str = "item";
pub const INDEX_DIR: &str = "index";
pub const SQLITE_FILE: &str = "memory.sqlite";

const PERCENT_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'!')
    .add(b'"')
    .add(b'#')
    .add(b'$')
    .add(b'%')
    .add(b'&')
    .add(b'\'')
    .add(b'(')
    .add(b')')
    .add(b'*')
    .add(b'+')
    .add(b',')
    .add(b'/')
    .add(b':')
    .add(b';')
    .add(b'<')
    .add(b'=')
    .add(b'>')
    .add(b'?')
    .add(b'@')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}')
    .add(0x7f);

/// Every path segment derived from data is percent-encoded (TD-11); `.` and
/// `..` are encoded too so no segment can leave its directory.
pub fn seg(s: &str) -> String {
    match s {
        "." => "%2E".to_string(),
        ".." => "%2E%2E".to_string(),
        "" => "%00".to_string(),
        _ => utf8_percent_encode(s, PERCENT_SET).to_string(),
    }
}

pub fn random_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{}-{nanos}", std::process::id())
}

#[cfg(unix)]
pub fn sync_dir(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
pub fn sync_dir(_path: &Path) -> Result<()> {
    Ok(())
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        super::AgentMemoryError::Invalid(format!("no parent: {}", path.display()))
    })?;
    fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(
        ".{}.tmp.{}",
        path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default(),
        random_suffix()
    ));
    {
        let mut f = OpenOptions::new().create_new(true).write(true).open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    sync_dir(parent)
}

fn write_jsonl<'a, T: Serialize + 'a>(
    path: PathBuf,
    values: impl Iterator<Item = &'a T>,
) -> Result<()> {
    let mut buf = Vec::new();
    for v in values {
        serde_json::to_writer(&mut buf, v)?;
        buf.push(b'\n');
    }
    atomic_write(&path, &buf)
}

fn touch(path: &Path) -> Result<()> {
    if let Some(p) = path.parent() {
        fs::create_dir_all(p)?;
    }
    OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)?;
    Ok(())
}

pub fn canonical_path(root: &Path, dir: &str, id: &str) -> PathBuf {
    root.join(dir).join(seg(id))
}

/// Rebuild every derived artifact from `view`.
pub fn materialize(root: &Path, view: &GraphView) -> Result<()> {
    write_canonical(root, view)?;
    let graph = root.join(GRAPH_DIR);
    fs::create_dir_all(&graph)?;
    write_jsonl(graph.join("objects.jsonl"), view.objects())?;
    write_jsonl(graph.join("observations.jsonl"), view.observations())?;
    write_jsonl(graph.join("items.jsonl"), view.items())?;
    rebuild_path_indexes(root, view)?;
    rebuild_sqlite(root, view)
}

fn write_canonical(root: &Path, view: &GraphView) -> Result<()> {
    let mut expected = HashSet::new();
    let mut put = |dir: &str, id: &str, bytes: Vec<u8>| -> Result<()> {
        let p = canonical_path(root, dir, id);
        if fs::read(&p).ok().as_deref() != Some(bytes.as_slice()) {
            atomic_write(&p, &bytes)?;
        }
        expected.insert(p);
        Ok(())
    };
    for o in view.occasions() {
        put(OCCASION_DIR, &o.occasion_id, serde_json::to_vec_pretty(o)?)?;
    }
    for o in view.objects() {
        put(OBJECT_DIR, &o.object_id, serde_json::to_vec_pretty(o)?)?;
    }
    for o in view.observations() {
        put(
            OBSERVATION_DIR,
            &o.observation_id,
            serde_json::to_vec_pretty(o)?,
        )?;
    }
    for i in view.items() {
        put(ITEM_DIR, &i.item_id, serde_json::to_vec_pretty(i)?)?;
    }
    for dir in [OCCASION_DIR, OBJECT_DIR, OBSERVATION_DIR, ITEM_DIR] {
        if let Ok(rd) = fs::read_dir(root.join(dir)) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_file()
                    && !expected.contains(&p)
                    && !p
                        .file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with('.'))
                {
                    let _ = fs::remove_file(&p);
                }
            }
        }
    }
    Ok(())
}

fn rebuild_path_indexes(root: &Path, view: &GraphView) -> Result<()> {
    let tmp = root.join(format!(".{INDEX_DIR}.tmp.{}", random_suffix()));
    fs::create_dir_all(&tmp)?;
    for o in view.objects() {
        touch(
            &tmp.join("by_kind")
                .join(seg(o.kind.as_str()))
                .join(seg(&o.object_id)),
        )?;
        for a in &o.aliases {
            if a.status == AliasStatus::Active {
                touch(
                    &tmp.join("by_alias")
                        .join(seg(&normalize_alias(&a.alias)))
                        .join(seg(&o.object_id)),
                )?;
            }
        }
    }
    for obs in view.observations() {
        if obs.status != ObservationStatus::Active {
            continue;
        }
        for e in &obs.entities {
            touch(
                &tmp.join("by_entity")
                    .join(seg(e))
                    .join("obs")
                    .join(seg(&obs.observation_id)),
            )?;
        }
    }
    for item in view.items() {
        if !item.status.recallable() || item.kind == ItemKind::Salience {
            continue;
        }
        for e in &item.entities {
            touch(
                &tmp.join("by_entity")
                    .join(seg(e))
                    .join("item")
                    .join(seg(&item.item_id)),
            )?;
        }
        touch(
            &tmp.join("by_kind")
                .join(seg(item.kind.as_str()))
                .join(seg(&item.item_id)),
        )?;
        if item.kind == ItemKind::Relation {
            let g = |k: &str| {
                item.claim
                    .get(k)
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string()
            };
            let (s, p, o) = (g("subject"), g("predicate"), g("object"));
            let pair = if s <= o {
                format!("{s}__{o}")
            } else {
                format!("{o}__{s}")
            };
            touch(
                &tmp.join("by_pair")
                    .join(seg(&pair))
                    .join(seg(&item.item_id)),
            )?;
            touch(
                &tmp.join("by_predicate")
                    .join(seg(&p))
                    .join(seg(&item.item_id)),
            )?;
            touch(
                &tmp.join("by_relation")
                    .join(seg(&s))
                    .join(seg(&p))
                    .join(seg(&o))
                    .join(seg(&item.item_id)),
            )?;
        }
    }
    let live = root.join(INDEX_DIR);
    let old = root.join(format!(".{INDEX_DIR}.old.{}", random_suffix()));
    if live.exists() {
        fs::rename(&live, &old)?;
    }
    fs::rename(&tmp, &live)?;
    let _ = fs::remove_dir_all(&old);
    Ok(())
}

const SCHEMA_SQL: &str = "
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE objects (object_id TEXT PRIMARY KEY, kind TEXT NOT NULL, canonical_name TEXT NOT NULL,
  weight REAL NOT NULL, confidence REAL NOT NULL, status TEXT NOT NULL, merged_into TEXT);
CREATE TABLE aliases (alias_norm TEXT NOT NULL, object_id TEXT NOT NULL, alias_type TEXT NOT NULL,
  status TEXT NOT NULL, PRIMARY KEY(alias_norm, object_id));
CREATE TABLE observations (observation_id TEXT PRIMARY KEY, kind TEXT NOT NULL, content TEXT NOT NULL,
  status TEXT NOT NULL, event_ref TEXT);
CREATE TABLE items (item_id TEXT PRIMARY KEY, revision INTEGER NOT NULL, kind TEXT NOT NULL,
  claim_json TEXT NOT NULL, scope_json TEXT, basis TEXT, explicit INTEGER NOT NULL,
  weight REAL NOT NULL, confidence REAL NOT NULL, status TEXT NOT NULL);
CREATE TABLE item_entities (item_id TEXT NOT NULL, object_id TEXT NOT NULL, PRIMARY KEY(item_id, object_id));
CREATE TABLE item_evidence (item_id TEXT NOT NULL, observation_id TEXT NOT NULL, PRIMARY KEY(item_id, observation_id));
CREATE TABLE dispositions (perception_ref TEXT PRIMARY KEY, outcome TEXT NOT NULL, occasion_id TEXT NOT NULL);
CREATE INDEX idx_aliases_alias ON aliases(alias_norm);
CREATE INDEX idx_item_entities_object ON item_entities(object_id);
CREATE VIRTUAL TABLE memory_fts USING fts5(ref_id UNINDEXED, ref_type UNINDEXED, tokens,
  tokenize = 'unicode61 remove_diacritics 2');
";

fn rebuild_sqlite(root: &Path, view: &GraphView) -> Result<()> {
    let tmp = root.join(format!(".{SQLITE_FILE}.tmp.{}", random_suffix()));
    {
        let conn = Connection::open(&tmp)?;
        conn.execute_batch(SCHEMA_SQL)?;
        let tx = conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO meta(key, value) VALUES('graph_seq', ?)",
            params![view.seq().to_string()],
        )?;
        for o in view.objects() {
            tx.execute(
                "INSERT INTO objects VALUES(?, ?, ?, ?, ?, ?, ?)",
                params![
                    o.object_id,
                    o.kind.as_str(),
                    o.canonical_name,
                    o.weight,
                    o.confidence,
                    o.status.as_str(),
                    o.merged_into
                ],
            )?;
            for a in &o.aliases {
                tx.execute(
                    "INSERT OR REPLACE INTO aliases VALUES(?, ?, ?, ?)",
                    params![
                        normalize_alias(&a.alias),
                        o.object_id,
                        a.alias_type.as_str(),
                        a.status.as_str()
                    ],
                )?;
            }
        }
        for obs in view.observations() {
            tx.execute(
                "INSERT INTO observations VALUES(?, ?, ?, ?, ?)",
                params![
                    obs.observation_id,
                    obs.kind.as_str(),
                    obs.content,
                    obs.status.as_str(),
                    obs.source_ref.as_ref().and_then(|s| s.event_ref.clone())
                ],
            )?;
        }
        for i in view.items() {
            tx.execute(
                "INSERT INTO items VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    i.item_id,
                    i.revision as i64,
                    i.kind.as_str(),
                    serde_json::to_string(&i.claim)?,
                    i.scope.as_ref().map(serde_json::to_string).transpose()?,
                    i.basis.map(|b| b.as_str()),
                    i.explicit as i64,
                    i.weight,
                    i.confidence,
                    i.status.as_str()
                ],
            )?;
            for e in &i.entities {
                tx.execute(
                    "INSERT OR IGNORE INTO item_entities VALUES(?, ?)",
                    params![i.item_id, e],
                )?;
            }
            for e in &i.evidence {
                tx.execute(
                    "INSERT OR IGNORE INTO item_evidence VALUES(?, ?)",
                    params![i.item_id, e],
                )?;
            }
        }
        for (r, d) in view.dispositions() {
            tx.execute(
                "INSERT INTO dispositions VALUES(?, ?, ?)",
                params![r, d.disposition.outcome.as_str(), d.occasion_id],
            )?;
        }
        for (id, ty, tokens) in view.fts_documents() {
            tx.execute(
                "INSERT INTO memory_fts VALUES(?, ?, ?)",
                params![id, ty, tokens],
            )?;
        }
        tx.commit()?;
    }
    File::open(&tmp)?.sync_all()?;
    fs::rename(&tmp, root.join(SQLITE_FILE))?;
    sync_dir(root)
}

/// `graph_seq` the SQLite cache was built at (None when absent/unreadable).
pub fn sqlite_seq(root: &Path) -> Option<u64> {
    let conn =
        Connection::open_with_flags(root.join(SQLITE_FILE), OpenFlags::SQLITE_OPEN_READ_ONLY)
            .ok()?;
    conn.query_row("SELECT value FROM meta WHERE key = 'graph_seq'", [], |r| {
        r.get::<_, String>(0)
    })
    .ok()?
    .parse()
    .ok()
}

/// Refs whose full text shares a token with the query, through FTS5. Only
/// used when the cache matches `graph_seq`; otherwise the caller scans.
pub fn fts_lookup(
    root: &Path,
    graph_seq: u64,
    text: &str,
) -> std::result::Result<HashSet<String>, String> {
    let tokens = fts_tokens(text);
    if tokens.is_empty() {
        return Ok(HashSet::new());
    }
    let conn =
        Connection::open_with_flags(root.join(SQLITE_FILE), OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| format!("sqlite unavailable: {e}"))?;
    let seq: Option<u64> = conn
        .query_row("SELECT value FROM meta WHERE key = 'graph_seq'", [], |r| {
            r.get::<_, String>(0)
        })
        .ok()
        .and_then(|v| v.parse().ok());
    if seq != Some(graph_seq) {
        return Err(format!("index at {seq:?}, graph at {graph_seq}"));
    }
    let expr = tokens
        .iter()
        .map(|t| format!("\"{}\"", t.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" OR ");
    let mut stmt = conn
        .prepare("SELECT ref_id FROM memory_fts WHERE memory_fts MATCH ?")
        .map_err(|e| format!("fts: {e}"))?;
    let rows = stmt
        .query_map(params![expr], |r| r.get::<_, String>(0))
        .map_err(|e| format!("fts: {e}"))?;
    let mut out = HashSet::new();
    for r in rows {
        out.insert(r.map_err(|e| format!("fts: {e}"))?);
    }
    Ok(out)
}
