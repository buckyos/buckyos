CREATE TABLE IF NOT EXISTS cache_index (
                 cache_key TEXT PRIMARY KEY,
                 result_obj_id TEXT NOT NULL,
                 artifact_refs TEXT NOT NULL,
                 fingerprint TEXT NOT NULL,
                 expires_at INTEGER,
                 created_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS service_epoch (
                 id INTEGER PRIMARY KEY CHECK (id = 1),
                 epoch INTEGER NOT NULL,
                 started_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS active_executions (
                 execution_id TEXT PRIMARY KEY,
                 run_id TEXT NOT NULL,
                 node_id TEXT NOT NULL,
                 epoch INTEGER NOT NULL,
                 temp_dir TEXT NOT NULL,
                 task_id TEXT,
                 started_at INTEGER NOT NULL
             );
