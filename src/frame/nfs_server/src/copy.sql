CREATE TABLE IF NOT EXISTS copy_job(task TEXT PRIMARY KEY, input TEXT NOT NULL, rules TEXT NOT NULL DEFAULT '{}');
            CREATE TABLE IF NOT EXISTS copy_item(task TEXT NOT NULL, id INTEGER NOT NULL, parent INTEGER, state TEXT NOT NULL, payload TEXT NOT NULL, PRIMARY KEY(task,id));
            CREATE INDEX IF NOT EXISTS copy_state ON copy_item(task,state,id);
