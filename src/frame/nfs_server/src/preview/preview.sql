CREATE TABLE IF NOT EXISTS preview_works(
  work_key TEXT PRIMARY KEY,
  state TEXT NOT NULL,
  attempt_id TEXT NOT NULL,
  input_obj_id TEXT NOT NULL,
  converter TEXT NOT NULL,
  params_json TEXT NOT NULL,
  params_hash TEXT NOT NULL,
  result_json TEXT,
  error_json TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  accessed_at INTEGER NOT NULL,
  expires_at INTEGER
);
CREATE INDEX IF NOT EXISTS preview_works_accessed ON preview_works(state, accessed_at);
CREATE TABLE IF NOT EXISTS preview_inputs(
  source TEXT PRIMARY KEY,
  size INTEGER NOT NULL,
  mtime INTEGER NOT NULL,
  obj_id TEXT NOT NULL,
  updated_at INTEGER NOT NULL
);
