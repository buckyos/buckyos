CREATE TABLE IF NOT EXISTS sessions (
  session_id          TEXT PRIMARY KEY,
  source_workspace_id TEXT NOT NULL,
  workspace_id        TEXT NOT NULL,
  path_id             TEXT NOT NULL,
  title               TEXT NOT NULL,
  owner               TEXT NOT NULL,
  presenter           TEXT NOT NULL,
  state               TEXT NOT NULL,
  access              TEXT NOT NULL DEFAULT 'open',
  posts               TEXT NOT NULL DEFAULT 'open',
  policy_json         TEXT NOT NULL,
  baseline_json       TEXT NOT NULL,
  archive_json        TEXT,
  controller          TEXT,
  created_ms          INTEGER NOT NULL,
  started_ms          INTEGER,
  ended_ms            INTEGER,
  saved_ms            INTEGER,
  expires_ms          INTEGER,
  deleted_ms          INTEGER
);
CREATE INDEX IF NOT EXISTS sessions_source ON sessions(source_workspace_id);
CREATE INDEX IF NOT EXISTS sessions_run ON sessions(workspace_id);
CREATE TABLE IF NOT EXISTS session_invites (
  invite_id   TEXT PRIMARY KEY,
  session_id  TEXT NOT NULL,
  kind        TEXT NOT NULL,
  token       TEXT NOT NULL UNIQUE,
  label       TEXT,
  config_json TEXT NOT NULL DEFAULT '{}',
  created_by  TEXT NOT NULL,
  created_ms  INTEGER NOT NULL,
  revoked_ms  INTEGER
);
CREATE INDEX IF NOT EXISTS session_invites_session ON session_invites(session_id);
CREATE TABLE IF NOT EXISTS session_participants (
  session_id   TEXT NOT NULL,
  principal    TEXT NOT NULL,
  class        TEXT NOT NULL,
  level        TEXT NOT NULL,
  name         TEXT NOT NULL,
  role         TEXT NOT NULL,
  via_invite   TEXT,
  joined_ms    INTEGER NOT NULL,
  last_seen_ms INTEGER NOT NULL,
  online       INTEGER NOT NULL DEFAULT 0,
  speak        INTEGER,
  speak_by     TEXT,
  speak_ms     INTEGER,
  speak_req_ms INTEGER,
  PRIMARY KEY (session_id, principal)
);
CREATE TABLE IF NOT EXISTS session_posts (
  post_id     TEXT PRIMARY KEY,
  session_id  TEXT NOT NULL,
  kind        TEXT NOT NULL,
  author      TEXT NOT NULL,
  author_name TEXT,
  body        TEXT,
  score       INTEGER,
  t_rel_ms    INTEGER,
  created_ms  INTEGER NOT NULL,
  deleted_ms  INTEGER
);
CREATE INDEX IF NOT EXISTS session_posts_session ON session_posts(session_id, created_ms);
