CREATE TABLE IF NOT EXISTS kv(
  k TEXT PRIMARY KEY, v TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS entities(
  node_id INTEGER PRIMARY KEY AUTOINCREMENT,
  gen INTEGER NOT NULL DEFAULT 1,
  kind TEXT NOT NULL,
  owner TEXT,
  created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS anchors(
  node_id INTEGER PRIMARY KEY REFERENCES entities(node_id),
  root_id TEXT NOT NULL,
  ino INTEGER NOT NULL,
  dev INTEGER NOT NULL DEFAULT 0,
  path TEXT NOT NULL,
  size INTEGER,
  mtime INTEGER,
  qcid TEXT,
  full_hash TEXT,
  state TEXT NOT NULL DEFAULT 'ok'
);
CREATE INDEX IF NOT EXISTS idx_anchors_path ON anchors(root_id, path);
CREATE INDEX IF NOT EXISTS idx_anchors_ino ON anchors(root_id, ino);
CREATE TABLE IF NOT EXISTS namespace_bindings(
  entry_id INTEGER PRIMARY KEY AUTOINCREMENT,
  parent_node_id INTEGER NOT NULL REFERENCES entities(node_id),
  name TEXT NOT NULL,
  target_type TEXT NOT NULL,
  target_node_id INTEGER,
  target_obj_id TEXT,
  binding_type TEXT NOT NULL DEFAULT 'reference',
  state TEXT NOT NULL DEFAULT 'ok',
  created_at INTEGER NOT NULL,
  UNIQUE(parent_node_id, name)
);
CREATE TABLE IF NOT EXISTS views(
  node_id INTEGER PRIMARY KEY REFERENCES entities(node_id),
  view_id TEXT UNIQUE NOT NULL,
  origin TEXT NOT NULL DEFAULT 'auto',
  title TEXT NOT NULL,
  generation INTEGER NOT NULL DEFAULT 1,
  stale INTEGER NOT NULL DEFAULT 0,
  query_json TEXT
);
CREATE TABLE IF NOT EXISTS view_groups(
  node_id INTEGER PRIMARY KEY REFERENCES entities(node_id),
  view_node_id INTEGER NOT NULL REFERENCES views(node_id),
  label TEXT NOT NULL,
  by_dim TEXT,
  ord INTEGER NOT NULL DEFAULT 0,
  UNIQUE(view_node_id, label)
);
CREATE TABLE IF NOT EXISTS view_base(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  view_node_id INTEGER NOT NULL,
  group_node_id INTEGER,
  target_type TEXT NOT NULL,
  target_node_id INTEGER,
  target_obj_id TEXT,
  provenance_json TEXT,
  ord INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_view_base ON view_base(view_node_id);
CREATE TABLE IF NOT EXISTS view_patch(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  view_node_id INTEGER NOT NULL,
  op TEXT NOT NULL,
  target_type TEXT,
  target_node_id INTEGER,
  target_obj_id TEXT,
  base_id INTEGER,
  ord INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS collections(
  node_id INTEGER PRIMARY KEY REFERENCES entities(node_id),
  collection_id TEXT UNIQUE NOT NULL,
  title TEXT NOT NULL,
  description TEXT,
  generation INTEGER NOT NULL DEFAULT 1,
  owner TEXT
);
CREATE TABLE IF NOT EXISTS collection_nodes(
  entry_id INTEGER PRIMARY KEY AUTOINCREMENT,
  collection_node_id INTEGER NOT NULL REFERENCES collections(node_id),
  parent_group_node_id INTEGER,
  node_type TEXT NOT NULL,
  target_type TEXT,
  target_node_id INTEGER,
  target_obj_id TEXT,
  name TEXT,
  manual_order INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_coll_nodes ON collection_nodes(collection_node_id, parent_group_node_id);
CREATE TABLE IF NOT EXISTS meta_records(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  anchor TEXT NOT NULL,
  ns TEXT NOT NULL,
  key TEXT NOT NULL,
  value_json TEXT NOT NULL,
  source_json TEXT NOT NULL,
  confidence REAL,
  visibility TEXT NOT NULL DEFAULT 'private',
  updated_at INTEGER NOT NULL,
  UNIQUE(anchor, ns, key)
);
CREATE INDEX IF NOT EXISTS idx_meta_anchor ON meta_records(anchor);
CREATE TABLE IF NOT EXISTS grants(
  cap_id TEXT PRIMARY KEY,
  token_hash TEXT NOT NULL,
  subtree TEXT NOT NULL,
  ops_json TEXT NOT NULL,
  audience TEXT,
  expires_at INTEGER,
  max_uses INTEGER,
  revoked INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS acls(
  root_id TEXT NOT NULL,
  path TEXT NOT NULL,
  inherit INTEGER NOT NULL DEFAULT 1,
  entries_json TEXT NOT NULL,
  updated_by TEXT NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY(root_id, path)
);
CREATE TABLE IF NOT EXISTS content_index(
  hash TEXT PRIMARY KEY,
  root_id TEXT NOT NULL,
  path TEXT NOT NULL,
  size INTEGER NOT NULL,
  mtime INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
