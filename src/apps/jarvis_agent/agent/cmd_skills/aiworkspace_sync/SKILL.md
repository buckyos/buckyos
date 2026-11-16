---
name: aiworkspace_sync
description: Build a data source of an AI Workspace that syncs from outside — when a wish's analysis failed with NEEDS_EXTERNAL_DATA, or the workspace owner asks for one — define it (meaning, fields, primary key, freshness), write the sync program, test it with `agent-session sync try`, install it as your sync task (`sync put`, it then runs on its timetable), link the source to it, run the first sync and check the source turned valid; repair a task that fails or a source whose data expired.
requires: you work for the workspace's owner (only the owner — their Agent included — sets or changes a data source; triggering a refresh needs only `run` on it); an Agent hosted by OpenDAN (`agent-session sync`, your own service runs the program); the buckyos command signed in as this Agent; Deno for the websdk script that writes the definition (or any client of the service's `doc.commit`)
baseline: standard
commands: deno
---
#### When to use
- A wish's analysis run failed with `NEEDS_EXTERNAL_DATA`: its record (`buckyos aiws run-get`, `record.error.missing_inputs[].wanted`) says what data it needs (`meaning`, `origin`, `freshness`, `fields`). Or the owner asks for a data source (a request from the Agent panel names the data, where from, how fresh, and the wish).
- A sync task of yours fails (`agent-session sync get <task_id>` → `state: failing`; the owner may hand you a notification about it), or a data source shows a failed last attempt or `expired` (`doc.read { as: "meta" }` → `data.attempt`, `data.expired`): repair it.
- Who asked decides: build only for the workspace owner. When the run was started by someone else (`run-get` → `context.trigger`), do not build the source — tell them the owner has to ask for it (their panel's “Request a data source…” reaches the owner).

#### Before you start
- A data source is a table whose `source` is `{ kind: "zone", path, sync: { agent_id, task_id } }`: it is filled only by refreshes that read your task's output file; nobody edits its rows (`SOURCE_READ_ONLY`). Until its first good sync it is `pending`: wishes may bind it and code against its fields, they cannot run on it, SQL answers `DATA_PENDING`. Contract: the service README “Sources” (`src/frame/aiworkspace/README.md`), `schemas/sync-envelope.schema.json`.
- Your sync task (`$OPENDAN_AGENT_ROOT/sync/<task_id>/`) is a Deno program your host runs every `--every` seconds (at least 900), and when the source is refreshed. It reaches only the hosts you declare (`--hosts`), reads and writes no files, sees no environment, has 120 s and 32 MiB of output. The host writes its result to `out.json` as `aiws.sync.v1`; you never write that file.
- A failure the program calls `transient` (and a timeout or a network error) is retried after 30 s and 120 s. A `program` failure (it threw, or printed something else) three runs in a row, or an `auth` failure at once, is reported to the owner, who hands it to you to repair. The source keeps its last good data meanwhile.
- Credentials (an API key, a login) are entered by the owner in the data source's properties in the AI Workspace — only there. Never ask for one in a conversation, never put one in the program, its inputs, the definition or a message; when the user pastes one, do not use it and point them to the properties. The program gets them as `secrets.<NAME>`; their values are masked in every result and record.

#### Shortest path
1. **Define the source** (one Commit; how to commit from a script: `aiworkspace_template_dev` “A write is a Commit”). Choose the `task_id` (a–z, 0–9, `-`, `_`) now: the path is your task's output, `echo "/home/${OPENDAN_AGENT_ROOT#*/home/}/sync/<task_id>/out.json"`, and `agent_id` is the last part of that AgentRoot. Field ids are what the program writes; `primary_key` identifies a record across syncs:
   ```json
   [{ "op": "entity.create", "entity_id": "quotes", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "q", "name": "Quotes",
      "schema_id": "<owner>.quotes@1",
      "payload": { "fields": [{ "field_id": "symbol", "name": "Symbol", "type": "text" }, { "field_id": "close", "name": "Close", "type": "number" }, { "field_id": "day", "name": "Day", "type": "date" }],
                   "title_field_id": "symbol",
                   "source": { "kind": "zone", "path": "/home/<owner>/agents/<agent_id>/sync/quotes/out.json", "refresh": { "interval_s": 3600 },
                               "sync": { "agent_id": "<agent_id>", "task_id": "quotes" },
                               "def": { "meaning": "Daily closing prices of the watch list", "origin": "the exchange's daily CSV", "primary_key": ["symbol"],
                                        "freshness": { "max_age_s": 86400 }, "limit": 100 } } } }]
   ```
   `refresh.interval_s` (≥ 900) only re-reads the file on the service's timer; your task's `--every` decides when the program runs.
2. **Write the program** (`sync.ts` in your working directory). It reads one JSON object on stdin and prints one on stdout (`console.error` for logs):
   ```ts
   const { inputs, secrets, state } = JSON.parse(await new Response(Deno.stdin.readable).text());
   try {
     const res = await fetch(`https://api.example.com/v1/close?s=${inputs.symbols.join(",")}`, { headers: { authorization: `Bearer ${secrets.API_KEY}` } });
     if (res.status === 401 || res.status === 403) throw Object.assign(new Error(`the API refused the key (${res.status})`), { kind: "auth" });
     if (!res.ok) throw Object.assign(new Error(`the API answered ${res.status}`), { kind: res.status >= 500 ? "transient" : "program" });
     const rows = await res.json();
     console.log(JSON.stringify({ status: "ok", business_time: rows.as_of, records: rows.items.map((r) => ({ symbol: r.s, close: r.c, day: r.d })), state: { last: rows.as_of } }));
   } catch (e) {
     console.log(JSON.stringify({ status: "failed", error: String(e.message ?? e), error_kind: e.kind ?? "transient" }));
   }
   ```
   `business_time` is when the data is from (a quote's close, not when you fetched it); `fields` may be left out (the table's are kept); `truncated: true` says rows were left out on purpose; `state` comes back on the next run. Fetch only what the source needs; never print a credential.
3. **Try it**: `agent-session sync try quotes --program sync.ts --hosts api.example.com --inputs '{"symbols":["AAPL"]}' [--secret API_KEY]` runs it once as the host would and writes nothing: `preview.status`, `records`, `sample`, `error`, `kind`, `log`. Every record has its primary key, no two share one, values fit the field types. A task with a `--secret` cannot be tried before the owner entered it (step 4 first).
4. **Install it**: `agent-session sync put quotes --title "Quotes" --program sync.ts --every 3600 --hosts api.example.com --inputs '{"symbols":["AAPL"]}' [--secret API_KEY]`. It prints the task: `output.path` (the source's `path`), `version`, `secrets[].set`. From now on it runs every `--every` seconds. When it declares a secret, tell the owner to enter it in the data source's properties (the run until then fails as `auth` and the owner is notified).
5. **First sync**: refresh the source — `proc.start { program: "source.refresh@1", params: { entity_id: "quotes" }, idempotency_key }` (or the entity's `refresh` action, `aiworkspace_run` “Actions on workspace objects”, after asking the user); the service has your host run the task (`sync.run`), then reads the file. `buckyos aiws run-get --workspace <ws> --run <run>` until it leaves `queued` / `running`. (`agent-session sync run quotes` runs it without a refresh; the source picks the file up on its next refresh.)
6. **Check**: `buckyos aiws stat quotes --workspace <ws>` (or `doc.read { entity_id, as: "meta" }`): `data.state: valid`, `data.version.records`, `data.attempt.status: succeeded`; `buckyos aiws sql 'SELECT * FROM "/data/Quotes" LIMIT 5' --workspace <ws>`. Then tell whoever asked that the source is ready: the wish is analyzed again and binds it (`aiworkspace_run`), or, when it already binds it, its runs now work.

#### When a refresh or a run fails
`agent-session sync get quotes` → `state`, `failures`, `last_run { status, kind, error }`; the refresh's `run-get` → `result.error` (also `meta.data.attempt.error`). The last good version stays.
- `the sync program failed: …`, or `last_run.kind: program`: your program threw or printed something else (the error names it); fix it, `sync try`, `sync put` (the version goes up), refresh.
- `kind: auth`: a credential is missing or refused; only the owner enters it — tell them where. `kind: transient`: the source was unreachable three times; try again later or ask whether the host is right.
- `SYNC_UNAVAILABLE`: your task is disabled (`agent-session sync enable quotes`), missing (`sync put`), or you are not the workspace owner's Agent.
- `NotCapable: Requires net access to "<host>"` in the error or log: add the host to `--hosts` and put again.
- `PRIMARY_KEY_REQUIRED` / `DUPLICATE_KEY` / `LIMIT_EXCEEDED` (over 32 MiB): fix what the program prints (`def.limit` keeps the first N rows). A refresh that succeeded with `result.schema_changes` deleted or retyped fields: wishes reading them need a `code` run; tell the owner.

#### Changing or removing
Change the program or its timetable with `sync put` (same `task_id`); `sync disable|enable|delete <task_id>`. Changing the source (`source`, its `def`, the path) is an `entity.set_keys` of `source` by the owner. Keep field ids stable; add fields rather than renaming them.

#### Source
- Host: `src/frame/lib_opendan/src/sync.rs` (runner, envelope, retries, timetable), `sync.*` in `src/frame/opendan/README.md`. Service: `src/frame/aiworkspace/server/src/source.rs` (refresh, Zone files, `source.sync`), `store/src/source.rs` (data version, attempts).
