---
name: aiworkspace_sync
description: Build a data source of an AI Workspace that syncs from outside — when a wish's analysis failed with NEEDS_EXTERNAL_DATA, or the workspace owner asks for one — define it (meaning, fields, primary key, freshness), write the sync program, test it with `agent-session sync try`, install it as your sync task (`sync put`, it then runs on its timetable), link the source to it, run the first sync and check the source turned valid; repair a task that fails or a source whose data expired.
requires: you work for the workspace's owner (only the owner — their Agent included — sets or changes a data source; triggering a refresh needs only `run` on it); an Agent hosted by OpenDAN (`agent-session sync`, your own service runs the program); the buckyos command signed in as this Agent; Deno for the websdk script that writes the definition (or any client of the service's `doc.commit`)
baseline: standard
commands: deno
---
#### Define and test the source
A failed wish names missing data in `run-get` → `record.error.missing_inputs[].wanted`. Check `context.trigger`: only the workspace owner or their Agent sets or changes a source. Refer a collaborator to the owner's “Request a data source…” flow.

1. Agree on meaning, origin, fields, primary key and freshness. Create a `buckyos.table-source` by Commit with durable field IDs, a `source.def` and `source: {kind:"zone",path,sync:{agent_id,task_id},…}`. The task ID uses lowercase letters, digits, `-` and `_`. Set `path` to the Zone path of `$OPENDAN_AGENT_ROOT/sync/<task_id>/out.json`.
2. Write `sync.ts` with one JSON input on stdin (`inputs`, `secrets`, `state`) and one JSON output on stdout. Logs go to stderr. Its host allows only declared network hosts, no filesystem or environment, 120 seconds and 32 MiB of output. The host writes `out.json`. Give each row a unique primary key and typed values; set the envelope's business time from the source.
3. Run `agent-session sync try quotes --program sync.ts --hosts api.example.com --inputs '{"symbols":["AAPL"]}'` and inspect `preview.status`, sample and errors. Install with `agent-session sync put quotes --title "Quotes" --program sync.ts --every 3600 --hosts api.example.com --inputs '{"symbols":["AAPL"]}'`. Verify `output.path`. Set `--every` ≥ 900 seconds. The source's optional `refresh.interval_s` independently rereads the output file.
4. For credentials declare `--secret API_KEY` and install first. Only the owner enters the value in the source's properties. Never solicit or use a pasted key or put it in code, inputs, messages or logs. Check `secrets[].set`, then try with the declared secret.
5. Refresh via `proc.start {program:"source.refresh@1",params:{entity_id:"quotes"},idempotency_key}`. Ask before invoking the equivalent object action. `agent-session sync run quotes` updates the file alone. Poll `buckyos aiws run-get --workspace <ws> --run <run>` and check `buckyos aiws stat quotes --workspace <ws>` for `data.state: valid`, `data.version.records` and `data.attempt.status: succeeded`. Sample the data with `buckyos aiws sql 'SELECT * FROM "/data/Quotes" LIMIT 5' --workspace <ws>`. Tell the requester to reanalyze the wish or run it if already bound.

Until the first success, `pending` data can be bound or coded against, and SQL returns `DATA_PENDING`. Sourced rows are read-only. Failed refreshes preserve the last good version.

#### Examples and repair
Read `$OPENDAN_AGENT_ROOT/cmd_skills/aiworkspace_sync/references/sync.md` for the source definition, Deno program, retries, diagnostics and changing or removing the task.
