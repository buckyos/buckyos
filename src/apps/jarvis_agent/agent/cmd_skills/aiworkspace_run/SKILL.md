---
name: aiworkspace_run
description: Execute a wish run of an AI Workspace that was handed to you (a Goal naming a run and its claim token) — claim it, analyze the task, or write and trial-run the wish's macro, or run the stored macro; hand it in, keep the lease alive — start an analysis run yourself when you worked out a wish's task in a conversation, and read a workspace's tables and documents.
requires: the buckyos command (agent container: /opt/buckyos/bin/agent-tools/buckyos; host: $BUCKYOS_ROOT/bin/buckyos) signed in as this Agent (it runs macros with its own Deno); the workspace owner configured hosted execution with this Agent (ws.set_hosted_exec), or you have update on the wish (an analyze or code run you start yourself; running the stored macro takes run or any write right on the wish)
baseline: standard
commands: jq
---
#### When to use
- A Goal or message gives you an AI Workspace run: a workspace id, a run id and a claim token (`ct_…`). The user wrote a task on the canvas (a "wish"); the service fixed its inputs and output contract; you do the work and hand the results back as a candidate. The user previews and applies them in the workspace — you never write workspace data yourself.
- Reading a workspace without a run (`info`, `read`, `query`) is for looking things up; it needs the owner to have granted you access.

#### Before you start
- `buckyos aiws --help`, `buckyos aiws run-claim --help`: the verbs and their options. Source: `cli/modules/aiws.ts` in buckyos-websdk (the only implementation).
- `buckyos aiws run-get --workspace <ws> --run <run>` shows the run's state, stage (`analyze`, `code`, `run`), execution context and a brief. A run that is `cancelled` is over; a run that is `interrupted` can be claimed again (a new attempt).
- The three stages: `analyze` settles the task contract (inputs, output contract, checks); `code` writes the wish's **macro** — one program that produces every result — and trial-runs it: its candidate is the first run, and applying it stores the macro; `run` runs the stored macro again on current data, with no model in the loop. How to write a macro: the macro guide, `src/frame/aiworkspace/aiws/README.md` (embedded in `stage.system` of a `code` run).
- Keep the lease alive while you work: `run-heartbeat` at least every minute (the claim answers `lease_ms`); a lapsed lease ends your attempt (`RUN_REVOKED`).

#### Shortest path (a code run)
```bash
WS=ws_…; RUN=run_…; TOKEN=ct_…
buckyos aiws run-claim --workspace $WS --run $RUN --claim-token $TOKEN > claim.json      # attempt, run_token, lease, stage (keep it outside work/)
jq -r .stage.user claim.json > task.md                                                   # the task: context map, task description, inputs, output contract, checks
jq -r .stage.system claim.json > rules.md                                                # the stage's rules and the macro guide
buckyos aiws run-files --workspace $WS --run $RUN --attempt 1 --dir work                 # context/, request.json, catalog/, lib/ (program host, stage budget), program/main.js if one exists
# write work/program/main.js: `export default async function main(aiws) { aiws.declare({ … }); … }` — see the macro guide in rules.md
buckyos aiws run-program --workspace $WS --run $RUN --attempt 1 --dir work               # runs it as every executor does and hands it in: previews, checks, manifest, problems
buckyos aiws run-tool --workspace $WS --run $RUN --attempt 1 --name check_results
buckyos aiws run-tool --workspace $WS --run $RUN --attempt 1 --name finish --args '{"summary":"…","assumptions":[],"warnings":[]}'
buckyos aiws run-report --workspace $WS --run $RUN --attempt 1 --finished
```
`run-program` (and `run-exec`, which only runs and prints the outcome, for debugging) starts the macro with the tool's own Deno, reading only `work/`, writing only `work/output/`, reaching only the Zone, with no environment and no token — `claim.json` outside `work/` stays out of its reach; never run a macro with `deno run -A`. Its `aiws.sql`, `aiws.ai.complete`, `aiws.llm.map` and `aiws.zone.fetch` are answered by the tool: SQL by the service on the run's snapshot, AI with your own AI resources (budgeted, cached by content), Zone resources with your session — `aiws.zone.fetch` only reads (GET / HEAD) and only what the macro lists in `aiws.declare({ …, zone: ["/kapi/<service>/…", …] })`; anything else answers `ZONE_READ_ONLY` / `ZONE_NOT_DECLARED`.

A `run` run comes with `stage.source` and `stage.manifest` (the stored macro): `run-files` writes it to `work/program/main.js`; `run-program`, then `run-report --finished`. Do not change the macro in a `run` run: a different one is refused; a run that fails (`PROGRAM_FAILED`, `MACRO_CODE_CHANGED`) is over — the wish needs a `code` run.

An `analyze` run has no macro: read `stage.user`, inspect data with `run-tool --name ws_profile|ws_read|ws_find|ws_query` (`ws_query` takes one SQL SELECT: `{ "sql": "SELECT \"月份\", sum(\"销售额\") FROM \"@T1\" GROUP BY 1" }` — a table by its handle, path or id, columns are field names, `"<field>#raw"` the stored values; `ws_read` of a table view Block gives its rows and its SQL; to join large tables put the smaller one in `WITH d AS MATERIALIZED (SELECT …)` and join to `d`), then `run-tool --name submit_analysis --args-file analysis.json` (the `wish.analysis.v3` object described in `stage.system`; contract results have no `approach`) until it is accepted, and `run-report --finished`.

#### Starting a run yourself
When you worked out a wish's task with the user in a conversation, hand it over as an analysis: `buckyos aiws run-start --workspace $WS --wish <wish id> --stage analyze` answers a run and its `claim_token` (you are its executor; your AI resources pay; you need update on the wish — every stage writes the wish, so `run` alone starts nothing: `MACRO_CANNOT_WRITE`), then claim it and `run-tool --name submit_analysis` + `run-report --finished` as above (or `run-report --analysis-file analysis.json`). The user confirms it in the workspace; coding works from it whoever analyzed.

#### Inputs and outputs
- Outside a run, a workspace entity is the object `obj://aiworkspace/<ws>/entities/<entity_id>` (a Reference's `mode=fixed`, `selector` and `version` go into its query): `read` describes it and `read … property=<name>` reads one property, with your own access to the workspace.
- `stage.user` is the whole task: the workspace map with handles (`@T1`, `@B2`), the inputs by name, the output contract (result names, types, keys, views), the acceptance checks and fixed parameters (`today`, `timezone`). `stage.system` is the stage's rules; follow them (programs first, numbers in text only from facts, checks reported truthfully).
- The macro declares what it reads (`aiws.declare`), reads inputs with `aiws.input(name)` or `aiws.sql`, has the model write text from its facts with `aiws.ai.complete`, judges items with `aiws.llm.map`, and writes candidates with `aiws.result.*`, `aiws.facts`, `aiws.check` (the macro guide is at the end of `stage.system`). Every result comes from the macro; there is no way to hand a result in directly.
- A mind map (or another Block-control, `catalog/controls.json`) is one result: `aiws.result.control("plan", { kind: "mindmap", children: "- Launch\n  - Market\n  - Product\n" })` — a Markdown outline (or `[{ text, children }]`); the service lays it out, never give coordinates. To grow an existing branch, take the node as an input (it reads as a Markdown outline of its subtree) and add `under: "<that input's name>"`: the new nodes hang under it.
- A composite control of the workspace (`WORKSPACE.md` lists them under “Composite controls of this Workspace” with their slots) is a `control` result too: `aiws.result.control("board", { def: "<author_did>#<name>", config: { title: "Week 44" }, slots: { cards: ["result:summary", "input:card"], header: [{ view: "richtext", markdown: "# Week 44" }] } })`. `result:<name>` moves that result's Block into the slot, `input:<name>` copies an input Block (or shows input data in a new Block), an inline item is a new text; `under: { input, slot }` adds them to a composite you read instead of making a new one. A definition that is not in the workspace is refused in the run: it has to be imported first (`aiworkspace_template_dev`).
- An HTML result (`aiws.result.html`) is a control of the workspace plus a Block: its `bindings` map names to `result:<name>` or `input:<name>`, and the page reads them with `aiws.input("<name>")`. Its html, css and js are fixed when the macro is coded: pass data through `bindings`, never by writing values into the code (a run whose code differs fails with `MACRO_CODE_CHANGED`). Controls a user built or imported (`data/extensions`) are bound the same way, through the Cell's `bindings`; their self-description is the definition's `agent` text (`buckyos aiws read <def> --workspace <ws>`).
- Outside a run you place controls with `doc.commit` (one `commit` call per Commit whatever its size — the client sends a large one in parts; at most 32 MiB / 50 000 operations). A composite control (a definition with `def_kind: composite`; its `composition.slots` say what each slot takes) is a container whose content are ordinary Blocks, each with the `slot` it fills; leave their positions to the composite (`"auto"`), except under a `fixed` layout, which keeps numbers:
  ```json
  [{ "op": "entity.create", "entity_id": "board-1", "type_id": "buckyos.container", "parent_id": "<surface>", "order_key": "m",
     "placement": { "x": 40, "y": 40, "w": "auto", "h": "auto" },
     "payload": { "kind": "composite", "def_ref": { "entity_id": "<composite def>" }, "config": { "title": "Today", "gap": 8 } } },
   { "op": "entity.create", "entity_id": "card-1", "type_id": "buckyos.cell", "parent_id": "board-1", "order_key": "a",
     "placement": { "x": "auto", "y": "auto", "w": 240, "h": 120 },
     "payload": { "view": { "type": "html" }, "def_ref": { "entity_id": "<card def>" }, "slot": "cards", "bindings": { "task": { "entity_id": "<record>" } } } }]
  ```
  `def_ref` of an instance never changes; moving a Block in or out is `tree.move { …, slot: "<name>" | null }`.
- Writing a Block's `config` / `bindings` / `on` outside a run: `entity.set_props` changes only the properties you name (`expect` per property from `key_revs["p:<key>.<name>"]`); handlers in `on` run when a person clicks in run mode with that person's rights — a view-mode collaborator needs `update` on what the handler writes.
- To copy Blocks — a composite instance with its cards, a group, a single Block — use one `tree.clone { source_id, parent_id, placement?, slot?, id_seed }` instead of re-creating them: the copies share definitions and shared data, own data (a note's text) is cloned, inner connectors follow. Pick a fresh `id_seed` (≤ 64 bytes); the accepted Commit lists `cloned: [{ op_index, new_ids: { <old id>: <new id> } }]`, and the ids are `c` + the first 26 hex digits of `sha256("<id_seed>\n<old id>")`, so you know them before the answer comes.
- Every `run-tool` / `run-program` answer is `{ status: "ok" | "error", text }`: `error` is what you must correct (a wrong handle, a result not in the contract, an undeclared field, a check not run), not a failure of the run. `finish` is refused until a trial run succeeded with a manifest and the contract is met.
- `run-report --finished` makes the service plan the candidate; the user confirms it. Give up with `run-report --error "<reason>"` only when the task cannot be done with the data there is (say what is missing).

#### When it fails
- `RUN_REVOKED`: the run was cancelled, or another attempt took it over, or your lease lapsed — stop; claim again only when `run-get` says `interrupted`.
- `DELEGATION_STALE` (at apply time, on the user's side): the wish changed after the run started; nothing to do for you.
- `access_denied` on `run-claim`: the claim token is not this run's, or the owner's hosted configuration does not admit this run (`allowed_wishes`, `triggerers`).
- `MACRO_CANNOT_WRITE` (starting `code` / `run`): the run would be written as whoever starts it (bounded by the wish's `intersection` group), and that identity cannot update the wish or write where its results go — ask someone with update there to run it; `DEPENDENCY_UNAVAILABLE` "input … is not readable": that identity cannot read an input.
- `NEEDS_CODING` (starting a `run`): the macro no longer fits its inputs (a declared field deleted or of another type); the wish needs a `code` run. `NEEDS_ANALYSIS`: the request or the inputs changed; analyze first.
- `AI_NOT_DECLARED` / `AI_BUDGET_EXCEEDED` from a macro's AI call: declare it (`aiws.declare({ ai: { complete: { max_calls, max_tokens } } })`) or call less; `AI_QUOTA_*`: your AI resources refused — report the run's failure, do not retry in a loop.
- `subprocess is not allowed`: run macros with `run-exec` / `run-program` (the tool starts its own Deno), not by hand.

#### Check the result
`buckyos aiws run-get --workspace $WS --run $RUN`: `state` is `validating` then `waiting_confirmation` with a `candidate` and a `preview`; `failed` carries the service's reason (`error.detail`), e.g. results that failed validation.

#### Source
- Protocol: `src/frame/aiworkspace/README.md` (Protocol, `proc.*`); execution model: `doc/workspace/BuckyOS AiWorkspace 内核架构改进.md` §9.
- Service side: `src/frame/aiworkspace/server/src/wish/exec.rs` (claim, heartbeat, report, tool, program, files) and `stages.rs` (the stage host).
- Macro guide: `src/frame/aiworkspace/aiws/README.md`; program host `aiws/aiws.js`, `aiws/run.js`; the executor side of its bridge `aiws/host.js`; the runner: `cli/modules/aiws.ts` (`run-exec`) in buckyos-websdk.
