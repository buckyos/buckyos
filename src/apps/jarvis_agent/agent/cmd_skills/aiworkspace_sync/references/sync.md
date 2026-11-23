#### Source definition
Set `task_id` first. `echo "/home/${OPENDAN_AGENT_ROOT#*/home/}/sync/quotes/out.json"` gives the Zone path; the AgentRoot's last component is the Agent ID. Submit the operations below as one Commit (see `aiworkspace_template_dev`). `refresh.interval_s` rereads the file and `def.freshness.max_age_s` sets the business freshness threshold.

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

#### Sync program
Use stable IDs from the definition as record keys. `business_time` is the time the data describes. Omit `fields` to keep the table fields; return `truncated:true` when deliberately omitting rows, and `state` to pass a cursor to the next run. Fetch only the source's data. The host masks secret values in results and records.

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

#### Repair and change
Use `agent-session sync get quotes` (state, failures, last_run), `run-get` (result.error) and `doc.read {entity_id,as:"meta"}` (`data.expired`, `data.attempt`). Fix the cause, try the program, put it with the same task ID to increment its version, refresh and verify the data.

- `program`: malformed output, a thrown exception or an unhandled non-transient API response. Three consecutive program failures notify the owner.
- `auth`: missing or refused credential, notifies immediately. Have the owner update the source properties. A task awaiting the owner's secret can fail on its timetable.
- `transient`, network error or timeout: the host retries after 30 s and 120 s. If it still fails, verify the host or retry later. For `NotCapable: Requires net access` add the missing host to `--hosts` and put again.
- `SYNC_UNAVAILABLE`: enable the task with `agent-session sync enable quotes`, reinstall a missing task, or check that the Agent belongs to the workspace owner.
- `PRIMARY_KEY_REQUIRED`, `DUPLICATE_KEY` or `LIMIT_EXCEEDED`: correct the records or reduce output size (`def.limit` keeps the first N rows). A successful refresh with removed/retyped fields in `result.schema_changes` means wishes consuming them need a code run.
- Use `sync put` to change the program or timetable and `agent-session sync disable|enable|delete <task_id>` for lifecycle. Only the owner may change the source definition/path with `entity.set_keys` of `source`. Preserve field IDs and prefer adding fields.

Contracts: `src/frame/lib_opendan/src/sync.rs`, `src/frame/opendan/README.md` (`sync.*`) and `src/frame/aiworkspace/{README.md,schemas/sync-envelope.schema.json,server/src/source.rs,store/src/source.rs}`.
