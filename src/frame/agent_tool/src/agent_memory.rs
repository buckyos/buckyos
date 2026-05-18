//! Agent Memory v2.8 — local file + JSONL log + SQLite/FTS5 cache.
//!
//! See `doc/opendan/Agent Memory v2.md` for the stable contract this module
//! implements. The Rust API mirrors the CLI verbs: [`AgentMemory::open`],
//! [`AgentMemory::set`], [`AgentMemory::remove`], [`AgentMemory::load`],
//! [`AgentMemory::get`], [`AgentMemory::list`], [`AgentMemory::verify`],
//! [`AgentMemory::compact`].

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, SecondsFormat, Utc};
use fs2::FileExt;
use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
use rusqlite::{params, Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const SCHEMA_VERSION: &str = "2.8";
pub const PRIMARY_LANGUAGE: &str = "en";
pub const META_DIR: &str = ".meta";
pub const META_JSON: &str = "meta.json";
pub const LOG_FILE: &str = "log.jsonl";
pub const STATE_FILE: &str = "state.jsonl";
pub const LOCK_FILE: &str = "lock";
pub const SQLITE_FILE: &str = "memory.sqlite";
pub const ARCHIVE_DIR: &str = "archive";

pub const DEFAULT_MAX_RECORDS: usize = 50;
pub const DEFAULT_MAX_BYTES: usize = 65536;
pub const DEFAULT_BODY_TRUNCATE_BYTES: usize = 4096;
pub const DEFAULT_LOCK_TIMEOUT: Duration = Duration::from_secs(5);
pub const SOFT_CONTENT_WARN_BYTES: usize = 256 * 1024;

pub const MAX_SEGMENT_BYTES: usize = 200;
pub const MIN_TAG_BYTES: usize = 2;
pub const MAX_TAG_BYTES: usize = 32;

const FORBIDDEN_FIRST_SEGMENTS: &[&str] = &[META_DIR, SQLITE_FILE];

/// Percent-encoding: only RFC3986 unreserved bytes are kept literal.
/// `unreserved = ALPHA / DIGIT / "-" / "." / "_" / "~"`.
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

#[derive(Debug, Error)]
pub enum AgentMemoryError {
    #[error("invalid argument: {0}")]
    Invalid(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("lock contention: {0}")]
    LockTimeout(String),
    #[error("corrupted state: {0}")]
    Corrupted(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

impl AgentMemoryError {
    /// CLI exit code per spec §3.2.
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::LockTimeout(_) => 2,
            Self::Corrupted(_) => 3,
            _ => 1,
        }
    }
}

pub type Result<T> = std::result::Result<T, AgentMemoryError>;

#[derive(Clone, Debug)]
pub struct AgentMemoryConfig {
    pub root: PathBuf,
    pub lock_timeout: Duration,
}

impl AgentMemoryConfig {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            lock_timeout: DEFAULT_LOCK_TIMEOUT,
        }
    }
}

/// One line in `.meta/log.jsonl` or `.meta/state.jsonl`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Envelope {
    pub schema_version: String,
    pub key: String,
    pub ts: String,
    pub valid: bool,
    pub reason: String,
    pub content_digest: Option<String>,
    pub content_size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WriterInfo {
    pub lang: String,
    pub r#impl: String,
    pub version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EncodingMeta {
    pub key_to_path: String,
    pub max_segment_bytes: usize,
    pub filename_format: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IndexMeta {
    pub engine: String,
    pub tokenizer: String,
    pub key_text: String,
    pub content_text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MetaJson {
    pub schema_version: String,
    pub primary_language: String,
    pub writer: WriterInfo,
    pub encoding: EncodingMeta,
    pub index: IndexMeta,
    pub compaction_strategy: String,
    pub created_at: String,
}

impl MetaJson {
    fn default_now() -> Self {
        Self {
            schema_version: SCHEMA_VERSION.to_string(),
            primary_language: PRIMARY_LANGUAGE.to_string(),
            writer: WriterInfo {
                lang: "rust".to_string(),
                r#impl: "agent-memory-rs".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            encoding: EncodingMeta {
                key_to_path: "percent".to_string(),
                max_segment_bytes: MAX_SEGMENT_BYTES,
                filename_format: "bare".to_string(),
            },
            index: IndexMeta {
                engine: "sqlite-fts5".to_string(),
                tokenizer: "unicode61 remove_diacritics 2".to_string(),
                key_text: "full_logical_key".to_string(),
                content_text: "content_body_without_preamble".to_string(),
            },
            compaction_strategy: "snapshot".to_string(),
            created_at: now_iso8601(),
        }
    }
}

/// Parsed system preamble at the head of a content file.
#[derive(Clone, Debug, Default)]
pub struct Preamble {
    pub importance: i64,
    pub expired_at: Option<String>,
    /// Raw content body without the preamble (and without the blank separator line).
    pub body: String,
    /// Number of leading bytes consumed by preamble + blank line.
    pub body_offset: usize,
}

/// Options for `load`.
#[derive(Clone, Debug)]
pub struct LoadOptions {
    pub max_records: usize,
    pub max_bytes: usize,
    pub body_truncate_bytes: usize,
    pub current_time: Option<DateTime<Utc>>,
}

impl Default for LoadOptions {
    fn default() -> Self {
        Self {
            max_records: DEFAULT_MAX_RECORDS,
            max_bytes: DEFAULT_MAX_BYTES,
            body_truncate_bytes: DEFAULT_BODY_TRUNCATE_BYTES,
            current_time: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct LoadItem {
    pub key: String,
    pub matched: Vec<String>,
    pub ts: String,
    pub size: usize,
    pub truncated: bool,
    pub content: String,
}

#[derive(Clone, Debug, Default)]
pub struct VerifyReport {
    pub ok_keys: usize,
    pub orphan_files: Vec<PathBuf>,
    pub tombstone_residue: Vec<PathBuf>,
    pub missing_content: Vec<String>,
    pub digest_mismatch: Vec<String>,
    pub repaired_index: bool,
}

impl VerifyReport {
    pub fn has_unrecoverable(&self) -> bool {
        !self.missing_content.is_empty()
    }
    pub fn is_clean(&self) -> bool {
        self.orphan_files.is_empty()
            && self.tombstone_residue.is_empty()
            && self.missing_content.is_empty()
            && self.digest_mismatch.is_empty()
    }
}

#[derive(Clone)]
pub struct AgentMemory {
    cfg: AgentMemoryConfig,
}

impl AgentMemory {
    /// Open (and initialize if needed) the memory root.
    pub fn open(cfg: AgentMemoryConfig) -> Result<Self> {
        let m = Self { cfg };
        m.ensure_initialized()?;
        Ok(m)
    }

    /// Explicit `init` verb. Idempotent.
    pub fn init(cfg: AgentMemoryConfig) -> Result<Self> {
        Self::open(cfg)
    }

    pub fn root(&self) -> &Path {
        &self.cfg.root
    }

    fn meta_dir(&self) -> PathBuf {
        self.cfg.root.join(META_DIR)
    }
    fn meta_json_path(&self) -> PathBuf {
        self.meta_dir().join(META_JSON)
    }
    fn log_path(&self) -> PathBuf {
        self.meta_dir().join(LOG_FILE)
    }
    fn state_path(&self) -> PathBuf {
        self.meta_dir().join(STATE_FILE)
    }
    fn lock_path(&self) -> PathBuf {
        self.meta_dir().join(LOCK_FILE)
    }
    fn sqlite_path(&self) -> PathBuf {
        self.cfg.root.join(SQLITE_FILE)
    }
    fn archive_dir(&self) -> PathBuf {
        self.meta_dir().join(ARCHIVE_DIR)
    }

    fn ensure_initialized(&self) -> Result<()> {
        fs::create_dir_all(&self.cfg.root)?;
        fs::create_dir_all(self.meta_dir())?;

        let meta_path = self.meta_json_path();
        if !meta_path.exists() {
            let m = MetaJson::default_now();
            let s = serde_json::to_vec_pretty(&m)?;
            atomic_write(&meta_path, &s)?;
        } else {
            let bytes = fs::read(&meta_path)?;
            let m: MetaJson = serde_json::from_slice(&bytes)
                .map_err(|e| AgentMemoryError::Corrupted(format!("meta.json: {}", e)))?;
            if m.primary_language != PRIMARY_LANGUAGE {
                return Err(AgentMemoryError::Invalid(format!(
                    "unsupported primary_language; v2.8 only supports en (got {})",
                    m.primary_language
                )));
            }
            if schema_major(&m.schema_version) != schema_major(SCHEMA_VERSION) {
                return Err(AgentMemoryError::Invalid(format!(
                    "incompatible schema_version major: {}",
                    m.schema_version
                )));
            }
            if m.encoding.key_to_path != "percent" {
                return Err(AgentMemoryError::Invalid(format!(
                    "unsupported encoding.key_to_path: {}",
                    m.encoding.key_to_path
                )));
            }
            if m.compaction_strategy != "snapshot" && m.compaction_strategy != "log_only" {
                return Err(AgentMemoryError::Invalid(format!(
                    "unsupported compaction_strategy: {}",
                    m.compaction_strategy
                )));
            }
        }

        // Ensure log + lock files exist.
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.log_path())?;
        OpenOptions::new()
            .create(true)
            .write(true)
            .read(true)
            .open(self.lock_path())?;

        // Touch SQLite and ensure schema. Holds the writer lock briefly.
        self.with_writer_lock(|_| {
            let conn = self.open_db()?;
            ensure_schema(&conn)?;
            Ok(())
        })
    }

    // ---------------------------------------------------------------- set

    pub fn set(&self, key: &str, content: &str, reason: &str) -> Result<()> {
        let key = normalize_key(key)?;
        validate_content(content)?;
        validate_reason(reason)?;
        if reason_is_empty(reason) {
            return Err(AgentMemoryError::Invalid("reason is empty".into()));
        }

        self.with_writer_lock(|_| {
            let file_path = self.key_to_path(&key);
            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent)?;
            }
            atomic_write(&file_path, content.as_bytes())?;

            let digest = blake3_hex(content.as_bytes());
            let env = Envelope {
                schema_version: SCHEMA_VERSION.to_string(),
                key: key.clone(),
                ts: now_iso8601(),
                valid: true,
                reason: reason.to_string(),
                content_digest: Some(format!("blake3:{}", digest)),
                content_size: content.len() as u64,
            };
            self.append_envelope(&env)?;

            // Index update is best-effort; swallow errors but log.
            if let Err(e) = self.update_index_for(&env, &file_path, content) {
                log::warn!(
                    "agent_memory: failed to update sqlite index for {}: {}",
                    key,
                    e
                );
            }
            Ok(())
        })
    }

    // ---------------------------------------------------------------- remove

    pub fn remove(&self, key: &str, reason: Option<&str>) -> Result<()> {
        let key = normalize_key(key)?;
        if let Some(r) = reason {
            validate_reason(r)?;
        }
        let reason_text = reason.unwrap_or("").to_string();

        self.with_writer_lock(|_| {
            let env = Envelope {
                schema_version: SCHEMA_VERSION.to_string(),
                key: key.clone(),
                ts: now_iso8601(),
                valid: false,
                reason: reason_text,
                content_digest: None,
                content_size: 0,
            };
            self.append_envelope(&env)?;

            let file_path = self.key_to_path(&key);
            match fs::remove_file(&file_path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
            if let Err(e) = self.delete_from_index(&key) {
                log::warn!(
                    "agent_memory: failed to delete sqlite index for {}: {}",
                    key,
                    e
                );
            }
            Ok(())
        })
    }

    // ---------------------------------------------------------------- get

    pub fn get(&self, key: &str) -> Result<String> {
        let key = normalize_key(key)?;
        let states = self.replay_states()?;
        let env = states
            .get(&key)
            .ok_or_else(|| AgentMemoryError::NotFound(key.clone()))?;
        if !env.valid {
            return Err(AgentMemoryError::NotFound(key));
        }
        let path = self.key_to_path(&key);
        let bytes = fs::read(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                AgentMemoryError::Corrupted(format!("missing content for {}", key))
            } else {
                AgentMemoryError::Io(e)
            }
        })?;
        String::from_utf8(bytes)
            .map_err(|_| AgentMemoryError::Corrupted(format!("non-utf8 content for {}", key)))
    }

    // ---------------------------------------------------------------- list

    pub fn list(&self, prefix: Option<&str>) -> Result<Vec<String>> {
        let prefix = prefix.unwrap_or("/");
        if !prefix.starts_with('/') {
            return Err(AgentMemoryError::Invalid(
                "list prefix must start with /".into(),
            ));
        }
        let states = self.replay_states()?;
        let mut out: Vec<String> = states
            .into_iter()
            .filter_map(|(k, e)| {
                if e.valid && (prefix == "/" || k == prefix || k.starts_with(prefix)) {
                    Some(k)
                } else {
                    None
                }
            })
            .collect();
        out.sort();
        Ok(out)
    }

    // ---------------------------------------------------------------- load

    pub fn load(&self, tags: &[String], opts: LoadOptions) -> Result<Vec<LoadItem>> {
        let now = opts
            .current_time
            .unwrap_or_else(Utc::now)
            .to_rfc3339_opts(SecondsFormat::Secs, true);

        let normalized_tags: Vec<String> = tags
            .iter()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect();
        for t in &normalized_tags {
            validate_tag(t)?;
        }
        let star_query =
            normalized_tags.is_empty() || (normalized_tags.len() == 1 && normalized_tags[0] == "*");

        // Surface candidates from SQLite when possible.
        let conn = self.open_db()?;
        ensure_schema(&conn)?;

        struct Candidate {
            key: String,
            ts: String,
            importance: i64,
            #[allow(dead_code)]
            expired_at: Option<String>,
            file_path: String,
            bm25: f64,
        }

        let mut candidates: Vec<Candidate> = Vec::new();
        if star_query {
            let mut stmt = conn.prepare(
                "SELECT key, ts, COALESCE(importance, 0), expired_at, file_path
                 FROM memory
                 WHERE valid = 1
                   AND (expired_at IS NULL OR expired_at > ?)",
            )?;
            let rows = stmt.query_map(params![now], |r| {
                Ok(Candidate {
                    key: r.get(0)?,
                    ts: r.get(1)?,
                    importance: r.get(2)?,
                    expired_at: r.get(3)?,
                    file_path: r.get(4)?,
                    bm25: 0.0,
                })
            })?;
            for c in rows {
                candidates.push(c?);
            }
        } else {
            let phrases: Vec<String> = normalized_tags
                .iter()
                .map(|t| {
                    // §8.3 forbids quotes/control chars, so wrapping in `"` is safe.
                    let normalized = collapse_whitespace(t).to_lowercase();
                    format!("\"{}\"", normalized)
                })
                .collect();
            let match_expr = phrases.join(" OR ");
            let mut stmt = conn.prepare(
                "SELECT m.key, m.ts, COALESCE(m.importance, 0), m.expired_at, m.file_path,
                        bm25(memory_fts, 4.0, 1.0) AS bm25_score
                 FROM memory_fts
                 JOIN memory AS m ON m.key = memory_fts.key
                 WHERE memory_fts MATCH ?
                   AND m.valid = 1
                   AND (m.expired_at IS NULL OR m.expired_at > ?)
                 ORDER BY bm25_score ASC, m.ts DESC, COALESCE(m.importance, 0) DESC, m.key ASC",
            )?;
            let rows = stmt.query_map(params![match_expr, now], |r| {
                Ok(Candidate {
                    key: r.get(0)?,
                    ts: r.get(1)?,
                    importance: r.get(2)?,
                    expired_at: r.get(3)?,
                    file_path: r.get(4)?,
                    bm25: r.get(5)?,
                })
            })?;
            for c in rows {
                candidates.push(c?);
            }
        }

        // Compute boost and matched tags by re-tokenizing content body.
        struct Ranked {
            key: String,
            ts: String,
            importance: i64,
            file_path: String,
            bm25: f64,
            boost: i64,
            matched: Vec<String>,
        }

        let lowered_tags: Vec<String> = normalized_tags
            .iter()
            .map(|t| collapse_whitespace(t).to_lowercase())
            .collect();

        let mut ranked: Vec<Ranked> = Vec::new();
        for c in candidates {
            let path = PathBuf::from(&c.file_path);
            let content = match fs::read_to_string(&path) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let preamble = parse_preamble(&content);
            let haystack = format!("{} {}", c.key.to_lowercase(), preamble.body.to_lowercase());

            let mut boost = 0i64;
            let mut matched = Vec::new();
            if !star_query {
                for (idx, tag) in lowered_tags.iter().enumerate() {
                    if tag.is_empty() || tag == "*" {
                        continue;
                    }
                    if phrase_hit(&haystack, tag) {
                        boost += match idx {
                            0 => 8,
                            1 => 4,
                            2 => 2,
                            _ => 1,
                        };
                        matched.push(normalized_tags[idx].clone());
                    }
                }
                if matched.is_empty() && !star_query {
                    // FTS5 returned candidate but local re-check found no phrase
                    // match. Keep it (FTS5 is the contract for candidate set);
                    // boost stays 0.
                }
            }

            ranked.push(Ranked {
                key: c.key,
                ts: c.ts,
                importance: c.importance,
                file_path: c.file_path,
                bm25: c.bm25,
                boost,
                matched,
            });
        }

        if star_query {
            ranked.sort_by(|a, b| {
                b.ts.cmp(&a.ts)
                    .then_with(|| b.importance.cmp(&a.importance))
                    .then_with(|| a.key.cmp(&b.key))
            });
        } else {
            ranked.sort_by(|a, b| {
                b.boost
                    .cmp(&a.boost)
                    .then_with(|| {
                        a.bm25
                            .partial_cmp(&b.bm25)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .then_with(|| b.ts.cmp(&a.ts))
                    .then_with(|| b.importance.cmp(&a.importance))
                    .then_with(|| a.key.cmp(&b.key))
            });
        }

        // Apply truncation, max-records, max-bytes.
        let mut out = Vec::new();
        let mut bytes_used = 0usize;
        for r in ranked {
            if out.len() >= opts.max_records {
                break;
            }
            let content = match fs::read_to_string(&r.file_path) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let (body_text, truncated) =
                truncate_at_char_boundary(&content, opts.body_truncate_bytes);
            let size = body_text.len();
            if bytes_used.saturating_add(size) > opts.max_bytes && !out.is_empty() {
                break;
            }
            bytes_used = bytes_used.saturating_add(size);
            out.push(LoadItem {
                key: r.key,
                matched: r.matched,
                ts: r.ts,
                size,
                truncated,
                content: body_text,
            });
        }

        Ok(out)
    }

    /// Format a list of load items in the spec's text format.
    pub fn format_load_items(items: &[LoadItem]) -> String {
        let mut out = String::new();
        for item in items {
            out.push_str(&format!("KEY {}\n", item.key));
            out.push_str(&format!("SIZE {}\n", item.size));
            out.push_str(&format!(
                "TRUNCATED {}\n",
                if item.truncated { 1 } else { 0 }
            ));
            out.push_str(&format!("MATCHED {}\n", item.matched.join(",")));
            out.push_str(&format!("TS {}\n", item.ts));
            out.push_str("---\n");
            out.push_str(&item.content);
            if !item.content.ends_with('\n') {
                out.push('\n');
            }
            out.push_str("END\n");
        }
        out
    }

    // ---------------------------------------------------------------- verify

    pub fn verify(&self, repair: bool) -> Result<VerifyReport> {
        let mut report = VerifyReport::default();
        let states = self.replay_states()?;

        // 1. Cross-check envelopes vs files.
        let mut keys_with_files: HashSet<PathBuf> = HashSet::new();
        for (key, env) in &states {
            let path = self.key_to_path(key);
            keys_with_files.insert(path.clone());
            if env.valid {
                match fs::read(&path) {
                    Ok(bytes) => {
                        if let Some(want_digest) = &env.content_digest {
                            let actual = format!("blake3:{}", blake3_hex(&bytes));
                            if &actual != want_digest {
                                report.digest_mismatch.push(key.clone());
                            }
                        }
                        report.ok_keys += 1;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        report.missing_content.push(key.clone());
                    }
                    Err(e) => return Err(e.into()),
                }
            } else {
                // Tombstone — file should not exist.
                if path.exists() {
                    report.tombstone_residue.push(path.clone());
                    if repair {
                        let _ = fs::remove_file(&path);
                    }
                }
            }
        }

        // 2. Walk filesystem to find orphans.
        let mut on_disk: Vec<PathBuf> = Vec::new();
        walk_business_files(&self.cfg.root, &mut on_disk)?;
        for p in on_disk {
            if !keys_with_files.contains(&p) {
                report.orphan_files.push(p);
            }
        }

        // 3. Optionally rebuild SQLite.
        if repair {
            self.with_writer_lock(|_| self.rebuild_index_locked(&states))?;
            report.repaired_index = true;
        }

        if report.has_unrecoverable() && !repair {
            return Err(AgentMemoryError::Corrupted(format!(
                "{} key(s) missing content files",
                report.missing_content.len()
            )));
        }
        Ok(report)
    }

    // ---------------------------------------------------------------- compact

    pub fn compact(&self) -> Result<()> {
        self.with_writer_lock(|_| {
            let states = self.replay_states_locked()?;
            // Write state.jsonl atomically with all known envelopes.
            let mut buf = Vec::new();
            // Sort by key for determinism.
            let mut ordered: Vec<&Envelope> = states.values().collect();
            ordered.sort_by(|a, b| a.key.cmp(&b.key));
            for env in &ordered {
                serde_json::to_writer(&mut buf, env)?;
                buf.push(b'\n');
            }
            atomic_write(&self.state_path(), &buf)?;

            // Archive current log.jsonl.
            fs::create_dir_all(self.archive_dir())?;
            let archive_path = self
                .archive_dir()
                .join(format!("log_{}.jsonl", Utc::now().format("%Y%m%dT%H%M%SZ")));
            let log_path = self.log_path();
            if log_path.exists() {
                fs::rename(&log_path, &archive_path)?;
            }
            // Re-create empty log.
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)?;

            // Clean tombstone residue & rebuild index.
            for env in &ordered {
                if !env.valid {
                    let path = self.key_to_path(&env.key);
                    let _ = fs::remove_file(&path);
                }
            }
            self.rebuild_index_locked(&states)?;
            Ok(())
        })
    }

    // ----------------------------------------------------------- internals

    fn key_to_path(&self, key: &str) -> PathBuf {
        let mut p = self.cfg.root.clone();
        let stripped = key.trim_start_matches('/');
        for seg in stripped.split('/') {
            let encoded = utf8_percent_encode(seg, PERCENT_SET).to_string();
            p.push(encoded);
        }
        p
    }

    fn open_db(&self) -> Result<Connection> {
        let conn = Connection::open_with_flags(
            self.sqlite_path(),
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", true)?;
        Ok(conn)
    }

    fn append_envelope(&self, env: &Envelope) -> Result<()> {
        let mut line = serde_json::to_vec(env)?;
        line.push(b'\n');
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.log_path())?;
        f.write_all(&line)?;
        f.sync_all()?;
        Ok(())
    }

    fn update_index_for(&self, env: &Envelope, file_path: &Path, content: &str) -> Result<()> {
        let preamble = parse_preamble(content);
        let conn = self.open_db()?;
        ensure_schema(&conn)?;
        let reason_summary = summarize_reason(&env.reason);
        let file_path_str = file_path.to_string_lossy().to_string();
        // Upsert memory row.
        conn.execute(
            "INSERT INTO memory(key, file_path, ts, valid, importance, expired_at, reason_summary, content_size)
             VALUES(?, ?, ?, 1, ?, ?, ?, ?)
             ON CONFLICT(key) DO UPDATE SET
                file_path=excluded.file_path,
                ts=excluded.ts,
                valid=1,
                importance=excluded.importance,
                expired_at=excluded.expired_at,
                reason_summary=excluded.reason_summary,
                content_size=excluded.content_size",
            params![
                env.key,
                file_path_str,
                env.ts,
                preamble.importance,
                preamble.expired_at,
                reason_summary,
                env.content_size as i64,
            ],
        )?;
        // Reset FTS row.
        conn.execute("DELETE FROM memory_fts WHERE key = ?", params![env.key])?;
        conn.execute(
            "INSERT INTO memory_fts(key, key_text, content_text) VALUES(?, ?, ?)",
            params![env.key, env.key, preamble.body],
        )?;
        Ok(())
    }

    fn delete_from_index(&self, key: &str) -> Result<()> {
        let conn = self.open_db()?;
        ensure_schema(&conn)?;
        conn.execute("DELETE FROM memory WHERE key = ?", params![key])?;
        conn.execute("DELETE FROM memory_fts WHERE key = ?", params![key])?;
        Ok(())
    }

    fn rebuild_index_locked(&self, states: &BTreeMap<String, Envelope>) -> Result<()> {
        // Drop and re-open the SQLite file to start clean.
        let _ = fs::remove_file(self.sqlite_path());
        let conn = self.open_db()?;
        ensure_schema(&conn)?;
        for env in states.values() {
            if !env.valid {
                continue;
            }
            let path = self.key_to_path(&env.key);
            let content = match fs::read_to_string(&path) {
                Ok(s) => s,
                Err(_) => continue,
            };
            self.update_index_for(env, &path, &content)?;
        }
        Ok(())
    }

    fn replay_states(&self) -> Result<BTreeMap<String, Envelope>> {
        // Read-only path: do not require lock.
        let mut states = BTreeMap::new();
        if self.state_path().exists() {
            apply_jsonl(&self.state_path(), &mut states)?;
        }
        if self.log_path().exists() {
            apply_jsonl(&self.log_path(), &mut states)?;
        }
        Ok(states)
    }

    fn replay_states_locked(&self) -> Result<BTreeMap<String, Envelope>> {
        self.replay_states()
    }

    fn with_writer_lock<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&File) -> Result<T>,
    {
        let lock_file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(self.lock_path())?;
        let deadline = Instant::now() + self.cfg.lock_timeout;
        loop {
            match lock_file.try_lock_exclusive() {
                Ok(()) => break,
                Err(e) => {
                    if Instant::now() >= deadline {
                        return Err(AgentMemoryError::LockTimeout(format!(
                            "could not acquire {}: {}",
                            self.lock_path().display(),
                            e
                        )));
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        }
        let result = f(&lock_file);
        let _ = FileExt::unlock(&lock_file);
        result
    }
}

// ============================================================ free helpers

fn schema_major(v: &str) -> &str {
    v.split('.').next().unwrap_or("")
}

fn now_iso8601() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn random_suffix() -> String {
    // pid + nanos is good enough for tmp file uniqueness; not security-sensitive.
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("{}-{}", pid, nanos)
}

fn blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        AgentMemoryError::Invalid(format!("path has no parent: {}", path.display()))
    })?;
    fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(
        "{}.tmp.{}",
        path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "memory".to_string()),
        random_suffix()
    ));
    {
        let mut f = OpenOptions::new().create_new(true).write(true).open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    sync_dir(parent)?;
    Ok(())
}

#[cfg(unix)]
fn sync_dir(path: &Path) -> Result<()> {
    let f = File::open(path)?;
    f.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_dir(_path: &Path) -> Result<()> {
    Ok(())
}

fn ensure_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS memory (
           key            TEXT PRIMARY KEY,
           file_path      TEXT NOT NULL,
           ts             TEXT NOT NULL,
           valid          INTEGER NOT NULL,
           importance     INTEGER,
           expired_at     TEXT,
           reason_summary TEXT,
           content_size   INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_memory_ts  ON memory(ts);
         CREATE INDEX IF NOT EXISTS idx_memory_imp ON memory(importance);
         CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts USING fts5(
           key UNINDEXED,
           key_text,
           content_text,
           tokenize = 'unicode61 remove_diacritics 2'
         );",
    )?;
    Ok(())
}

fn apply_jsonl(path: &Path, states: &mut BTreeMap<String, Envelope>) -> Result<()> {
    let f = File::open(path)?;
    let reader = BufReader::new(f);
    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let env: Envelope = match serde_json::from_str(trimmed) {
            Ok(e) => e,
            Err(e) => {
                log::warn!(
                    "agent_memory: skipping malformed envelope in {}: {}",
                    path.display(),
                    e
                );
                continue;
            }
        };
        states.insert(env.key.clone(), env);
    }
    Ok(())
}

fn walk_business_files(root: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let mut stack: Vec<PathBuf> = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let p = entry.path();
        let name = match p.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if name == META_DIR || name == SQLITE_FILE {
            continue;
        }
        if entry.file_type()?.is_dir() {
            stack.push(p);
        } else {
            out.push(p);
        }
    }
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let p = entry.path();
            if entry.file_type()?.is_dir() {
                stack.push(p);
            } else {
                out.push(p);
            }
        }
    }
    Ok(())
}

// ------------------------------------------------------------ key validation

fn normalize_key(raw: &str) -> Result<String> {
    if raw.is_empty() {
        return Err(AgentMemoryError::Invalid("key is empty".into()));
    }
    if !raw.starts_with('/') {
        return Err(AgentMemoryError::Invalid(format!(
            "key must start with '/': {}",
            raw
        )));
    }
    if raw.contains('\0') || raw.contains('\n') || raw.contains('\r') {
        return Err(AgentMemoryError::Invalid(
            "key contains NUL or newline".into(),
        ));
    }
    if raw.chars().any(|c| c.is_control()) {
        return Err(AgentMemoryError::Invalid(
            "key contains control characters".into(),
        ));
    }

    // Collapse repeated '/' and split.
    let mut segs = Vec::new();
    for seg in raw.split('/') {
        if seg.is_empty() {
            continue;
        }
        if seg == "." || seg == ".." {
            return Err(AgentMemoryError::Invalid(format!(
                "key has invalid segment '{}'",
                seg
            )));
        }
        if seg.as_bytes().len() > MAX_SEGMENT_BYTES {
            return Err(AgentMemoryError::Invalid(format!(
                "key segment exceeds {} bytes: {}",
                MAX_SEGMENT_BYTES, seg
            )));
        }
        // Sanity: no Path traversal sneaks in via Component parsing.
        for c in Path::new(seg).components() {
            match c {
                Component::Normal(_) => {}
                _ => {
                    return Err(AgentMemoryError::Invalid(format!(
                        "key segment is not a normal path component: {}",
                        seg
                    )))
                }
            }
        }
        segs.push(seg.to_string());
    }
    if segs.is_empty() {
        return Err(AgentMemoryError::Invalid("key has no segments".into()));
    }
    if FORBIDDEN_FIRST_SEGMENTS.contains(&segs[0].as_str()) {
        return Err(AgentMemoryError::Invalid(format!(
            "key first segment must not be reserved: {}",
            segs[0]
        )));
    }
    Ok(format!("/{}", segs.join("/")))
}

fn validate_content(content: &str) -> Result<()> {
    if content.is_empty() {
        return Err(AgentMemoryError::Invalid("content is empty".into()));
    }
    if content.starts_with('\u{feff}') {
        return Err(AgentMemoryError::Invalid("content has UTF-8 BOM".into()));
    }
    Ok(())
}

fn validate_reason(reason: &str) -> Result<()> {
    if reason.is_empty() {
        return Err(AgentMemoryError::Invalid("reason is empty".into()));
    }
    if reason.contains('\0') {
        return Err(AgentMemoryError::Invalid("reason contains NUL".into()));
    }
    Ok(())
}

fn reason_is_empty(reason: &str) -> bool {
    reason.trim().is_empty()
}

fn validate_tag(tag: &str) -> Result<()> {
    let t = tag.trim();
    let len = t.as_bytes().len();
    if len < MIN_TAG_BYTES || len > MAX_TAG_BYTES {
        return Err(AgentMemoryError::Invalid(format!(
            "tag length must be {}-{} bytes: {:?}",
            MIN_TAG_BYTES, MAX_TAG_BYTES, t
        )));
    }
    let mut has_alnum = false;
    for c in t.chars() {
        let ok = matches!(c, 'A'..='Z' | 'a'..='z' | '0'..='9' | ' ' | '-');
        if !ok {
            return Err(AgentMemoryError::Invalid(format!(
                "tag has forbidden character {:?}: {:?}",
                c, t
            )));
        }
        if c.is_ascii_alphanumeric() {
            has_alnum = true;
        }
    }
    if !has_alnum {
        return Err(AgentMemoryError::Invalid(format!(
            "tag must contain at least one alphanumeric: {:?}",
            t
        )));
    }
    Ok(())
}

fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for c in s.chars() {
        if c == ' ' {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    out.trim().to_string()
}

/// Substring containment, used as a cheap stand-in for "the FTS5 phrase
/// matches this row" when computing per-tag boost/matched lists. Tags are
/// validated to ASCII so a case-insensitive substring check on the lowered
/// haystack is a sound approximation for English content.
fn phrase_hit(haystack: &str, lowered_tag: &str) -> bool {
    if lowered_tag.is_empty() {
        return false;
    }
    haystack.contains(lowered_tag)
}

/// Truncate `s` at a UTF-8 char boundary so that the returned slice is at
/// most `max_bytes` bytes. Returns `(slice, truncated)`.
fn truncate_at_char_boundary(s: &str, max_bytes: usize) -> (String, bool) {
    if s.len() <= max_bytes {
        return (s.to_string(), false);
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    (s[..end].to_string(), true)
}

fn summarize_reason(reason: &str) -> String {
    // Take the first non-empty line, truncated to 200 chars.
    let line = reason.lines().next().unwrap_or("");
    if line.len() <= 200 {
        line.to_string()
    } else {
        let mut end = 200;
        while end > 0 && !line.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &line[..end])
    }
}

/// Parse the optional system preamble at the head of `content`.
///
/// Per §7.1: lines matching `^[A-Z][A-Za-z0-9-]*: ` form the preamble; an empty
/// line terminates it. If the very first line does not match, there is no
/// preamble and `body` equals the full input.
pub fn parse_preamble(content: &str) -> Preamble {
    let bytes = content.as_bytes();
    let mut p = Preamble::default();

    // First line must match `^[A-Z][A-Za-z0-9-]*: `.
    let first_line_end = match content.find('\n') {
        Some(i) => i,
        None => content.len(),
    };
    let first_line = &content[..first_line_end];
    if !is_preamble_line(first_line) {
        p.body = content.to_string();
        p.body_offset = 0;
        return p;
    }

    let mut cursor = 0usize;
    let mut headers: HashMap<String, String> = HashMap::new();
    loop {
        let line_end = bytes[cursor..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|i| cursor + i)
            .unwrap_or(bytes.len());
        let line = &content[cursor..line_end];
        if line.is_empty() {
            // Blank line terminates preamble. Advance past the newline.
            let body_start = if line_end < bytes.len() {
                line_end + 1
            } else {
                bytes.len()
            };
            p.body_offset = body_start;
            p.body = content[body_start..].to_string();
            break;
        }
        if !is_preamble_line(line) {
            // Malformed — treat everything as body.
            p.body = content.to_string();
            p.body_offset = 0;
            return p;
        }
        if let Some(colon) = line.find(": ") {
            let k = line[..colon].to_string();
            let v = line[colon + 2..].to_string();
            headers.insert(k, v);
        }
        if line_end >= bytes.len() {
            // Preamble runs to EOF without a blank line — treat as body to be safe.
            p.body = content.to_string();
            p.body_offset = 0;
            return p;
        }
        cursor = line_end + 1;
    }

    if let Some(v) = headers.get("Importance") {
        if let Ok(n) = v.trim().parse::<i64>() {
            p.importance = n;
        }
    }
    if let Some(v) = headers.get("Expired-At") {
        let v = v.trim();
        if !v.is_empty() {
            p.expired_at = Some(v.to_string());
        }
    }
    p
}

fn is_preamble_line(line: &str) -> bool {
    let bytes = line.as_bytes();
    let colon = match line.find(": ") {
        Some(c) => c,
        None => return false,
    };
    if colon == 0 {
        return false;
    }
    let head = &bytes[..colon];
    if !head[0].is_ascii_uppercase() {
        return false;
    }
    for &b in &head[1..] {
        let ok = b.is_ascii_alphanumeric() || b == b'-';
        if !ok {
            return false;
        }
    }
    true
}

// ============================================================== unit tests
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn open_tmp() -> (TempDir, AgentMemory) {
        let tmp = TempDir::new().unwrap();
        let m = AgentMemory::open(AgentMemoryConfig::new(tmp.path())).unwrap();
        (tmp, m)
    }

    #[test]
    fn init_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let _ = AgentMemory::open(AgentMemoryConfig::new(tmp.path())).unwrap();
        let _ = AgentMemory::open(AgentMemoryConfig::new(tmp.path())).unwrap();
        assert!(tmp.path().join(".meta/meta.json").exists());
        assert!(tmp.path().join(".meta/log.jsonl").exists());
        assert!(tmp.path().join(".meta/lock").exists());
        assert!(tmp.path().join("memory.sqlite").exists());
    }

    #[test]
    fn set_get_remove_roundtrip() {
        let (_tmp, m) = open_tmp();
        m.set(
            "/user/preference/style",
            "concise english",
            "user conversation;c=1",
        )
        .unwrap();
        assert_eq!(m.get("/user/preference/style").unwrap(), "concise english");
        m.remove("/user/preference/style", Some("user removed"))
            .unwrap();
        assert!(matches!(
            m.get("/user/preference/style"),
            Err(AgentMemoryError::NotFound(_))
        ));
    }

    #[test]
    fn list_filters_by_prefix_and_tombstones() {
        let (_tmp, m) = open_tmp();
        m.set("/user/a", "x", "r").unwrap();
        m.set("/user/b", "y", "r").unwrap();
        m.set("/kb/c", "z", "r").unwrap();
        m.remove("/user/b", None).unwrap();
        let users = m.list(Some("/user/")).unwrap();
        assert_eq!(users, vec!["/user/a".to_string()]);
        let all = m.list(None).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn key_validation_rejects_bad_input() {
        let (_tmp, m) = open_tmp();
        assert!(m.set("user/no-leading-slash", "x", "r").is_err());
        assert!(m.set("/.meta/blocked", "x", "r").is_err());
        assert!(m.set("/a/../b", "x", "r").is_err());
        assert!(m.set("/a", "", "r").is_err());
        assert!(m.set("/a", "x", "").is_err());
    }

    #[test]
    fn load_returns_recent_when_no_tags() {
        let (_tmp, m) = open_tmp();
        m.set("/user/a", "alpha content", "r").unwrap();
        m.set("/user/b", "bravo content", "r").unwrap();
        let items = m.load(&[], LoadOptions::default()).unwrap();
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn load_filters_by_tag_match() {
        let (_tmp, m) = open_tmp();
        m.set("/user/dental", "Dental followup at 10am", "r")
            .unwrap();
        m.set("/user/groceries", "Buy bread and milk", "r").unwrap();
        let items = m
            .load(&["dental".to_string()], LoadOptions::default())
            .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].key, "/user/dental");
        assert!(items[0].matched.contains(&"dental".to_string()));
    }

    #[test]
    fn load_skips_expired_items() {
        let (_tmp, m) = open_tmp();
        m.set(
            "/user/expired",
            "Importance: 1\nExpired-At: 2000-01-01T00:00:00Z\n\nstale",
            "r",
        )
        .unwrap();
        let opts = LoadOptions {
            current_time: Some(Utc::now()),
            ..Default::default()
        };
        let items = m.load(&[], opts).unwrap();
        assert!(items.is_empty(), "expired item should be filtered");
    }

    #[test]
    fn preamble_parsing() {
        let pre = parse_preamble("Importance: 3\nExpired-At: 2030-01-01T00:00:00Z\n\nbody text");
        assert_eq!(pre.importance, 3);
        assert_eq!(pre.expired_at.as_deref(), Some("2030-01-01T00:00:00Z"));
        assert_eq!(pre.body, "body text");

        let no_pre = parse_preamble("just a body");
        assert_eq!(no_pre.body, "just a body");
        assert_eq!(no_pre.importance, 0);
    }

    #[test]
    fn tag_validation_enforces_charset() {
        assert!(validate_tag("dental").is_ok());
        assert!(validate_tag("phone case").is_ok());
        assert!(validate_tag("a").is_err()); // too short
        assert!(validate_tag("with\"quote").is_err());
        assert!(validate_tag("中文").is_err());
    }

    #[test]
    fn format_load_items_uses_size_prefix() {
        let items = vec![LoadItem {
            key: "/k".to_string(),
            matched: vec!["a".into()],
            ts: "2026-05-09T10:00:00Z".to_string(),
            size: 5,
            truncated: false,
            content: "hello".to_string(),
        }];
        let s = AgentMemory::format_load_items(&items);
        assert!(s.contains("KEY /k\n"));
        assert!(s.contains("SIZE 5\n"));
        assert!(s.contains("MATCHED a\n"));
        assert!(s.contains("---\nhello\nEND\n"));
    }

    #[test]
    fn key_to_path_percent_encodes() {
        let tmp = TempDir::new().unwrap();
        let m = AgentMemory::open(AgentMemoryConfig::new(tmp.path())).unwrap();
        let p = m.key_to_path("/user/calendar/2026-02-23 dental");
        assert!(p.ends_with("user/calendar/2026-02-23%20dental"));
    }

    #[test]
    fn verify_detects_orphan_file() {
        let (tmp, m) = open_tmp();
        m.set("/user/a", "content", "r").unwrap();
        let orphan = tmp.path().join("user").join("orphan");
        fs::write(&orphan, "stray").unwrap();
        let r = m.verify(false).unwrap();
        assert_eq!(r.orphan_files.len(), 1);
    }

    #[test]
    fn compact_writes_state_and_archives_log() {
        let (_tmp, m) = open_tmp();
        m.set("/user/a", "alpha", "r").unwrap();
        m.set("/user/b", "bravo", "r").unwrap();
        m.remove("/user/a", None).unwrap();
        m.compact().unwrap();
        assert!(m.state_path().exists());
        let archives: Vec<_> = fs::read_dir(m.archive_dir()).unwrap().collect();
        assert_eq!(archives.len(), 1);
        // After compaction /user/a tombstone retained; its file removed.
        assert!(!m.cfg.root.join("user/a").exists());
        assert!(m.cfg.root.join("user/b").exists());
    }

    #[test]
    fn replay_after_reopen_reconstructs_state() {
        let tmp = TempDir::new().unwrap();
        {
            let m = AgentMemory::open(AgentMemoryConfig::new(tmp.path())).unwrap();
            m.set("/user/x", "value", "r").unwrap();
        }
        // Drop the SQLite file to ensure replay is from log.
        let _ = fs::remove_file(tmp.path().join("memory.sqlite"));
        let m = AgentMemory::open(AgentMemoryConfig::new(tmp.path())).unwrap();
        assert_eq!(m.get("/user/x").unwrap(), "value");
    }
}
