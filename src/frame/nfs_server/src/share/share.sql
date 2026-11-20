CREATE TABLE IF NOT EXISTS share_operations(
  operation_id TEXT PRIMARY KEY,
  intent TEXT NOT NULL,
  owner_user_id TEXT NOT NULL,
  actor_user_id TEXT NOT NULL,
  actor_app_id TEXT,
  input_json TEXT NOT NULL,
  idempotency_key TEXT NOT NULL,
  task_id TEXT,
  share_id TEXT,
  state TEXT NOT NULL,
  error_json TEXT,
  progress_json TEXT,
  result_json TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  UNIQUE(owner_user_id, idempotency_key)
);
CREATE TABLE IF NOT EXISTS share_versions(
  obj_id TEXT PRIMARY KEY,
  obj_type TEXT NOT NULL,
  root_name TEXT NOT NULL,
  mime TEXT,
  total_size INTEGER NOT NULL,
  item_count INTEGER NOT NULL,
  source_path TEXT NOT NULL,
  source_mtime INTEGER,
  source_size INTEGER,
  excluded_json TEXT,
  prepared_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS shares(
  share_id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  owner_user_id TEXT NOT NULL,
  created_by_app TEXT,
  title TEXT NOT NULL,
  obj_id TEXT NOT NULL REFERENCES share_versions(obj_id),
  source_path TEXT NOT NULL,
  cap_id TEXT,
  related_share_id TEXT,
  status TEXT NOT NULL,
  status_reason TEXT,
  hidden INTEGER NOT NULL DEFAULT 0,
  revision INTEGER NOT NULL DEFAULT 1,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  stopped_at INTEGER
);
CREATE INDEX IF NOT EXISTS idx_shares_owner ON shares(owner_user_id, status);
CREATE INDEX IF NOT EXISTS idx_shares_source ON shares(source_path);
CREATE TABLE IF NOT EXISTS share_link_policy(
  share_id TEXT PRIMARY KEY REFERENCES shares(share_id),
  access TEXT NOT NULL,
  password_hash TEXT,
  password_rev INTEGER NOT NULL DEFAULT 0,
  allow_download INTEGER NOT NULL DEFAULT 1
);
CREATE TABLE IF NOT EXISTS share_deliveries(
  share_id TEXT PRIMARY KEY REFERENCES shares(share_id),
  target_app TEXT NOT NULL,
  handoff_at INTEGER NOT NULL,
  delivery_state TEXT NOT NULL,
  business_ref_json TEXT,
  reported_at INTEGER,
  error TEXT
);
CREATE TABLE IF NOT EXISTS share_audit(
  audit_id INTEGER PRIMARY KEY AUTOINCREMENT,
  share_id TEXT NOT NULL,
  action TEXT NOT NULL,
  revision_before INTEGER,
  revision_after INTEGER,
  actor_user_id TEXT NOT NULL,
  actor_app_id TEXT,
  source TEXT NOT NULL,
  reason TEXT,
  outcome TEXT NOT NULL,
  at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_share_audit ON share_audit(share_id, audit_id);
CREATE TABLE IF NOT EXISTS share_access_log(
  log_id INTEGER PRIMARY KEY AUTOINCREMENT,
  share_id TEXT NOT NULL,
  at INTEGER NOT NULL,
  kind TEXT NOT NULL,
  status INTEGER NOT NULL,
  bytes INTEGER NOT NULL DEFAULT 0,
  visitor TEXT,
  inner_path TEXT
);
CREATE INDEX IF NOT EXISTS idx_access_share_at ON share_access_log(share_id, at);
CREATE INDEX IF NOT EXISTS idx_access_visitor ON share_access_log(share_id, visitor, at);
CREATE TABLE IF NOT EXISTS share_access_stats(
  share_id TEXT NOT NULL,
  bucket INTEGER NOT NULL,
  views INTEGER NOT NULL DEFAULT 0,
  downloads INTEGER NOT NULL DEFAULT 0,
  bytes INTEGER NOT NULL DEFAULT 0,
  visitors INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY(share_id, bucket)
);
CREATE TABLE IF NOT EXISTS public_observations(
  rel_path TEXT PRIMARY KEY,
  entry_kind TEXT NOT NULL,
  link_target TEXT,
  link_ok INTEGER,
  size INTEGER,
  mtime INTEGER,
  first_seen_at INTEGER NOT NULL,
  last_seen_at INTEGER NOT NULL,
  missing_since INTEGER
);
