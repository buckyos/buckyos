CREATE TABLE IF NOT EXISTS families (
  family_key TEXT PRIMARY KEY,
  subject TEXT NOT NULL,
  principal_kind TEXT NOT NULL,
  target TEXT NOT NULL,
  session_id TEXT NOT NULL,
  claims TEXT NOT NULL,
  source TEXT,
  parent_key TEXT,
  absolute_exp INTEGER,
  created_at INTEGER NOT NULL,
  refresh_jti TEXT NOT NULL,
  refresh_exp INTEGER NOT NULL,
  session_jti TEXT NOT NULL,
  session_exp INTEGER NOT NULL,
  prev_refresh_jti TEXT,
  prev_rotated_at INTEGER
);
CREATE INDEX IF NOT EXISTS families_subject ON families(subject);
CREATE INDEX IF NOT EXISTS families_parent ON families(parent_key);
