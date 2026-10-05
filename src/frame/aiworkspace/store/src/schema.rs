//! Storage DDL (design doc §4.2, §2.11, §7). `storage_schema_version = 1`.

pub const STORAGE_SCHEMA_VERSION: u32 = 1;

/// The document database: the working form of one portable Workspace.
pub const DOC_DDL: &str = r#"
CREATE TABLE workspace_meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
) WITHOUT ROWID;

CREATE TABLE entities (
  entity_id      TEXT PRIMARY KEY,
  type_id        TEXT NOT NULL,
  schema_version INTEGER NOT NULL,
  scope          TEXT NOT NULL DEFAULT 'shared',
  name           TEXT,
  write_policy   TEXT NOT NULL DEFAULT 'open',
  payload_json   TEXT NOT NULL DEFAULT '{}',
  key_revs_json  TEXT NOT NULL DEFAULT '{}',
  created_seq    INTEGER NOT NULL,
  meta_rev       INTEGER NOT NULL,
  content_rev    INTEGER NOT NULL,
  life_rev       INTEGER NOT NULL,
  deleted_seq    INTEGER
) WITHOUT ROWID;

CREATE TABLE tree_edges (
  child_id       TEXT PRIMARY KEY REFERENCES entities(entity_id),
  parent_id      TEXT NOT NULL REFERENCES entities(entity_id),
  order_key      TEXT NOT NULL,
  placement_json TEXT,
  struct_rev     INTEGER NOT NULL
) WITHOUT ROWID;
CREATE INDEX tree_edges_parent ON tree_edges(parent_id, order_key, child_id);

CREATE TABLE refs (
  src_entity_id    TEXT NOT NULL REFERENCES entities(entity_id),
  src_selector     TEXT NOT NULL DEFAULT '',
  kind             TEXT NOT NULL,
  dst_workspace_id TEXT NOT NULL DEFAULT '',
  dst_entity_id    TEXT NOT NULL DEFAULT '',
  dst_object_id    TEXT NOT NULL DEFAULT '',
  dst_query_json   TEXT NOT NULL DEFAULT '',
  PRIMARY KEY (src_entity_id, src_selector, kind, dst_workspace_id, dst_entity_id, dst_object_id, dst_query_json)
) WITHOUT ROWID;
CREATE INDEX refs_dst_entity ON refs(dst_workspace_id, dst_entity_id);
CREATE INDEX refs_dst_object ON refs(dst_object_id);

CREATE TABLE table_fields (
  source_id   TEXT NOT NULL REFERENCES entities(entity_id),
  field_id    TEXT NOT NULL,
  def_json    TEXT NOT NULL,
  order_key   TEXT NOT NULL,
  def_rev     INTEGER NOT NULL,
  type_rev    INTEGER NOT NULL,
  values_rev  INTEGER NOT NULL DEFAULT 0,
  deleted_seq INTEGER,
  PRIMARY KEY (source_id, field_id)
) WITHOUT ROWID;

CREATE TABLE table_records (
  source_id   TEXT NOT NULL REFERENCES entities(entity_id),
  record_id   TEXT NOT NULL,
  values_json TEXT NOT NULL DEFAULT '{}',
  revs_json   TEXT NOT NULL DEFAULT '{}',
  meta_json   TEXT NOT NULL DEFAULT '{}',
  body_entity_id TEXT REFERENCES entities(entity_id),
  created_seq INTEGER NOT NULL,
  rev         INTEGER NOT NULL,
  deleted_seq INTEGER,
  PRIMARY KEY (source_id, record_id)
) WITHOUT ROWID;

CREATE TABLE richtext_states (
  entity_id      TEXT PRIMARY KEY REFERENCES entities(entity_id),
  lineage_id     TEXT NOT NULL,
  engine         TEXT NOT NULL,
  engine_version TEXT NOT NULL,
  encoding       TEXT NOT NULL,
  snapshot       BLOB NOT NULL,
  snapshot_seq   INTEGER NOT NULL,
  ast_json       TEXT NOT NULL,
  block_index_json TEXT NOT NULL
) WITHOUT ROWID;

CREATE TABLE richtext_updates (
  entity_id TEXT NOT NULL REFERENCES entities(entity_id),
  seq       INTEGER NOT NULL,
  idx       INTEGER NOT NULL,
  update_bytes BLOB NOT NULL,
  PRIMARY KEY (entity_id, seq, idx)
) WITHOUT ROWID;

CREATE TABLE commits (
  seq            INTEGER PRIMARY KEY,
  commit_id      TEXT NOT NULL UNIQUE,
  principal      TEXT NOT NULL,
  app_id         TEXT,
  session_id     TEXT,
  origin         TEXT NOT NULL,
  run_id         TEXT,
  undo_group     TEXT,
  undoes         TEXT,
  idem_key       TEXT NOT NULL,
  request_digest TEXT NOT NULL,
  message        TEXT,
  accepted_at    TEXT NOT NULL,
  result_json    TEXT NOT NULL,
  UNIQUE (principal, idem_key)
);

CREATE TABLE commit_ops (
  seq          INTEGER NOT NULL REFERENCES commits(seq),
  op_index     INTEGER NOT NULL,
  op_json      TEXT NOT NULL,
  inverse_json TEXT,
  touched_json TEXT NOT NULL,
  PRIMARY KEY (seq, op_index)
) WITHOUT ROWID;

CREATE TABLE assets (
  object_id   TEXT PRIMARY KEY,
  media_type  TEXT NOT NULL,
  size        INTEGER NOT NULL,
  first_seq   INTEGER NOT NULL
) WITHOUT ROWID;

CREATE TABLE entity_versions (
  entity_id   TEXT NOT NULL,
  content_rev INTEGER NOT NULL,
  object_id   TEXT NOT NULL,
  PRIMARY KEY (entity_id, content_rev)
) WITHOUT ROWID;
CREATE INDEX entity_versions_object ON entity_versions(object_id);

CREATE TABLE snapshots (
  snapshot_id  TEXT PRIMARY KEY,
  content_root TEXT NOT NULL,
  seq          INTEGER NOT NULL,
  kind         TEXT NOT NULL,
  retained     INTEGER NOT NULL DEFAULT 1,
  created_at   TEXT NOT NULL,
  created_by   TEXT NOT NULL
) WITHOUT ROWID;
"#;

/// Tables of the document database that a replica may receive, in creation
/// order. The replica is built from an empty database by copying allowed rows —
/// never by copying the file and deleting from it.
pub const REPLICA_TABLES: &[&str] = &[
    "workspace_meta",
    "entities",
    "tree_edges",
    "refs",
    "table_fields",
    "table_records",
    "richtext_states",
    "richtext_updates",
    "assets",
];

/// Deployment-local state: grants, locks, runs, staged uploads. Never exported.
pub const LOCAL_DDL: &str = r#"
CREATE TABLE local_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL) WITHOUT ROWID;

CREATE TABLE grants (
  subject         TEXT NOT NULL,
  scope_entity_id TEXT NOT NULL DEFAULT '',
  capabilities    INTEGER NOT NULL,
  PRIMARY KEY (subject, scope_entity_id)
) WITHOUT ROWID;

CREATE TABLE locks (
  entity_id   TEXT PRIMARY KEY,
  lock_id     TEXT NOT NULL UNIQUE,
  principal   TEXT NOT NULL,
  session_id  TEXT NOT NULL,
  acquired_at TEXT NOT NULL,
  expires_at  TEXT NOT NULL,
  last_write_at TEXT
) WITHOUT ROWID;
CREATE TABLE lock_events (
  id INTEGER PRIMARY KEY AUTOINCREMENT, entity_id TEXT NOT NULL, event TEXT NOT NULL,
  principal TEXT NOT NULL, session_id TEXT, by_principal TEXT, at TEXT NOT NULL
);

CREATE TABLE runs (
  run_id         TEXT PRIMARY KEY,
  program        TEXT NOT NULL,
  principal      TEXT NOT NULL,
  idem_key       TEXT NOT NULL,
  state          TEXT NOT NULL,
  params_json    TEXT NOT NULL,
  inputs_json    TEXT NOT NULL DEFAULT '[]',
  candidate_json TEXT,
  warnings_json  TEXT NOT NULL DEFAULT '[]',
  result_json    TEXT,
  created_at     TEXT NOT NULL,
  updated_at     TEXT NOT NULL,
  UNIQUE (principal, idem_key)
) WITHOUT ROWID;

CREATE TABLE staged_assets (
  object_id  TEXT PRIMARY KEY,
  media_type TEXT NOT NULL,
  size       INTEGER NOT NULL,
  principal  TEXT NOT NULL,
  expires_at TEXT NOT NULL
) WITHOUT ROWID;

CREATE TABLE upload_sessions (
  upload_id  TEXT PRIMARY KEY,
  kind       TEXT NOT NULL,
  principal  TEXT NOT NULL,
  size       INTEGER NOT NULL,
  file_name  TEXT,
  state      TEXT NOT NULL,
  created_at TEXT NOT NULL
) WITHOUT ROWID;
"#;
