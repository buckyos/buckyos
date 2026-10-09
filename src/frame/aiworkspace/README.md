# aiworkspace — AI Workspace artifact management service

Backend of the BuckyOS AI Workspace (phase one kernel, phase two structure). Design:
[第一期核心架构设计与验证](<../../../doc/workspace/BuckyOS AI Workspace 第一期核心架构设计与验证.md>),
[第一期内置对象详细设计](<../../../doc/workspace/BuckyOS AI Workspace 第一期内置对象详细设计.md>) (called "the design" below) and
[第二期规划](<../../../doc/workspace/BuckyOS AI Workspace 第二期规划.md>) (the two trees, wishes, dependency records, freshness) and
[许愿格详细设计](<../../../doc/workspace/BuckyOS AI  Workspace 许愿格详细设计.md>) v0.2 (the real wish executor, W0–W4) and
[第三期规划](<../../../doc/workspace/BuckyOS AI Workspace 第三期规划.md>) v0.2 (Frames, Viewports, presentation paths, non-public shows).

```text
core/      aiworkspace-core   pure logic, builds for wasm32-unknown-unknown (no tokio/fs/sqlite/clock/random);
                              anchor.rs: annotation anchors (target / range / quote) and per-type anchor adapters;
                              freshness.rs: dependency-based freshness and relation queries (shared with the replica);
                              wish.rs: wish v2 contracts and digests; markdown.rs: Markdown ⇄ rich text
store/     aiworkspace-store  SQLite storage, object store, packages, Mock runs, URL sources;
                              wish/: snapshot, context map, read tools, result collector, planner, run records
server/    aiworkspace        process entry, kRPC dispatch, upload/download routes, auth;
                              wish/: run orchestration, xllm adapter and host tools, Deno program runner, llm.map
aiws/      aiws v2 package    program host (aiws.js, run.js), API doc for the model, prompts, renderer catalog (deno test)
wasm/      aiworkspace-wasm   wasm-bindgen facade of core for the browser replica (`wasm/build.sh` writes it into the Desktop app)
schemas/   richtext.basic.v1.json (single source for the Rust validator and the ProseMirror schema)
fixtures/  project-workspace/commits.json (the shared sample as a commit sequence), presentation/commits.json (the third
           phase M0 fixture: a five-step mixed path and a guide), vectors/
```

## Run

Standalone (independent testing; data in an isolated folder, static test identities):

```bash
cargo build -p aiworkspace
echo '{"tokens":{"tok-alice":{"principal":"alice"},"tok-bob":{"principal":"bob"}}}' > /tmp/aiws-tokens.json
aiworkspace --data-dir /tmp/aiws-data --listen 127.0.0.1:4120 --auth-file /tmp/aiws-tokens.json --fixture-sources
```

BuckyOS service mode (no `--data-dir`): logs in through buckyos-api as kernel service `aiworkspace`,
keeps Workspace folders under the service data folder, listens on port 4120, is reached through the
generic gateway route `/kapi/aiworkspace`. Every request is authenticated by the service itself
(verify-hub session token); the gateway does not do it.

`AIWS_FAILPOINT=before_txn|in_txn|after_commit[:abort][:N]` injects a fault into the N-th commit.

## Tests

```bash
cargo test -p aiworkspace-core      # values, canonical ids, filters, rich text codec, planner, annotation anchors, phase2 (two trees, wishes, freshness)
cargo test -p aiworkspace-store     # V01–V22, V24, write locks, Mock — against real SQLite files;
                                    # tests/replica.rs: the offline engine incl. the incremental rows a replica persists
cargo test -p aiworkspace           # V23 + crash recovery — against the real process over HTTP;
                                    # tests/wish.rs, tests/quality.rs: wish runs through the real xllm loop and Deno
                                    # with a scripted model (needs `deno` on PATH or AIWS_DENO)
(cd frame/aiworkspace/aiws && deno test -A)   # the program host (temp dirs, a local test server)
```

## The two trees (phase two §4)

Every Workspace has the fixed system nodes `root` → `data` (the data tree) and `surfaces` (the Surface
collection); `canvas-content` is a system folder under `data` holding one folder per Surface, `shows` another one
holding the presentation paths and Viewports (below). They exist from
`seq` 0 and cannot be created, deleted, moved or renamed by operations. Structure rules (`core/src/plan.rs`
`child_allowed`): `data` / `folder` take folders and data entities (TableSource, RichText, Record, AssetRef,
Annotation, `buckyos.wish`, `buckyos.block-def`); `surfaces` takes `surface` containers; `surface` / `group`
take Cells and groups only. A Surface names its content folder (`content_folder_id`); the folder may point back
(`surface_id`). A Surface may carry `icon`: the id of a client-side preset icon (`[a-z][a-z0-9-]{0,31}`, Surfaces
only; the client owns the preset list), a shared, undoable key projected into the outline like `title`. `placement` is `{ x, y, w, h, rotation? }` relative to the parent (`rotation`: degrees clockwise about the centre,
`[0, 360)`, absent = 0); stacking order is `order_key`. A Cell or a group may carry `locked: true`: a shared flag
the client honours (no moves, resizing or deletion from the canvas) and the core only checks for being boolean.
`FORMAT_VERSION` is 0.3 since these two keys (older kernels would refuse them on replay or import); stores and
packages of format 0.2 are refused like any other version. `PROTOCOL_VERSION` is 0.4 since named connector anchors
(0.3: connectors, below). Free
notes are annotations without `target`. A Cell's `view.type` is any renderer id (format checked here, support
decided by the front-end registry, D6); it may have no `source_ref`, a `config` (≤ 64 KiB) and a `def_ref` to a
Block definition entity (blocks its deletion).

`Cell.config.snapshot` is reserved for a static Block preview: `{ object_id, media_type, size }`.
The core verifies it through the AssetRef upload path and normalizes media type and size from the uploaded
content. It creates an `asset` reference owned by the Cell, so access follows the Cell's read permission.
Materialization, package closure, retention, replica bootstrap and undo include this reference. A package
without the snapshot bytes lists the asset in `missing`; a self-contained package includes the bytes.
The Desktop validates renderer-specific definition metadata; the core keeps accepting unknown renderers
and their configuration for generic fallback.

Results of a wish carry a **dependency record** (`entity.set_derived`): wish, run, executor and the read-set
versions; it is stored in `entities.derived_json`, indexed as `derived` / `produced` references (never blocking
deletion), travels in packages (rebased onto the package's own versions on import; a stale result stays stale)
and gets an addressable version in `entity_versions` at every application. Freshness (`doc.freshness`) compares
A result the user keeps against a later run ("保留人工修改") is re-recorded with that run's read set and `kept_manual: true`, so the wish is current again without regenerating it.
those versions with the cells now: `current | stale | upstream_stale | unavailable | unknown`.

Format version is `0.4` (since the presentation entities below; protocol `0.5`); stores and packages of an older format are refused with `UNSUPPORTED_VERSION` (no migration).

## Presentation (第三期规划 §4–§6, §8, §11)

The data tree has a fourth system node, `shows` (a folder with `system: "shows"` under `data`), holding the presentation
paths and Viewports and nothing else; neither type is allowed anywhere else (`plan.rs` `child_allowed`).

| Entity | Payload |
| --- | --- |
| `buckyos.viewport` | `title?`, `surface_ref: { entity_id }` (a free-layout Surface, judged when written), `center: { x, y }`, `zoom` in [0.05, 4], `notes?` (≤ 16 KiB), `caption?` (≤ 2000 chars). The visible area is `stage / zoom` around `center`, `stage` being the size of the path that shows it. |
| `buckyos.show-path` | `title?`, `purpose: presentation \| guide`, `stage: { w, h }` (16–16384), `background?` (`#rgb`/`#rrggbb`/`#rrggbbaa`), `steps: [{ id, target: { kind: frame \| viewport, entity_id }, title?, enabled?, transition?: auto \| fly \| fade \| cut, hide?: [cell ids] }]` (≤ 500, ids unique). |
| Cell `presentation` | `{ live? }` on any Block but a line (operable on stage, §10.2); `{ background?, notes?, caption? }` on a Frame only. `null` removes it. |

- Step targets are judged when a step is added or retargeted (alive, readable, a Frame Cell on a free Surface or a
  Viewport); an untouched step whose target went away stays — a dangling step the editor lists and a show skips.
- References never block deletion: every step is a `show_target` edge (selector `{ kind: step, id }`), a Viewport's
  Surface a `show_surface` edge. The whole `steps` array is one version cell; the editor replays its step commands on
  conflicts (front end).
- Outline: a Viewport carries `viewport: { surface_id, center, zoom }`, a path `show_path: { purpose, stage, steps }`
  (a count), a Block with `presentation.live` carries `live: true`. Notes and captions are read with `doc.read`.
- Wishes never see `shows` or its entities (`wish/snapshot.rs`), and the read tools never list a Cell's `presentation`.
- `doc.export` takes `include_notes` (default: true for `personal_backup`, false for `share`); without it Frame
  `presentation.notes` and Viewport `notes` are left out of the package (`materialize_with`), captions stay. The manifest
  says `notes_included`.

### Non-public shows

| Method | Caller | Params → result |
| --- | --- | --- |
| `show.start` | session | `workspace_id`, `path_id`, `live` → `show_id`, `prompter_token`, `clone_workspace_id?`, `locked`, `live`, `started_at`, `expires_at`, `lease_ms` |
| `show.heartbeat` | presenter | `show_id` → `expires_at` (renews the lock and the clone) |
| `show.publish` | presenter | `show_id`, `seq`, `state` (≤ 64 KiB; an older `seq` never replaces a newer state) |
| `show.command` | presenter or token | `show_id`, `command: { command_id, kind, args? }` → `cursor`, `duplicate` |
| `show.watch` | presenter or token | `show_id`, `after_seq?`, `after_command?`, `timeout_ms?` (long poll) → `state`, `seq`, `commands`, `cursor`, `timed_out` |
| `show.notes` | presenter or token | `show_id` → the path's steps with title, notes, caption, as the presenter reads them |
| `show.end` | presenter, or `manage` on the Workspace (forced end) | `show_id` |

- A caller with workspace-level write access locks the Workspace (`store/src/show.rs`): every document commit from any
  session is refused with `SHOW_LOCKED` (retryable; `data: { show_id, presenter, started_at }`), user state is not
  affected. `ws.get_info` and every `doc.wait_changes` answer carry `show_lock`. A reader presents without a lock.
- With `live` (and write access) the show gets a temporary clone: `doc.sqlite` and `local.sqlite` are copied with
  `VACUUM INTO` inside the Workspace's writer, identity rewritten (`workspace_meta.purpose = show`, source, show id),
  grants and wish runs kept (run folders hard-linked, active runs marked interrupted), locks, uploads and user state
  dropped. Objects are content-addressed in the service's store and shared, not copied (10,000 × 20 table: ~45 ms,
  release build). A clone is not listed by `ws.list`, cannot be prepared for offline or presented itself.
- The relay, the lock and the clones live in memory: the lease is 60 s (`AIWS_SHOW_LEASE_MS` shortens it for tests);
  a sweeper ends shows that were not renewed (lock released, clone deleted); a restarted service drops every clone left
  on disk. A prompter authenticates with `params.prompter_token` instead of a session token, and only for
  `show.command`, `show.watch` and `show.notes` of its show.

## Connector (连接线方案 C1)

Design: [连接线实现方案讨论](<../../../doc/workspace/连接线实现方案讨论.md>) §4, §6, §8.3. A connector is a Cell with
`view.type = "connector"` (`view.version` 1): a line from the top-left to the bottom-right corner of its placement
box, mirrored by `flip`. Its own payload keys, each a separate version cell, are refused on any other view:

| Key | Shape |
| --- | --- |
| `flip` | absent, `null` or `{ h?: bool, v?: bool }` |
| `start` / `end` | absent or `null` = coordinate endpoint (the corner); bound = `{ entity_id, anchor: { kind: "named", id } }`, id `[A-Za-z0-9_.:-]{1,64}`: one of the target's anchors (which ids exist is the target renderer's business — by default the 16-point compass `n` … `nnw` — so unknown ids are not refused) |
| `route` | absent (= `straight`), `straight`, `elbow`, `curve` |
| `controls` | absent or ≤ 64 × `{ u, v, dx?, dy? }` (more → `LIMIT_EXCEEDED`) |
| `label` | absent or `{ t in [0, 1], offset? }` |

It may carry `title` (the label text), `config` (appearance, ≤ 64 KiB, not inspected) and `locked`; never
`source_ref`, `bindings`, `def_ref` or the table keys. All numbers are finite; unknown sub-keys are refused.
`view.type` never changes to or from `connector` (`INVALID_OPERATION`).

- **Placement** (`plan.rs` `check_placement`): required on `entity.create` and `tree.move`, `placement: null`
  refused on `tree.place` (`order_key` alone is fine); `{ x, y, w ≥ 0, h ≥ 0 }`, no `rotation`; with two coordinate
  ends the box is not a point (internal callers — undo — may restore one written while the ends were bound).
- **Bindings** (`types.rs` `validate_connector`), judged when written — on create, and for the endpoint whose key
  changed (an untouched dangling end stays editable): the target passes `check_ref_target` (alive and readable,
  else `REFERENCE_BROKEN`), is a Cell that is not a connector or a `group` (not the line itself, data, a Surface),
  and sits on the line's Surface (`plan::surface_of`, an ancestor group included) — else `INVALID_SCHEMA`. A
  self loop needs two different anchors and a route other than `straight`. Creating a connector needs a
  free-layout Surface, and so does a new binding (`INVALID_OPERATION`); a later move into a flow page or a layout
  switch is not checked (the line is kept, just not drawn). Import and replay check shapes only and keep dangling
  endpoints verbatim.
- **References**: each bound end is a `connector_endpoint` edge with selector `{ kind: "connector_end", end }`. It
  never blocks deletion (the client freezes the line's corner in the deleting commit), takes no part in
  freshness, and is listed as an incoming reference by `doc.relations`.
- **Outline**: a connector's envelope carries `connector: { start, end, flip?, route?, controls?, label? }`
  (`start` / `end` always present, `null` when free); `config` is read with `doc.read`.

## Wish runs (许愿格 v0.2)

Two `proc` programs (design §13). Runs live in `local.sqlite` `runs` (never exported); working files in
`<ws>/runs/<run_id>/{work,xllm}`, the `llm.map` cache in `<ws>/cache/llm_map/`.

| Program | `params` |
| --- | --- |
| `wish.xllm@1` | `wish_id`, `stage: analyze \| execute \| rerun_program \| repair_program`, `parent_run_id?` + `feedback?` (a feedback round), `location?` (`{ surface_id, cell_id, selection, viewport }`, the task location of the map), `request?` (`aiws.request`) |
| `wish.mock@1` | `wish_id`, `stage: execute`, `provided` — the browser Mock executor's results, planned and applied like a real run |

States: `queued → snapshotting → running → validating → waiting_confirmation → applying → succeeded`, or
`failed | cancelled | interrupted | conflict | rejected`. A service restart marks running runs `interrupted`
(no automatic re-run); an `applying` run is reconciled through the Commit idempotency record
`run/<run_id>/<plan_digest>`. `proc.get { run_id, choices? }` returns progress, the candidate and — while
waiting — the preview of the plan for `choices` (`{ results: { <name>: keep | replace | new }, confirm_structure? }`);
`proc.apply { run_id, plan_digest }` applies exactly that previewed plan in one Commit (origin `program`).

- **Analyze** (`llm.plan`): the model reads `WORKSPACE.md` (the context map) and the read-only tools
  `ws_outline / ws_find / ws_profile / ws_neighbors / ws_read / ws_query`, and delivers through
  `submit_analysis` (checked on the spot). Applying writes `analysis` (`wish.analysis.v2`; `basis.digest`
  is computed by core) and the named `inputs`.
- **Execute** (`llm.code`): bash tool group plus the read tools and `run_program / put_result / check_results /
  finish`. The program (`program/main.js`, `export default async function main(aiws)`) runs under Deno:
  `--allow-read=<work> --allow-write=<work>/output --allow-net`, time and memory limits from the config.
  Reads of undeclared data are appended to the inputs. Feedback rounds start from the parent's program
  and direct results on a new snapshot; `repair_program` changes only the program.
- **Rerun program**: no model; `llm.map` calls only for items not in the cache.

Configuration: in service mode `AiWorkspaceSettings.wish` (models `llm.plan / llm.code / llm.chat` through
AICC, iterations, `max_output_tokens` (every stage sends it; Claude requires one), timeouts, program memory,
`llm.map` item limit, `deno`). The service exchanges its startup assertion at verify-hub before it listens:
AICC accepts only verify-hub sessions. Standalone:
`--wish-config <json>` with `{ provider: { type: "openai", base_url, api_key_env }, analyze_model, … }`.
Deno: `deno` setting → `AIWS_DENO` → `$BUCKYOS_ROOT/libexec/buckyos-tool/runtime/deno` → `PATH`.

## Storage

```text
<data>/workspaces/<workspace_id>/doc.sqlite     the portable document (design §4.2)
<data>/workspaces/<workspace_id>/local.sqlite   grants, locks, runs, staged uploads — never exported
<data>/workspaces/<workspace_id>/staging/       export packages, replica databases
<data>/objects/{objects,chunks}/                content-addressed NamedObjects and chunks (CYFS ids)
<data>/trash/                                   deleted / replaced Workspaces
```

One writer per Workspace (a mutex around its connections); a Commit is planned on an overlay,
checked, and written in one SQLite transaction together with its history row, which is also the
idempotency record and the change stream.

## Protocol

kRPC: `POST /kapi/aiworkspace` with `{ "method", "params", "sys": [seq, session_token] }`.
**Business results are always in `result`**: `{ "ok": true, ... }`, `{ "ok": false, "error": { code, retryable, detail, sub_code?, data? } }`,
or the three-state commit result (`status: accepted | conflict | rejected`). The kRPC `error`
string is used only for an invalid token, an unknown method or an unparsable request.

| Method | Params (besides `workspace_id`) |
| --- | --- |
| `ws.create` | `title`, `workspace_id?` |
| `ws.list`, `ws.get_info`, `ws.delete`, `ws.fork` | |
| `ws.grant` / `ws.revoke` / `ws.list_grants` | `subject`, `scope_entity_id?`, `capabilities[]`; `list_grants` returns every row and the `owner` to a manager, only the caller's own rows otherwise (`complete: false`, no owner) |
| `ws.list_subjects` | principals a grant can name (static identities / zone users) |
| `ws.get_user_state` / `ws.set_user_state` | `entries: { key: value \| null }` — user work state per subject and Workspace in `local.sqlite`; never a Commit |
| `ws.begin_import` → PUT → `ws.import` | `upload_id`, `semantics: "restore" \| "new"`, `replace?` |
| `doc.outline` | — all readable entity envelopes in tree order |
| `doc.resolve` | `reference` or `path` |
| `doc.read` | `entity_id`, `selector?` — or `targets: [{ entity_id, selector? }]` |
| `doc.list_children` | `entity_id`, `include_deleted?` |
| `doc.list_annotations` | `target_ids?`, `parent_id?` — annotations anchored to those entities and/or placed under that parent, each with its resolved `anchor` |
| `doc.query` | `view_id` or `source_id`; `filter?`, `sorts?`, `fields?`, `group?`, `limit`, `cursor?`, `consistency?`, `source_revision?` |
| `doc.freshness` | `entity_ids[]` — dependency-based freshness of results and wishes (computed in core) |
| `doc.relations` | `entity_id` — outgoing / incoming references, Blocks showing it, produced results; unreadable referrers only as `hidden_incoming` |
| `doc.list_versions` | `entity_id` — addressable versions (generated results, checkpoints) with their commits |
| `doc.restore_version` | `entity_id`, `content_rev` — the operations restoring that version (submitted as an ordinary Commit) |
| `doc.source_capabilities` | `source_id` (URL query tables) |
| `doc.get_collab_state` | `entity_id` |
| `doc.prepare` / `doc.commit` | the Commit request (design §2.6) |
| `doc.get_submission` | `epoch`, `idempotency_key` |
| `doc.undo` | `epoch`, `commit_id`, `idempotency_key`, `mode?`, `plan_digest?`, `session_id?` |
| `doc.get_changes` | `epoch`, `after_seq`, `limit?`, `filter: { detail: "touched" }?` |
| `doc.wait_changes` | `epoch?`, `after_seq`, `timeout_ms?` (long poll) |
| `doc.checkpoint`, `doc.export` | `mode: "share" \| "personal_backup"`, `self_contained`, `include_notes?` |
| `asset.begin_upload` → PUT → `asset.finish_upload` | `size`, `upload_id` |
| `replica.bootstrap` | — then `GET /replica/<workspace_id>/<replica_id>` |
| `proc.start` / `proc.get` / `proc.apply` / `proc.cancel` | `program`, `params`, `idempotency_key`; `run_id`, `session_id?` — wish programs below |
| `proc.list` | `wish_id?`, `limit?` — wish runs, newest first |
| `lock.acquire` / `lock.renew` / `lock.release` / `lock.break` / `lock.list` | `entity_ids[]`, `session_id`; `lock_ids[]`; `entity_id` |
| `show.start` / `show.heartbeat` / `show.publish` / `show.command` / `show.watch` / `show.notes` / `show.end` | see “Non-public shows” |
| `diag.list_unretained`, `diag.verify_refs` | |

HTTP routes (same port, `Authorization: Bearer <session token>`):

| Route | Use |
| --- | --- |
| `PUT /kapi/aiworkspace/upload/<upload_id>` | asset bytes or an import package |
| `GET /kapi/aiworkspace/asset/<workspace_id>/<object_id>` | verified asset bytes |
| `GET /kapi/aiworkspace/export/<workspace_id>/<export_id>` | export package (zip) |
| `GET /kapi/aiworkspace/replica/<workspace_id>/<replica_id>` | replica database built by `replica.bootstrap` |
| `GET /kapi/aiworkspace/schemas/richtext.basic.v1.json` | rich text schema definition (no auth) |
| `POST /kapi/aiworkspace/wish-host/<token>/llm_map` | `aiws.llm.map` of a running wish program; the token is the run's (no session) |

Operations (`operations[]` of a Commit): `entity.create|delete|restore|rename|set_keys|unset_keys|set_write_policy|set_derived`,
`tree.move|place`, `table.insert_records|delete_records|set_values|unset_values|set_body|add_field|update_field|delete_field|add_option|update_option|delete_option|migrate_field`,
`richtext.apply_update|insert_blocks|replace_block|delete_blocks|move_block`. Parameters are exactly those of the design §3;
`core/tests/planner.rs` and `store/tests/*.rs` are executable examples of every one of them.

## Where this implementation differs from the design text

- **Export/replica download routes carry the `workspace_id`** (`/export/<workspace_id>/<export_id>`): there is no service-level database to look an export id up in.
- **Object storage is a content-addressed folder with CYFS ids and file names**, verified against ndn-lib in `store/tests/kernel.rs` (V01). It is not yet backed by the zone named store, so nothing is pinned there and assets are not fetchable through NDN. `FsObjectStore` is the boundary to replace. It is service-wide, so a show's clone shares every object without copying (第三期规划 §8.3 asked for the named store here; see the third phase report).
- **Files are single `mix256` chunks (≤ 32 MiB)**; ChunkList for larger assets and record files is not written.
- **Undo `partial` mode** determines the applicable part by probing each inverse operation on its own.
- **URL query sources** only reach adapters registered in the service (`fixture://` generated source for tests). There is no general HTTP fetcher: a URL in a document cannot make the service issue arbitrary requests.
- Extra methods: `doc.outline`, `doc.source_capabilities`, `diag.verify_refs`.
