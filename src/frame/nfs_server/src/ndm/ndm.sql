CREATE TABLE IF NOT EXISTS ndm_uploads(
  session_id TEXT PRIMARY KEY,
  principal TEXT NOT NULL,
  upload_key TEXT NOT NULL,
  canonical_upload_id TEXT NOT NULL,
  logical_path TEXT NOT NULL,
  file_hash TEXT,
  chunk_index INTEGER NOT NULL,
  chunk_hash TEXT,
  chunk_size INTEGER NOT NULL,
  offset INTEGER NOT NULL,
  status TEXT NOT NULL,
  chunk_obj_id TEXT,
  raw_metadata TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_ndm_uploads_key ON ndm_uploads(upload_key);
CREATE INDEX IF NOT EXISTS idx_ndm_uploads_principal ON ndm_uploads(principal, updated_at);
CREATE TABLE IF NOT EXISTS ndm_holdings(
  principal TEXT NOT NULL,
  obj_id TEXT NOT NULL,
  how TEXT NOT NULL,
  at INTEGER NOT NULL,
  PRIMARY KEY(principal, obj_id)
);
CREATE TABLE IF NOT EXISTS ndm_import_qcids(
  principal TEXT NOT NULL,
  qcid TEXT NOT NULL,
  content_id TEXT NOT NULL,
  size INTEGER NOT NULL,
  PRIMARY KEY(principal, qcid)
);
