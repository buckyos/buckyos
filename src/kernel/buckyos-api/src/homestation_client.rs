use crate::bucket_client::{BucketDecl, BucketKind, BucketScope, DataPolicy};
use crate::{RdbBackend, RdbInstanceConfig, RdbPartition};
use ::kRPC::kRPC;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

pub const HOMESTATION_UNIQUE_ID: &str = "homestation";
pub const HOMESTATION_SERVICE_NAME: &str = "homestation";
pub const HOMESTATION_SERVICE_PORT: u16 = 4130;
pub const HOMESTATION_HTTP_PATH: &str = "/kapi/homestation";
/// Zone-level protocol paths forwarded by the gateway: `/home/` (zone index, DID lookup),
/// `/home/<user>/...` (each user's stream, entries, objects, inbox), `/home/~zone/...`.
pub const HOMESTATION_PROTOCOL_PREFIX: &str = "/home/";
/// Short hostname (`homestation.<zone>`) whose `/` opens the default feed, like `www.<zone>`.
pub const HOMESTATION_HOST_PREFIX: &str = "homestation";

/// The one database of the service: every user's home (rows carry the `owner` username) and
/// the zone feed list. Schema in doc/homestation/HomeStation Durable Data Schema.md.
pub const HOMESTATION_RDB_INSTANCE_ID: &str = "homestation-main";
pub const HOMESTATION_RDB_SCHEMA_VERSION: u64 = 3;

/// `user/homestation/<user>/<name>` buckets (doc/homestation/HomeStation Durable Data Schema.md).
/// Files of the user's published entries, key `<namespace>/<entry key>/<version>/<n>`.
pub const HOMESTATION_BUCKET_PUBLISHED: &str = "published";
/// Object versions of the user's published entries, key `<namespace>/<entry key>/<version>`.
pub const HOMESTATION_BUCKET_ENTRIES: &str = "entries";
/// Files of other publishers' posts prepared for reading (ambient), key `<feed obj id>/<file obj id>`.
pub const HOMESTATION_BUCKET_FOLLOWING: &str = "following";
/// Other publishers' Feed Objects prepared for reading (ambient), key `<feed obj id>[/<wrapped feed obj id>]`.
pub const HOMESTATION_BUCKET_FOLLOWING_ENTRIES: &str = "following-entries";
/// Derived files (covers, thumbnails); not retained.
pub const HOMESTATION_BUCKET_CACHE: &str = "cache";
/// Uploads and captures not yet published, key `uploads/<id>/<name>`, `articles/<id>.md`,
/// `snapshots/<capture obj id>.html`.
pub const HOMESTATION_BUCKET_SCRATCH: &str = "scratch";
pub const HOMESTATION_SCRATCH_TTL_DAYS: u32 = 7;
/// The `scratch` pool of each user's HomeStation.
pub const HOMESTATION_STORAGE_QUOTA_MIB: u64 = 4096;

pub fn homestation_bucket_decls() -> HashMap<String, BucketDecl> {
    let decl = |kind, data_policy| BucketDecl {
        kind,
        data_policy,
        scope: BucketScope::User,
        versioning: None,
        quota_mib: None,
        max_objects: None,
        default_ttl_days: None,
        lifecycle: Vec::new(),
    };
    // Keys name one version each, so documents need no bucket versioning: a withdrawal unbinds.
    let unversioned = |kind| BucketDecl { versioning: Some(false), ..decl(kind, DataPolicy::Documents) };
    HashMap::from([
        (HOMESTATION_BUCKET_PUBLISHED.to_string(), unversioned(BucketKind::Data)),
        (HOMESTATION_BUCKET_ENTRIES.to_string(), unversioned(BucketKind::Object)),
        (HOMESTATION_BUCKET_FOLLOWING.to_string(), decl(BucketKind::Data, DataPolicy::Ambient)),
        (HOMESTATION_BUCKET_FOLLOWING_ENTRIES.to_string(), decl(BucketKind::Object, DataPolicy::Ambient)),
        (HOMESTATION_BUCKET_CACHE.to_string(), BucketDecl { quota_mib: Some(1024), ..decl(BucketKind::Data, DataPolicy::Cache) }),
        (
            HOMESTATION_BUCKET_SCRATCH.to_string(),
            BucketDecl { default_ttl_days: Some(HOMESTATION_SCRATCH_TTL_DAYS), ..decl(BucketKind::Data, DataPolicy::Temp) },
        ),
    ])
}

pub fn homestation_default_rdb_instance_config() -> RdbInstanceConfig {
    RdbInstanceConfig {
        backend: RdbBackend::Sqlite,
        version: HOMESTATION_RDB_SCHEMA_VERSION,
        schema: HashMap::from([(RdbBackend::Sqlite, HOMESTATION_RDB_SCHEMA_SQLITE.to_string())]),
        connection: String::new(),
        partitions: vec![RdbPartition::UserData],
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct HomeStationSettings {
    /// Accept and index public submissions from anyone (collector role).
    pub collector: bool,
    /// Attach the `candidate` / `preferred` admission disclosure to `accepted`.
    pub disclose_admission: bool,
    /// Zone hostname → HTTP origin, for zones not reachable at `https://<zone>`.
    pub peers: HashMap<String, String>,
    /// Logical AICC model for the `model` evaluation profile; none disables it.
    pub evaluation_model: Option<String>,
    pub spider: bool,
    pub reading_window: usize,
    /// What `www.<zone>/` and `homestation.<zone>/` open: a username, or `~zone` for the zone
    /// feed. None = the zone owner's feed.
    pub default_feed: Option<String>,
    /// Usernames that may list posts in the zone feed (`*` = all users). None = regular users
    /// (admin / root / user, not limited or guest).
    pub zone_feed_writers: Option<Vec<String>>,
    /// Display name of the zone feed; none = the zone hostname.
    pub zone_name: Option<String>,
}

impl Default for HomeStationSettings {
    fn default() -> Self {
        Self {
            collector: false,
            disclose_admission: true,
            peers: HashMap::new(),
            evaluation_model: None,
            spider: true,
            reading_window: 300,
            default_feed: None,
            zone_feed_writers: None,
            zone_name: None,
        }
    }
}

pub struct HomeStationClient {
    client: kRPC,
}

impl HomeStationClient {
    pub fn new(client: kRPC) -> Self {
        Self { client }
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value, ::kRPC::RPCErrors> {
        self.client.call(method, params).await
    }
}

/// Every user table is keyed by `owner` (the zone username) first; `zone_feed` and
/// `zone_changes` are the zone's own.
pub const HOMESTATION_RDB_SCHEMA_SQLITE: &str = r#"
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);

CREATE TABLE IF NOT EXISTS settings (owner TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL, PRIMARY KEY (owner, key));

CREATE TABLE IF NOT EXISTS import_metadata (owner TEXT NOT NULL, obj_id TEXT NOT NULL, metadata TEXT NOT NULL, PRIMARY KEY (owner, obj_id));

CREATE TABLE IF NOT EXISTS objects (
  owner TEXT NOT NULL,
  obj_id TEXT NOT NULL,
  obj_type TEXT NOT NULL,
  body TEXT NOT NULL,
  jwt TEXT,
  signer TEXT,
  publisher TEXT,
  verified INTEGER NOT NULL DEFAULT 0,
  local_only INTEGER NOT NULL DEFAULT 0,
  origin TEXT,
  received_at INTEGER NOT NULL,
  PRIMARY KEY (owner, obj_id)
);
CREATE INDEX IF NOT EXISTS objects_type ON objects(owner, obj_type);

CREATE TABLE IF NOT EXISTS feed_index (
  owner TEXT NOT NULL,
  obj_id TEXT NOT NULL,
  publisher TEXT NOT NULL,
  kind TEXT NOT NULL,
  comment_type TEXT,
  content_type TEXT,
  entry TEXT,
  entry_valid INTEGER NOT NULL DEFAULT 0,
  target TEXT,
  wraps TEXT,
  iat INTEGER NOT NULL,
  category TEXT,
  text TEXT NOT NULL DEFAULT '',
  tags TEXT NOT NULL DEFAULT '[]',
  PRIMARY KEY (owner, obj_id)
);
CREATE INDEX IF NOT EXISTS feed_index_target ON feed_index(owner, target);
CREATE INDEX IF NOT EXISTS feed_index_entry ON feed_index(owner, entry);
CREATE INDEX IF NOT EXISTS feed_index_publisher ON feed_index(owner, publisher, iat);
CREATE INDEX IF NOT EXISTS feed_index_wraps ON feed_index(owner, wraps);

CREATE TABLE IF NOT EXISTS heads (
  owner TEXT NOT NULL,
  entry TEXT NOT NULL,
  publisher TEXT NOT NULL,
  seq INTEGER NOT NULL,
  state TEXT NOT NULL,
  current TEXT,
  head_obj_id TEXT NOT NULL,
  conflict_obj_id TEXT,
  restricted INTEGER NOT NULL DEFAULT 0,
  tier TEXT,
  updated_at_ms INTEGER NOT NULL,
  verified_at INTEGER NOT NULL,
  PRIMARY KEY (owner, entry)
);

CREATE TABLE IF NOT EXISTS entry_versions (
  owner TEXT NOT NULL,
  entry TEXT NOT NULL,
  obj_id TEXT NOT NULL,
  first_seen INTEGER NOT NULL,
  PRIMARY KEY (owner, entry, obj_id)
);

CREATE TABLE IF NOT EXISTS published (
  owner TEXT NOT NULL,
  entry TEXT NOT NULL,
  key TEXT NOT NULL,
  namespace TEXT NOT NULL,
  kind TEXT NOT NULL,
  audience TEXT NOT NULL,
  category TEXT,
  target TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (owner, entry)
);
CREATE INDEX IF NOT EXISTS published_target ON published(owner, target);

CREATE TABLE IF NOT EXISTS head_history (
  owner TEXT NOT NULL,
  entry TEXT NOT NULL,
  seq INTEGER NOT NULL,
  head_obj_id TEXT NOT NULL,
  state TEXT NOT NULL,
  current TEXT,
  at INTEGER NOT NULL,
  PRIMARY KEY (owner, entry, seq)
);

CREATE TABLE IF NOT EXISTS stream_changes (
  cursor INTEGER PRIMARY KEY AUTOINCREMENT,
  owner TEXT NOT NULL,
  entry TEXT NOT NULL,
  kind TEXT NOT NULL,
  seq INTEGER NOT NULL,
  at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS stream_changes_owner ON stream_changes(owner, cursor);
CREATE INDEX IF NOT EXISTS stream_changes_entry ON stream_changes(owner, entry);

CREATE TABLE IF NOT EXISTS object_grants (
  owner TEXT NOT NULL,
  obj_id TEXT NOT NULL,
  entry TEXT NOT NULL,
  PRIMARY KEY (owner, obj_id, entry)
);

CREATE TABLE IF NOT EXISTS publish_tasks (
  owner TEXT NOT NULL,
  key TEXT NOT NULL,
  stage TEXT NOT NULL,
  entry TEXT,
  obj_id TEXT,
  input TEXT NOT NULL,
  error TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (owner, key)
);

CREATE TABLE IF NOT EXISTS outbox (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  owner TEXT NOT NULL,
  obj_id TEXT NOT NULL,
  recipient TEXT NOT NULL,
  target TEXT,
  entry TEXT,
  task_key TEXT,
  purpose TEXT NOT NULL,
  restricted INTEGER NOT NULL DEFAULT 0,
  state TEXT NOT NULL,
  admission TEXT,
  attempts INTEGER NOT NULL DEFAULT 0,
  next_at INTEGER NOT NULL,
  last_result TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  UNIQUE (owner, obj_id, recipient)
);
CREATE INDEX IF NOT EXISTS outbox_due ON outbox(owner, state, next_at);
CREATE INDEX IF NOT EXISTS outbox_entry ON outbox(owner, entry);

CREATE TABLE IF NOT EXISTS inbox_receipts (
  owner TEXT NOT NULL,
  obj_id TEXT NOT NULL,
  sender TEXT NOT NULL,
  admission TEXT,
  received_at INTEGER NOT NULL,
  PRIMARY KEY (owner, obj_id)
);
CREATE INDEX IF NOT EXISTS inbox_receipts_sender ON inbox_receipts(owner, sender, received_at);

CREATE TABLE IF NOT EXISTS sources (
  owner TEXT NOT NULL,
  id TEXT NOT NULL,
  kind TEXT NOT NULL,
  did TEXT,
  url TEXT,
  name TEXT NOT NULL,
  description TEXT,
  basis TEXT NOT NULL DEFAULT '[]',
  paused INTEGER NOT NULL DEFAULT 0,
  intent_id TEXT,
  notify TEXT NOT NULL,
  follow_entry TEXT,
  cursor INTEGER,
  last_success_at INTEGER,
  last_error TEXT,
  last_fetch_at INTEGER,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (owner, id)
);
CREATE UNIQUE INDEX IF NOT EXISTS sources_did ON sources(owner, did) WHERE did IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS sources_url ON sources(owner, url) WHERE url IS NOT NULL;

CREATE TABLE IF NOT EXISTS intents (
  owner TEXT NOT NULL,
  id TEXT NOT NULL,
  text TEXT NOT NULL,
  status TEXT NOT NULL,
  source_ids TEXT NOT NULL DEFAULT '[]',
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (owner, id)
);

CREATE TABLE IF NOT EXISTS followers (
  owner TEXT NOT NULL,
  did TEXT NOT NULL,
  entry TEXT NOT NULL,
  state TEXT NOT NULL,
  seq INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (owner, did)
);

CREATE TABLE IF NOT EXISTS candidates (
  owner TEXT NOT NULL,
  obj_id TEXT NOT NULL,
  publisher TEXT NOT NULL,
  selection TEXT NOT NULL,
  reason TEXT,
  rule_ids TEXT NOT NULL DEFAULT '[]',
  source_paths TEXT NOT NULL DEFAULT '[]',
  private_capture INTEGER NOT NULL DEFAULT 0,
  arrived_at INTEGER NOT NULL,
  score REAL,
  resources TEXT NOT NULL DEFAULT 'unknown',
  replaced_by TEXT,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (owner, obj_id)
);
CREATE INDEX IF NOT EXISTS candidates_selection ON candidates(owner, selection, arrived_at);

CREATE TABLE IF NOT EXISTS admission_history (
  owner TEXT NOT NULL,
  obj_id TEXT NOT NULL,
  first_admitted_at INTEGER,
  opened_at INTEGER,
  PRIMARY KEY (owner, obj_id)
);

CREATE TABLE IF NOT EXISTS reading (
  owner TEXT NOT NULL,
  obj_id TEXT NOT NULL,
  admitted_at INTEGER NOT NULL,
  order_key INTEGER NOT NULL,
  reason TEXT NOT NULL,
  resources TEXT NOT NULL,
  topics TEXT NOT NULL DEFAULT '[]',
  PRIMARY KEY (owner, obj_id)
);
CREATE INDEX IF NOT EXISTS reading_order ON reading(owner, order_key);

CREATE TABLE IF NOT EXISTS view_cursors (
  owner TEXT NOT NULL,
  view_key TEXT NOT NULL,
  anchor TEXT NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (owner, view_key)
);

CREATE TABLE IF NOT EXISTS personal (
  owner TEXT NOT NULL,
  obj_id TEXT NOT NULL,
  like_entry TEXT,
  repost_entry TEXT,
  bookmark_on INTEGER NOT NULL DEFAULT 0,
  bookmark_private TEXT,
  bookmark_entry TEXT,
  saved_at INTEGER,
  read_later INTEGER NOT NULL DEFAULT 0,
  read_later_at INTEGER,
  dislike INTEGER NOT NULL DEFAULT 0,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (owner, obj_id)
);

CREATE TABLE IF NOT EXISTS comment_sources (
  owner TEXT NOT NULL,
  comment_id TEXT NOT NULL,
  source_kind TEXT NOT NULL,
  source TEXT NOT NULL,
  seen_at INTEGER NOT NULL,
  PRIMARY KEY (owner, comment_id, source_kind, source)
);

CREATE TABLE IF NOT EXISTS author_list (
  owner TEXT NOT NULL,
  target TEXT NOT NULL,
  comment_id TEXT NOT NULL,
  listed INTEGER NOT NULL,
  position INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (owner, target, comment_id)
);

CREATE TABLE IF NOT EXISTS tracked (
  owner TEXT NOT NULL,
  target TEXT NOT NULL,
  author TEXT NOT NULL,
  reason TEXT NOT NULL,
  paused INTEGER NOT NULL DEFAULT 0,
  last_sync_at INTEGER,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (owner, target)
);

CREATE TABLE IF NOT EXISTS participants (
  owner TEXT NOT NULL,
  target TEXT NOT NULL,
  did TEXT NOT NULL,
  seen_at INTEGER NOT NULL,
  PRIMARY KEY (owner, target, did)
);

CREATE TABLE IF NOT EXISTS view_snapshots (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  owner TEXT NOT NULL,
  view TEXT NOT NULL,
  target TEXT NOT NULL,
  complete INTEGER NOT NULL,
  comment_ids TEXT NOT NULL,
  at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS view_snapshots_target ON view_snapshots(owner, view, target, at);

CREATE TABLE IF NOT EXISTS eval_records (
  owner TEXT NOT NULL,
  result_id TEXT NOT NULL,
  target_key TEXT NOT NULL,
  target_kind TEXT NOT NULL,
  profile_id TEXT NOT NULL,
  profile_revision TEXT NOT NULL,
  dimensions TEXT NOT NULL,
  scope TEXT NOT NULL,
  state TEXT NOT NULL,
  iat INTEGER NOT NULL,
  refresh_after INTEGER,
  result TEXT NOT NULL,
  superseded_by TEXT,
  PRIMARY KEY (owner, result_id)
);
CREATE INDEX IF NOT EXISTS eval_records_target ON eval_records(owner, target_key, profile_id);

CREATE TABLE IF NOT EXISTS eval_tasks (
  owner TEXT NOT NULL,
  task_id TEXT NOT NULL,
  request TEXT NOT NULL,
  scope TEXT NOT NULL,
  state TEXT NOT NULL,
  result_ids TEXT NOT NULL DEFAULT '[]',
  error TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (owner, task_id)
);

CREATE TABLE IF NOT EXISTS tag_overrides (
  owner TEXT NOT NULL,
  id TEXT NOT NULL,
  target_key TEXT NOT NULL,
  dimension TEXT NOT NULL,
  tag TEXT NOT NULL,
  action TEXT NOT NULL,
  replacement TEXT,
  scope TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  revoked_at INTEGER,
  PRIMARY KEY (owner, id)
);
CREATE INDEX IF NOT EXISTS tag_overrides_target ON tag_overrides(owner, target_key);

CREATE TABLE IF NOT EXISTS eval_changes (
  cursor INTEGER PRIMARY KEY AUTOINCREMENT,
  owner TEXT NOT NULL,
  target_key TEXT NOT NULL,
  result_id TEXT,
  kind TEXT NOT NULL,
  scope TEXT NOT NULL,
  at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS eval_changes_owner ON eval_changes(owner, cursor);

CREATE TABLE IF NOT EXISTS behavior_events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  owner TEXT NOT NULL,
  obj_id TEXT NOT NULL,
  event TEXT NOT NULL,
  value INTEGER,
  at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS behavior_events_at ON behavior_events(owner, at);

CREATE TABLE IF NOT EXISTS agreements (
  owner TEXT NOT NULL,
  id TEXT NOT NULL,
  target TEXT NOT NULL,
  receiver TEXT NOT NULL,
  action TEXT NOT NULL,
  terms TEXT,
  joined INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (owner, id)
);

CREATE TABLE IF NOT EXISTS consumption_proofs (
  owner TEXT NOT NULL,
  obj_id TEXT NOT NULL,
  agreement_id TEXT NOT NULL,
  target TEXT NOT NULL,
  receiver TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (owner, obj_id)
);

CREATE TABLE IF NOT EXISTS capture_shares (
  owner TEXT NOT NULL,
  capture_id TEXT NOT NULL,
  shared_id TEXT NOT NULL,
  entry TEXT NOT NULL,
  PRIMARY KEY (owner, capture_id)
);

CREATE TABLE IF NOT EXISTS collector_index (
  owner TEXT NOT NULL,
  obj_id TEXT NOT NULL,
  submitted_by TEXT NOT NULL,
  received_at INTEGER NOT NULL,
  PRIMARY KEY (owner, obj_id)
);
CREATE INDEX IF NOT EXISTS collector_index_at ON collector_index(owner, received_at);

CREATE TABLE IF NOT EXISTS zone_feed (
  entry TEXT PRIMARY KEY,
  user TEXT NOT NULL,
  publisher TEXT NOT NULL,
  iat INTEGER NOT NULL,
  listed INTEGER NOT NULL DEFAULT 1,
  listed_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS zone_feed_order ON zone_feed(listed, iat, entry);

CREATE TABLE IF NOT EXISTS zone_changes (
  cursor INTEGER PRIMARY KEY AUTOINCREMENT,
  entry TEXT NOT NULL,
  user TEXT NOT NULL,
  kind TEXT NOT NULL,
  at INTEGER NOT NULL
);
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn homestation_declarations_follow_the_tier_rules() {
        let buckets = homestation_bucket_decls();
        assert_eq!(crate::validate_app_buckets(&buckets, Some(HOMESTATION_STORAGE_QUOTA_MIB)), Vec::<String>::new());
        assert!(!crate::validate_app_buckets(&buckets, None).is_empty(), "scratch needs the app pool quota");
        crate::validate_rdb_instance_config(&homestation_default_rdb_instance_config(), HOMESTATION_SERVICE_NAME, HOMESTATION_RDB_INSTANCE_ID, false)
            .unwrap();
    }
}
