CREATE TABLE IF NOT EXISTS user_prefs (
  principal   TEXT PRIMARY KEY,
  prefs_json  TEXT NOT NULL,
  revision    INTEGER NOT NULL,
  updated_ms  INTEGER NOT NULL
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS user_opened (
  principal    TEXT NOT NULL,
  workspace_id TEXT NOT NULL,
  opened_ms    INTEGER NOT NULL,
  PRIMARY KEY (principal, workspace_id)
) WITHOUT ROWID;
