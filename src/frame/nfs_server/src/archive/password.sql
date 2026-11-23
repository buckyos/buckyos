CREATE TABLE IF NOT EXISTS archive_passwords(
  user_id TEXT NOT NULL,
  archive_sha256 TEXT NOT NULL,
  password TEXT NOT NULL,
  verified_at INTEGER NOT NULL,
  PRIMARY KEY(user_id, archive_sha256, password)
);
