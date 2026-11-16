---
name: aiworkspace_sync
description: Build a data source of an AI Workspace that syncs from outside — when a wish's analysis failed with NEEDS_EXTERNAL_DATA, or the workspace owner asks for one — define it (meaning, fields, primary key, freshness), write the sync program that fetches the data into the owner's home as `aiws.sync.v1`, test it, run the first sync and check the source turned valid; repair a source whose last attempt failed or whose data expired.
requires: the buckyos command signed in as this Agent; Deno for the websdk script that writes the definition (or any client of the service's `doc.commit`); you work for the workspace's owner (only the owner — an Agent working for them included — sets or changes a data source; triggering a refresh needs only `run` on it); the owner's home is mounted where you run (agent container and host: `$BUCKYOS_ROOT/data/home/<owner>/`, the Zone path `/home/<owner>/`)
baseline: standard
commands: deno, jq
---
#### When to use
- A wish's analysis run failed with `NEEDS_EXTERNAL_DATA`: its record (`buckyos aiws run-get`, `record.error.missing_inputs[].wanted`) says what data it needs (`meaning`, `origin`, `freshness`, `fields`). Or the owner asks for a data source (a request from the Agent panel names the data, where from, how fresh, and the wish).
- A data source of a workspace shows a failed last attempt or `expired` (`doc.read { as: "meta" }` → `data.attempt`, `data.expired`): repair it.
- Who asked decides: build only for the workspace owner. When the run was started by someone else (`run-get` → `context.trigger`), do not build the source — tell them the owner has to ask for it (their panel's “Request a data source…” reaches the owner).

#### Before you start
- What a data source is, its states and the file format: the service README “Sources” (`src/frame/aiworkspace/README.md`), `schemas/sync-envelope.schema.json`. A table whose `source` is `{ kind: "zone", path: "/home/<owner>/…" }` is filled only by refreshes that read that file as the owner; nobody edits its rows (`SOURCE_READ_ONLY`).
- Until its first good sync the source is `pending`: wishes may bind it and code against its fields, they cannot run on it (`NEEDS_INPUT`), SQL answers `DATA_PENDING`. After it, every refresh that changed something writes a new data version (`meta.data.version`: `business_time`, `synced_at`, `program`, `records`).
- Credentials never go into the definition, the workspace, the output file, the program's output or the conversation. A source that needs a credential (an API key, a login) cannot be built yet: tell the owner and stop.

#### Shortest path
1. **Define it** (one Commit; how to commit from a script: `aiworkspace_template_dev` “A write is a Commit”). Field ids are stable identifiers the program writes; the primary key identifies a record across syncs (never the row number); `limit` keeps only the first N rows:
   ```json
   [{ "op": "entity.create", "entity_id": "quotes", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "q", "name": "Quotes",
      "schema_id": "<owner>.quotes@1",
      "payload": { "fields": [{ "field_id": "symbol", "name": "Symbol", "type": "text" }, { "field_id": "close", "name": "Close", "type": "number" }, { "field_id": "day", "name": "Day", "type": "date" }],
                   "title_field_id": "symbol",
                   "source": { "kind": "zone", "path": "/home/<owner>/aiws-sync/quotes/out.json", "refresh": { "interval_s": 3600 },
                               "def": { "meaning": "Daily closing prices of the watch list", "origin": "the exchange's daily CSV", "primary_key": ["symbol"],
                                        "freshness": { "max_age_s": 86400 }, "limit": 100 } } } }]
   ```
   `refresh.interval_s` (≥ 900) only re-reads the file on the service's timer. `schema_id` (optional, `<namespace>.<name>@<major>`, not `buckyos.`) registers these fields as a schema a wish result can follow.
2. **Write the program** next to its output, e.g. `$BUCKYOS_ROOT/data/home/<owner>/aiws-sync/quotes/sync.ts`. It fetches only what the workspace needs, maps it to the field ids, and writes the whole output at once (write `out.json.tmp`, then rename it to `out.json`), so a refresh never reads half a file:
   ```json
   { "format": "aiws.sync.v1", "status": "ok", "program": { "name": "quotes", "version": "1" },
     "business_time": "2026-11-15T21:00:00Z", "produced_at": "2026-11-15T21:03:10Z",
     "records": [{ "symbol": "AAPL", "close": 190.5, "day": "2026-11-15" }], "truncated": false }
   ```
   `business_time` is when the data is from (a quote's close, not when you fetched it); `fields` may be left out (the table's are kept). When fetching or parsing fails, write `{ "format": "aiws.sync.v1", "status": "failed", "error": "<what went wrong, for a person>", "program": {…} }` instead of exiting silently: the refresh then fails with that reason and the last good version stays. Raise `program.version` with every change.
3. **Test it**: `deno run --allow-net=<the hosts it reads> --allow-read=<its folder> --allow-write=<its folder> sync.ts`, then `jq '.status, (.records | length), .records[0]' out.json`. Every record has its primary key, no two share one, values fit the field types.
4. **First sync**: `proc.start { program: "source.refresh@1", params: { entity_id: "quotes" }, idempotency_key }` (or the entity's `refresh` action, `aiworkspace_run` “Actions on workspace objects”, after asking the user), then `buckyos aiws run-get --workspace <ws> --run <run>` until it leaves `queued` / `running`.
5. **Check**: `buckyos aiws stat quotes --workspace <ws>` (or `doc.read { entity_id, as: "meta" }`): `data.state: valid`, `data.version.records`, `data.attempt.status: succeeded`; `buckyos aiws sql 'SELECT * FROM "/data/Quotes" LIMIT 5' --workspace <ws>`. Then tell whoever asked that the source is ready: the wish is analyzed again and binds it (`aiworkspace_run`), or, when it already binds it, its runs now work.

#### When a refresh fails
`run-get` → `result.error` (also `meta.data.attempt.error`); nothing was written, the last good version stays.
- `DEPENDENCY_UNAVAILABLE` “the sync program failed: …”: your program wrote `status: failed`; fix the fetch, run it again, refresh.
- `DEPENDENCY_UNAVAILABLE` “cannot be read” / “not a file of the owner's home”: the file is missing, or a link leads out of the home; the path must be a plain file under `/home/<owner>/`.
- `PERMISSION_DENIED` “not in the home of this workspace's owner”: the path names someone else's home — a copied workspace reads its new owner's home; the owner sets the path again.
- `PRIMARY_KEY_REQUIRED` (a record without its key, or a key field not among the fields) / `DUPLICATE_KEY` (two records with one key, `data.rows`): fix the program's output.
- `LIMIT_EXCEEDED`: the file is over 32 MiB; bring back less (filter at the source, use `def.limit`).
- A refresh that succeeded with `result.schema_changes` deleted or retyped fields: wishes reading them need a `code` run; tell the owner.

#### Repairing a source
`meta.data`: `attempt` (last try, its error), `version` (last good data), `expired` (its business time, else its last sync, is older than `def.freshness.max_age_s`). Read the error, fix `sync.ts` (raise `program.version`), test it as in step 3, refresh, check. Changing the definition (`source`, its `def`, the path) is an `entity.set_keys` of `source` by the owner. A changed field list is a schema change: keep field ids stable, add fields rather than renaming them.

#### Not available yet
- The system does not run the sync program on a schedule yet (where it runs is still being chosen): `refresh.interval_s` re-reads the file, it does not run `sync.ts`. Run the program when the owner wants fresh data, then refresh; do not set up your own timers or services for it.
- A place for credentials: see “Before you start”.

#### Source
- Service: `src/frame/aiworkspace/server/src/source.rs` (refresh, Zone files, primary keys), `store/src/source.rs` (data version, attempts); contract: README “Sources”, `doc/workspace/kernel/04-数据源发布与同步.md`.
