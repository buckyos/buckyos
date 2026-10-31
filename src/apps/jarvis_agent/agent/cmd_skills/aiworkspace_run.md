---
name: aiworkspace_run
description: Execute a wish run of an AI Workspace that was handed to you (a Goal naming a run and its claim token) — claim it, read the task and its data snapshot, produce the results with a program, hand them in, keep the lease alive — and read a workspace's tables and documents.
requires: the buckyos command (agent container: /usr/local/bin/buckyos; host: $BUCKYOS_ROOT/bin/buckyos) signed in as this Agent; Deno for running the program host (the buckyos tool runtime ships one); the workspace owner configured hosted execution with this Agent (ws.set_hosted_exec)
baseline: standard
---
#### When to use
- A Goal or message gives you an AI Workspace run: a workspace id, a run id and a claim token (`ct_…`). The user wrote a task on the canvas (a "wish"); the service fixed its inputs and output contract; you do the work and hand the results back as a candidate. The user previews and applies them in the workspace — you never write workspace data yourself.
- Reading a workspace without a run (`info`, `read`, `query`) is for looking things up; it needs the owner to have granted you access.

#### Before you start
- `buckyos aiws --help`, `buckyos aiws run-claim --help`: the verbs and their options. Source: `cli/modules/aiws.ts` in buckyos-websdk (the only implementation).
- `buckyos aiws run-get --workspace <ws> --run <run>` shows the run's state, stage (`analyze`, `execute`, `rerun_program`, `repair_program`), execution context and a brief. A run that is `cancelled` is over; a run that is `interrupted` can be claimed again (a new attempt).
- Keep the lease alive while you work: `run-heartbeat` at least every minute (the claim answers `lease_ms`); a lapsed lease ends your attempt (`RUN_REVOKED`).

#### Shortest path (an execute run)
```bash
WS=ws_…; RUN=run_…; TOKEN=ct_…
buckyos aiws run-claim --workspace $WS --run $RUN --claim-token $TOKEN > claim.json      # attempt, run_token, lease, stage
jq -r .stage.user claim.json > task.md                                                   # the task: context map, task description, inputs, output contract, checks
jq .stage.tools claim.json                                                               # the tools you may call with run-tool
buckyos aiws run-files --workspace $WS --run $RUN --attempt 1 --dir work                 # context/, request.json, catalog/, lib/ (the program host)
# write work/program/main.js: an ES module `export default async function main(aiws) { … }` — the API is in task.md (aiws v2)
(cd work && deno run -A lib/run.js > run.log 2>&1)                                       # output/.aiws/results.json; the summary on stdout
buckyos aiws run-program --workspace $WS --run $RUN --attempt 1 --dir work --log-file work/run.log   # validated against the contract: previews, checks, problems
buckyos aiws run-tool --workspace $WS --run $RUN --attempt 1 --name put_result --args '{"name":"commentary","type":"text","markdown":"## …"}'
buckyos aiws run-tool --workspace $WS --run $RUN --attempt 1 --name check_results
buckyos aiws run-tool --workspace $WS --run $RUN --attempt 1 --name finish --args '{"summary":"…","assumptions":[],"warnings":[]}'
buckyos aiws run-report --workspace $WS --run $RUN --attempt 1 --finished
```
An `analyze` run has no program: read `stage.user`, inspect data with `run-tool --name ws_profile|ws_read|ws_query`, then `run-tool --name submit_analysis --args-file analysis.json` (the `wish.analysis.v2` object described in `stage.system`) until it is accepted, and `run-report --finished`. A `rerun_program` run comes with `stage.source` (the stored program): write it to `work/program/main.js`, run it, `run-program`, `run-report --finished`.

#### Inputs and outputs
- Outside a run, a workspace entity is the object `obj://aiworkspace/<ws>/entities/<entity_id>` (a Reference's `mode=fixed`, `selector` and `version` go into its query): `read` describes it and `read … property=<name>` reads one property, with your own access to the workspace.
- `stage.user` is the whole task: the workspace map with handles (`@T1`, `@B2`), the inputs by name, the output contract (result names, types, keys, views), the acceptance checks and fixed parameters (`today`, `timezone`). `stage.system` is the stage's rules; follow them (programs first, numbers in text only from facts, checks reported truthfully).
- The program reads inputs with `aiws.input(name)` and writes candidates with `aiws.result.*`, `aiws.facts`, `aiws.check` (the API reference is at the end of `stage.system`). `aiws.llm.map` is not available to a program run outside the service in this version; do per-item judgements yourself and write them as a result table.
- A mind map (or another Block-control, `catalog/controls.json`) is one result: `aiws.result.control("plan", { kind: "mindmap", children: "- Launch\n  - Market\n  - Product\n" })` — a Markdown outline (or `[{ text, children }]`); the service lays it out, never give coordinates. To grow an existing branch, take the node as an input (it reads as a Markdown outline of its subtree) and add `under: "<that input's name>"`: the new nodes hang under it.
- An HTML result (`aiws.result.html`) is a control of the workspace plus a Block: its `bindings` map names to `result:<name>` or `input:<name>`, and the page reads them with `aiws.input("<name>")`. Controls a user built or imported (`data/extensions`) are bound the same way, through the Cell's `bindings`; their self-description is the definition's `agent` text (`buckyos aiws read <def> --workspace <ws>`).
- Outside a run you place controls with `doc.commit`. A composite control (a definition with `def_kind: composite`; its `composition.slots` say what each slot takes) is a container whose content are ordinary Blocks, each with the `slot` it fills; leave their positions to the composite (`"auto"`), except under a `fixed` layout, which keeps numbers:
  ```json
  [{ "op": "entity.create", "entity_id": "board-1", "type_id": "buckyos.container", "parent_id": "<surface>", "order_key": "m",
     "placement": { "x": 40, "y": 40, "w": "auto", "h": "auto" },
     "payload": { "kind": "composite", "def_ref": { "entity_id": "<composite def>" }, "config": { "title": "Today", "gap": 8 } } },
   { "op": "entity.create", "entity_id": "card-1", "type_id": "buckyos.cell", "parent_id": "board-1", "order_key": "a",
     "placement": { "x": "auto", "y": "auto", "w": 240, "h": 120 },
     "payload": { "view": { "type": "html" }, "def_ref": { "entity_id": "<card def>" }, "slot": "cards", "bindings": { "task": { "entity_id": "<record>" } } } }]
  ```
  `def_ref` of an instance never changes; moving a Block in or out is `tree.move { …, slot: "<name>" | null }`.
- To copy Blocks — a composite instance with its cards, a group, a single Block — use one `tree.clone { source_id, parent_id, placement?, slot?, id_seed }` instead of re-creating them: the copies share definitions and shared data, own data (a note's text) is cloned, inner connectors follow. Pick a fresh `id_seed` (≤ 64 bytes); the accepted Commit lists `cloned: [{ op_index, new_ids: { <old id>: <new id> } }]`, and the ids are `c` + the first 26 hex digits of `sha256("<id_seed>\n<old id>")`, so you know them before the answer comes.
- Every `run-tool` answer is `{ status: "ok" | "error", text }`: `error` is what you must correct (a wrong handle, a result not in the contract, a check not run), not a failure of the run. `finish` is refused until the contract is met.
- `run-report --finished` makes the service plan the candidate; the user confirms it. Give up with `run-report --error "<reason>"` only when the task cannot be done with the data there is (say what is missing).

#### When it fails
- `RUN_REVOKED`: the run was cancelled, or another attempt took it over, or your lease lapsed — stop; claim again only when `run-get` says `interrupted`.
- `DELEGATION_STALE` (at apply time, on the user's side): the wish changed after the run started; nothing to do for you.
- `access_denied` on `run-claim`: the claim token is not this run's, or the owner's hosted configuration does not admit this run (`allowed_wishes`, `triggerers`).
- `deno: command not found`: use the tool runtime's Deno (`$BUCKYOS_ROOT/libexec/buckyos-tool/runtime/deno`) or install one.

#### Check the result
`buckyos aiws run-get --workspace $WS --run $RUN`: `state` is `validating` then `waiting_confirmation` with a `candidate` and a `preview`; `failed` carries the service's reason (`error.detail`), e.g. results that failed validation.

#### Source
- Protocol: `src/frame/aiworkspace/README.md` (Protocol, `proc.*`); execution model: `doc/workspace/BuckyOS AiWorkspace 内核架构改进.md` §9.
- Service side: `src/frame/aiworkspace/server/src/wish/exec.rs` (claim, heartbeat, report, tool, program, files) and `stages.rs` (the stage host).
