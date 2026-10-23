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
import-drawio/                draw.io import/export and the canvas geometry shared with the Desktop (TypeScript, no
                              dependencies, runs in the browser and Deno; see its README; `deno test -A`)
schemas/   richtext.basic.v1.json (single source for the Rust validator and the ProseMirror schema)
fixtures/  project-workspace/commits.json (the shared sample as a commit sequence), presentation/commits.json (the third
           phase M0 fixture: a five-step mixed path and a guide), vectors/, drawio/ (draw.io files and their expected
           import plans)
```

## Run

Standalone (independent testing; data in an isolated folder, static test identities — optional per token: `class`
(`account` | `external` | `service`), `restricted`, `did`, `groups`, `contacts`, `contacts_unavailable`, `auth_method`;
optional top-level `sso` stands for the Zone switches; `groups` / `contacts` are read from the file again at each
request):

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

Named-object imports in standalone mode require `--named-store-config <json>` and `--ndm-gateway <origin>` together. The config uses NamedDataMgr's `stores` array; both the NDM gateway and AIWorkspace must access that store. Service mode resolves both through the BuckyOS runtime. The Desktop Playwright backend starts an isolated NFS/NDM process and supplies these options automatically.

## File and folder imports

The Desktop imports through WebSDK NDM and waits for object readiness, then calls `asset.import_object { workspace_id, obj_id }` for each FileObject. AIWorkspace checks workspace update/structure access and authorizes the object at the trusted NDM gateway using the original caller token. It reads the complete FileObject, including ChunkList content, locally from NamedStore and stages it in the existing workspace ObjectStore. The response retains the workspace `object_id` and adds `source_obj_id`; the two IDs are not interchangeable. This preserves AssetRef reads, package export, replica bootstrap and offline synchronization. The per-asset limit remains 32 MiB, including each leaf of a directory.

Directory roots and descendants become `buckyos.container` folders, and files become AssetRefs in one normal business commit. A canvas import also creates corresponding groups and asset cells. Online imports may be cancelled; after failure or refresh, reselecting the source uses NDM deduplication. There is no offline upload queue or automatic recovery of browser File handles. FileList-based folder pickers cannot enumerate empty folders; callers with an explicit directory tree can preserve them through the SDK. draw.io and workspace ZIP retain their format validation, while image replacement and generated snapshots use the same asset stage/commit semantics.

The existing binary staging API remains available to service/fixture producers and format-specific workspace ZIP import. Staging sessions bind kind, caller and target workspace, persist under `staging/<upload_id>.upload.json`, expire after 24 hours and are swept every minute. The submission intent is persisted before the business operation; a failed finish retains its bytes for a same-parameter retry, and an accepted finish records its result before removing bytes. Repeating identical parameters recovers the same result, including after restart; different parameters are rejected. Workspace imports additionally store per-upload receipts with the committed workspace and preserve them across later restores, so replaying an older upload cannot undo newer work. A restore journal recovers interruption between backing up the old directory and promoting the new one. Restore holds the workspace writer lock while rechecking permission, copying ACLs and replacing its state; queued writers observe the new epoch. Final asset submission rechecks current workspace permission.

## Tests

```bash
cargo test -p aiworkspace-core      # values, canonical ids, filters, rich text codec, planner, annotation anchors, phase2 (two trees, wishes, freshness)
cargo test -p aiworkspace-store     # V01–V22, V24, write locks, Mock — against real SQLite files; tests/sso.rs: caller classes,
                                    # group / contact subjects, share entries;
                                    # tests/replica.rs: the offline engine incl. the incremental rows a replica persists
cargo test -p aiworkspace           # V23 + crash recovery — against the real process over HTTP; tests/sso.rs: the same
                                    # with static Limited / External identities, entry.describe, ws.join_entry, denials;
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
only; the client owns the preset list), a shared, undoable key projected into the outline like `title`. A Surface may also
carry `config`, its page settings (`background`, `page_size`; an object of at most 64 KiB the client checks, read with
`doc.read`, not projected; draw.io 导入 TODO §2.5). `placement` is `{ x, y, w, h, rotation? }` relative to the parent (`rotation`: degrees clockwise about the centre,
`[0, 360)`, absent = 0); stacking order is `order_key`. A Cell or a group may carry `locked: true`: a shared flag
the client honours (no moves, resizing or deletion from the canvas) and the core only checks for being boolean.
(These two keys raised the format to 0.3 at the time; named connector anchors raised the protocol to 0.4; both are
1.0 now, see “Format version” below.) Free
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

Format version is `1.0` (内核架构改进 FORMAT 1.0: `schema_id`, the resource directory, the reference catalogue with `mode` / `integrity` and `owns`, the pure-application log, `$formula`, the `run` capability, delegation fields, the usage and retention ledgers, `user_state.revision`, `base_seq`; protocol `1.0`). Stores and packages of an older format are refused with `UNSUPPORTED_VERSION` (no migration). From 1.0 on, an incompatible change of the format needs an explicit conversion (D07); within 1.0 only additions an older implementation can ignore are allowed.

## Presentation (第三期规划 §4–§6, §8, §11)

The data tree has a fourth system node, `shows` (a folder with `system: "shows"` under `data`), holding the presentation
paths and Viewports and nothing else; neither type is allowed anywhere else (`plan.rs` `child_allowed`).

| Entity | Payload |
| --- | --- |
| `buckyos.viewport` | `title?`, `surface_ref: { entity_id }` (a free-layout Surface, judged when written), `center: { x, y }`, `zoom` in [0.05, 4], `notes?` (≤ 16 KiB), `caption?` (≤ 2000 chars). The visible area is `stage / zoom` around `center`, `stage` being the size of the path that shows it. |
| `buckyos.show-path` | `title?`, `purpose: presentation \| guide`, `stage: { w, h }` (16–16384), `background?` (`#rgb`/`#rrggbb`/`#rrggbbaa`), `steps: [{ id, target: { kind: frame \| viewport, entity_id }, title?, enabled?, transition?: auto \| fly \| fade \| cut, hide?: [cell ids] }]` (≤ 500, ids unique). |
| `buckyos.dir` | A **resource directory** (内核架构改进 D01, D25; not the data tree's `container(kind=folder)`): `members: { <name>: { kind: file, object_id, media_type, size } \| { kind: dir, entity_id } }` (≤ 10 000 names; a name is a version cell `m:<name>`, so renaming is remove + add), `meta?` (≤ 64 KiB). File members name uploaded objects (media type and size come from the content); child directories must be alive and must not contain this one (`member` references keep them alive). Members change only through `dir.set_member` / `dir.remove_member` / `dir.replace`. It materializes to the cyfs-ndn DirObject (`cydir`) with its time fields fixed to 0, so equal members give the same id (`content.dir_object`), and a DirObject imports as a directory (`types::dir_payload_from_dir_object`). v1 takes no external URL members. |
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

**Execution modes (内核架构改进 M3).** `proc.start { params: { …, execution: { mode: service | local | hosted } } }`
(`wish.default_execution` when absent). Every run records its execution context (`detail.context`: trigger, executor,
location, AI and compute attribution, the bound task digest, the declared outputs, expiry) or is refused with
`EXECUTION_CONTEXT_MISSING`. `local`: the starter's browser is the executor — the `proc.start` answer carries a one-time
`claim_token`; it claims, heartbeats and reports. `hosted`: the Owner's Agent (`ws.set_hosted_exec`) is given a Goal with the run
reference and claim token (`goals.create` + `goals.activate` on its OpenDAN service) and does the same through
`buckyos aiws run …`; without a Zone runtime the dispatch is recorded `unavailable` and the run waits to be claimed.
`service`: the transitional in-service executor (xllm + Deno), selectable while `wish.allow_service_execution` is true (T3-9).
Applying checks the delegation: a changed wish configuration is `DELEGATION_STALE`, a candidate of a superseded attempt
or a cancelled run is `RUN_REVOKED`, and results may only land where the task declared (`OUTPUT_NOT_DECLARED`).

Configuration: in service mode `AiWorkspaceSettings.wish` (models `llm.plan / llm.code / llm.chat` through
AICC, iterations, `max_output_tokens` (every stage sends it; Claude requires one), timeouts, program memory,
`llm.map` item limit, `deno`). The service exchanges its startup assertion at verify-hub before it listens:
AICC accepts only verify-hub sessions. Standalone:
`--wish-config <json>` with `{ provider: { type: "openai", base_url, api_key_env }, analyze_model, … }`.
Deno: `deno` setting → `AIWS_DENO` → `$BUCKYOS_ROOT/libexec/buckyos-tool/runtime/deno` → `PATH`
(`deno.exe` on Windows). The Windows program subprocess retains `SystemRoot` after clearing its environment.

## Storage

```text
<data>/workspaces/<workspace_id>/doc.sqlite     the portable document (design §4.2)
<data>/workspaces/<workspace_id>/local.sqlite   grants, share entries, locks, runs, staged uploads — never exported
<data>/workspaces/<workspace_id>/staging/       export packages, replica databases
<data>/objects/{objects,chunks}/                content-addressed NamedObjects and chunks (CYFS ids)
<data>/trash/                                   deleted / replaced Workspaces
```

**Retention, quota and the collector (内核架构改进 D06, D19).** Every object or chunk is kept while a root reaches it:
an asset an alive entity references (`assets.ended_seq IS NULL`), a history-only asset inside its retention period
(`history_retention_days`, 30 by default; `ws.purge_deleted` ends it at once), generated entity versions, snapshots
with a `retentions` row (checkpoints, exports, show clones), unexpired staged uploads, and every Workspace in
`trash/` for 30 days. The collector (`Service::gc`, hourly, `diag.gc`) marks from those roots through the objects'
own references and sweeps the rest once it is an hour old (an upload about to be referenced is never swept). The
`usage` ledger (`ws.get_info.usage`: `content`, `assets`, `exports`, `clones`, `staged`) moves with every
transaction; with a quota (`quota_bytes`) a Commit, an upload, an export or a show clone that would pass it is
refused with `QUOTA_EXCEEDED` (retryable: the maintenance methods can release enough) or `QUOTA_LIMIT`, with
`data { needed_bytes, reclaimable_bytes, quota_bytes, used_bytes, owner }`. The maintenance methods above are
accepted over quota. An undo or version restore that needs a reclaimed resource answers `RESOURCE_RECLAIMED`.

**Shows (D17).** A clone judges access by the grants of the Workspace it was cloned from as they are now
(revoking there ends the audience's access here); any write right, a subtree's included, locks the Workspace
for the show; a Workspace runs at most 4 clones at once and each counts on its quota (`clones`).

`storage_schema_version` is 4 (FORMAT 1.0: `commit_deltas`, `retentions`, `usage`, `user_state.revision`, `refs.mode` / `integrity`, `entities.schema_id`, `grants.task_digest` / `outputs_json`); folders of an older version are refused, not migrated —
reinstall the data of a development Zone.

One writer per Workspace (a mutex around its connections); a Commit is planned on an overlay,
checked, and written in one SQLite transaction together with its history row, which is also the
idempotency record and the change stream.

**The log is pure-application (内核架构改进 D20).** Beside the normalized operations (`commit_ops`, for audit
and undo) every Commit records the rows it wrote (`commit_deltas.rows_json`: entities, edges, fields, records,
rich text states and update bytes, references added and deleted, assets). `doc.get_changes` hands them out as
`rows` (restricted to what the caller may read) and a replica applies them as they are — it never re-plans an
accepted Commit, so the planner can change freely (`ws.get_info.planner_version` is diagnostic only). The fixed
vector `fixtures/vectors/apply-log.json` (`core/tests/apply_log.rs`) proves a recorded history still applies to
its digest without any planner. Local, not-yet-accepted submissions are still planned on the replica for its
provisional view.

## Protocol

kRPC: `POST /kapi/aiworkspace` with `{ "method", "params", "sys": [seq, session_token] }`.
**Business results are always in `result`**: `{ "ok": true, ... }`, `{ "ok": false, "error": { code, retryable, detail, sub_code?, data? } }`,
or the three-state commit result (`status: accepted | conflict | rejected`). The kRPC `error`
string is used only for an invalid token, an unknown method or an unparsable request.

**Callers (hybrid SSO, contract §2–§4, §8.3).** Every request runs as a verified `Caller` with a class:
an **account** (principal = user id; *restricted* when its session carries a ceiling — a Limited account), an
**External** DID (principal = the DID), a **guest** or a **service** (app, agent, system, device). In service mode the
token is checked with `verify_session_token_opened(token, aiworkspace)` — a restricted session without the
`aiworkspace` scope is refused before anything else; the class comes from the claims (`principal_kind`, `external`,
`restricted`, `auth_method`), never from the shape of the principal. Restricted accounts, Externals and guests own
nothing: `ws.create`, `ws.import`, `ws.begin_import`, `ws.fork` and `ws.list_subjects` refuse them
(`session_kind_not_allowed`); `ws.list` shows them only Workspaces granted to them (by name, group, contact or a joined
entry — never through `public` or `*`).

**Grant subjects** and whom they match:

| Subject | Matches |
| --- | --- |
| `<user id>` / `<did>` | that account / that External (exact principal) |
| `public` | everyone, guests included (a public link; never `manage`) |
| `*` | unrestricted accounts only (not Limited, External, guests or services) |
| `group:<id>` | accounts in that system user group (`users/<id>/settings.groups`) |
| `contact:<owner_did>#<tag>` | DIDs in that owner's contact group (msg-center `contact.check_membership`, projections included); only the owner (caller DID = `<owner_did>`) may write such a grant, so nobody probes someone else's groups |

Group and contact membership is settled per request, before the Workspace's writer is locked: the server lists the
Workspace's `group:` / `contact:` subjects (every Workspace's for `ws.list`) and asks system-config (the account's
groups; its DID from `users/<id>/doc`) and msg-center. A membership that cannot be checked refuses the request with
`authorization_unavailable` unless the caller already holds every capability; nothing is cached. A show's notes are read
with the presenter's memberships settled again at each `show.notes`.

**Guests.** A link's visitor has no session: `sys[1]` (or the bearer of the upload / asset / export
routes) carries a guest token instead, `gt_` + 26 base32 chars the browser made up and keeps. The caller is
then `guest:<26 chars>` (derived from the token's SHA-256, never the token itself), reaches the grants of subject
`public` and the active entries open to guests (as everyone does), and cannot own anything. `ws.get_info` returns `principal` so a guest
learns who it is.

**Share entries** (`local.sqlite` `share_entries`, beside the grants): a link that offers ways in — `methods`
`{ guest, password, self_register, did_login }` (`self_register` implies `password`) — and a capability template (never
`manage`) on the Workspace or an entity subtree, optionally until `expires_at` (unix seconds). The id names its Workspace,
`<workspace_id>~se_<26 chars>`, so a link resolves without a service-wide index; the visitor's page is
`/sso/entry?service=aiworkspace&entry=<entry_id>`.

- While an entry open to guests is active (not closed, not expired), every caller — guests, accounts, Externals,
  services — gets the template on its scope, computed per request (anyone could come in anonymously anyway).
- `ws.join_entry` gives a signed-in visitor (account or External) the template as a grant of its own (merged into
  what it already holds there; `via_entry` records the entry), if the entry ∩ the Zone switches (`system/sso/settings`)
  admit the session's sign-in method: `did_federation` needs `did_login`, any local method `password` /
  `self_register`. An entry open to guests that does not admit the session still lets it in, as anyone: the join
  succeeds with `granted: false` and writes no grant (the access ends with the entry). Joining again changes nothing;
  nobody becomes the owner. Closing an entry does not take back joined grants (`ws.revoke` does).
- `entry.describe` is public (no session): `buckyos_api::EntryDescription` (`state`, `resource: { kind: "workspace",
  title, owner_name, path: "/workspace/<id>" }`, the entry's own `methods`, `expires_at`) plus `ok: true`; a missing,
  closed or expired entry is `{ state: "unavailable" }` and nothing else. The Control Panel's `auth.entry.describe`
  intersects the methods with the Zone switches.

**Refusals (D-4).** A business failure that is a refusal carries `error.denial`, the full `AccessDenial`
(`reason`, `adjust: { party, location? }`, `service: "aiworkspace"`, `method`). NOT_FOUND and PERMISSION_DENIED without a
reason of their own are `resource_grant_missing`; for a restricted caller their detail is a fixed text and their
`data` / `path` are dropped, so a missing Workspace and one not shared with it give the same answer
(PERMISSION_DENIED only occurs where the caller already holds a grant). Explicit reasons: `session_kind_not_allowed`
(owning, joining as guest / service), `entry_unavailable`, `zone_method_disabled`, `authorization_unavailable`. A refused
session is a kRPC failure `{ error, code: "access_denied", data: AccessDenial }`, and so is every refusal of
`ws.join_entry` (it serves the Zone's entry page, which reads the shared kRPC wire); its other failures are plain kRPC
errors.

### Sources (内核架构改进 D12, D24, D26)

A data entity (table, rich text, record, asset, directory) may carry a `source` key in its payload:
`{ kind: url | workspace, url?, zone?, workspace_id?, entity_id?, mode: live | fixed, version?: { content_root }, query?, refresh?: { interval_s }, auth_mode: none | hosted, name? }`
(checked by `check_source` in [core/src/types.rs](core/src/types.rs); credentials never belong in it). Such an entity is **read-only** for
people and Blocks: every content write (`table.*` data operations, `richtext.*`, `dir.*`, `entity.set_keys` beyond `source` / `title` /
`description`) is refused with `INVALID_OPERATION/SOURCE_READ_ONLY`; only a refresh writes it, as one Commit of origin `source` with the
run id: `proc.start { program: "source.refresh@1", params: { entity_id } }` (`update` on the entity) — the service fetches (another Workspace of this service as `public`; otherwise one credential-less HTTP GET; `auth_mode: hosted` Sources wait for an Agent), diffs and commits; `proc.get` ends `succeeded` (`result`: the commit, `bytes`, `content_root` of a fixed Source) or `failed` (the last valid content stays). A `refresh.interval_s` Source is refreshed on the service's timer as the owner; fetched bytes count against `traffic_quota_bytes` (`ws.get_info.traffic`). The outline envelope shows `source` (kind, mode, origin,
schedule). Publishing (`ws.publish`) is the other side: the `public` subject reads the entity, and its link
`https://aiws-<workspace_id>.<zonehost>/<workspace_name>/<name>` (or `GET /kapi/aiworkspace/pub/<workspace_id>/<name>`) answers the
publication — JSON (`format: buckyos.ai-workspace/pub/1.0`, `reference`, `content`; `?fixed=1` adds `content_root`; `?fixed=<root>` reads
that pinned version) or, for a browser or crawler (`Accept: text/html`), a static HTML page with the title and the table or text.
`POST …/referrers` registers a consumer of another Zone; same-service consumers are registered by their refresh. On the Workspace's own host
`/` and `/<workspace_name>` open the Desktop page on the Zone host, `/<workspace_name>/<surface_name>` opens that Surface.

### Validator classes (D01, D04)

Every built-in type is checked by a Rust validator in [core/src/types.rs](core/src/types.rs) (tables and rich text also in
[core/src/plan_table.rs](core/src/plan_table.rs) and [core/src/plan_richtext.rs](core/src/plan_richtext.rs)). The checks fall into
three classes: **primitive** — the shape of a storage primitive itself (tree node, keyed object, Table, RichText, AssetRef);
**reference** — a target exists, is readable and is of the admitted kind; **schema** — a business rule of the built-in type
(value ranges, counts, which Surface a thing may sit on). Only primitive and reference checks are part of replay semantics; schema
checks run at admission only. This is the contract M1 (T1-1) implements by splitting `init_entity` / `validate_update` into a
primitive entry and a built-in-schema entry; today every class still runs on both paths. The doc comment of each validator starts
with `Validator class:`.

| Type | Validator | Class | Checks |
| --- | --- | --- | --- |
| table-source | `init_entity` (`TYPE_TABLE` branch) | primitive | field ids unique (`ID_CONFLICT`), field names unique (`NAME_CONFLICT`), order keys, `FieldDef` shape |
| table-source | `validate_table_meta` | primitive | `title_field_id` names a live text field; `data_mode` never changes; `embedded` carries no `source_ref` |
| table-source | `check_query_reference` | schema | `url_query` source: absolute URL without credentials, `query` object without cursor, version `live_head` / `fixed_revision{source_revision}` |
| table-source | `plan_table.rs` `check_unique` | primitive | `unique` is a kernel-enforced cross-record constraint (`UNIQUE_VIOLATION`) |
| table-source | `plan_table.rs` `required` / `nullable` checks (insert, set/unset, add/update/migrate field) | primitive | required values present, nulls only where allowed, tightening re-scans the live records (`SCHEMA_CONFLICT`) |
| table-source | `plan_table.rs` `check_body` | primitive | a record body is a live rich text under this table and the body of no other record |
| table-source | `plan_table.rs` `delete_field` / `migrate_field` | primitive | the title field cannot be deleted and stays a text field |
| richtext | `plan_richtext::create` / `apply_update` | primitive | document conforms to `buckyos.richtext.basic.v1`, block ids unique, CRDT update applies to a known base |
| richtext | `entity_refs` (`richtext_ref_edge`) | reference | `object_embed` / `object_link` targets are indexed as references; existence judged by the planner's `check_ref_target` |
| asset | `validate_asset` | primitive | `object_id` is a `type:hex` object id; `media_type` / `size` come from the verified content, never the client; `image { width, height }` |
| asset | `validate_asset` (object lookup) | reference | the object was uploaded and verified (`DEPENDENCY_UNAVAILABLE`); import tolerates a listed but missing object |
| container | `validate_container` (kind, layout, system nodes) | primitive | tree node kinds `folder` / `surface` / `group`; `root` / `data` / `surfaces` only internal; kind never changes; `layout.mode` |
| container | `validate_container` (`content_folder_id`) | reference | a Surface's content folder exists, is a folder and sits under `canvas-content` |
| container | `validate_container` (`icon`, `config`, `locked`, `surface_id`, `system`) | schema | preset icon id and ≤ 64 KiB page config only on a Surface; `locked` only on a group; `system` only on a folder |
| record | `record_schema` / `validate_record` (values) | schema | `schema.properties` declare the keys; every `p:` value normalizes under its property type; required properties present |
| record | `validate_record` (`object_ref` values) | reference | an `object_ref` property points at an existing entity of the declared target types |
| cell | `validate_cell` (`view`, keys) | schema | known keys, `view.type` is a renderer id, `view.version`, title ≤ 256, `options` / `config` objects, config ≤ 64 KiB, `bindings` ≤ 32 identifiers |
| cell | `validate_cell` (`source_ref`, `bindings`, `def_ref`) | reference | the bound data source exists and has the view's data type; bindings point at data types; `def_ref` is a block definition (judged when written; dangling configuration stays editable) |
| cell | `validate_cell` (table view configuration) | schema | `fields` / `filter` / `sorts` / `group` / `manual_order` only on table views, named fields live, ≤ 4 sorts, group field type |
| cell | `validate_cell` (`config.snapshot`) | primitive | an embedded snapshot is an AssetRef (checked by `validate_asset`) |
| cell (connector) | `validate_connector` (shapes) | schema | `flip`, `route`, ≤ 64 `controls`, `label.t` in [0, 1]; `start` / `end` are `{ entity_id, anchor { kind: named, id } }` |
| cell (connector) | `validate_connector` (endpoints) | reference | a bound endpoint is a readable Block or group on the connector's Surface, never itself or another line |
| cell (connector) | `validate_connector` (Surface rule) | schema | created and bound only on a free-layout Surface; a self loop needs two anchors and a non-straight route |
| cell (frame / any Block) | `check_presentation` / `check_notes` | schema | `presentation.live` boolean; `background` / `notes` / `caption` only on frames; notes ≤ 16 KiB, caption ≤ 2000 chars; none on a connector |
| viewport | `validate_viewport` (fields) | schema | `center { x, y }` finite, `zoom` in [0.05, 4], title, notes, caption |
| viewport | `validate_viewport` (`surface_ref`) | reference | the Surface exists (judged when written) |
| viewport | `validate_viewport` (free layout) | schema | the Surface is a free-layout Surface |
| show-path | `validate_show_path` (fields) | schema | `purpose`, `stage { w, h }` in [16, 16384], `background` color, ≤ 500 steps, step keys, ≤ 200 `hide` ids, unique step ids |
| show-path | `validate_show_path` (step targets) | reference | a new or retargeted step points at an existing Frame or Viewport; untouched dangling steps stay |
| show-path | `validate_show_path` (Frame rule) | schema | a Frame target is on a free-layout Surface |
| annotation | `validate_annotation` (fields) | schema | `kind` note / highlight, body ≤ 4000, `style` object, author fixed by the backend, only the author re-anchors |
| annotation | `validate_annotation` (`target`, `anchor::check`) | reference | the target exists; range / quote anchors resolve on it (strict only outside import) |
| wish | `validate_wish` (fields) | schema | prompt ≤ 20 000 chars, `executor`, `output_mode`, `output { container_id, surface_id, name, type }`, ≤ 200 inputs with unique names, size limits of `analysis` / `last_run` / `executor_config`, `knowledge` / `refinements` / `analysis` shapes (`wish.rs`) |
| wish | `check_input` (selector shape) | schema | a wish input is `entity`, `table_view { cell_id }` or `table_query { filter?, sorts?, fields? }`; `version { mode: follow \| fixed }` |
| wish | `check_input` (targets), `validate_wish` (`program`) | reference | the input entity exists and a `table_view` cell is a table view of it; the program source object was uploaded |
| block-def | `validate_block_def` | schema | `def_id` renderer id, `version`, `kind` declarative / html with the matching body, `accepts` known data types, `default_size`, ≤ 512 KiB |
| any (derived record) | `check_derived` | schema | the dependency record `{ wish_id, run_id, executor, inputs[], … }` of a generated result |
| any (keyed object) | `check_keys` / `opt_text` / `check_locked` | schema | helper checks used by the validators above: known keys, bounded strings, `locked` boolean |
| any (references) | `entity_refs`, `plan.rs` `check_ref_target` | reference | every indexed reference target is a live, readable entity of the admitted types (`REFERENCE_BROKEN`) |

### Known limitations (13.2)

- A Commit that contains `richtext.apply_update` has no inverse: `doc.undo` answers `NOT_UNDOABLE` ([store/src/workspace.rs](store/src/workspace.rs) `undo`). Likewise a `table.migrate_field` whose inverse would exceed 64 MiB (`Limits.max_inverse_bytes`, [core/src/plan.rs](core/src/plan.rs)) is not compensable.
- Import tolerates missing resources: under `import` an AssetRef whose object is not in the package is kept with its listed `media_type` / `size` and reads back as `missing` ([core/src/types.rs](core/src/types.rs) `validate_asset`); dangling view configuration, connector endpoints and step targets are kept verbatim. The referential integrity of an imported package is therefore weaker than that of an online Commit (§6.3: unclosed dependencies are listed, not refused).
- A read-only show has no fixed baseline: a presenter without any Workspace-level write capability (`ws_caps.any_write()` false, [server/src/show.rs](server/src/show.rs)) takes neither the show lock nor a clone and reads the live content; only writers get a frozen baseline (clone for live shows, lock otherwise).
- `proc.start` is idempotent per `(principal, idempotency_key)` and compares a digest of `params`: the same key with other parameters is `IDEMPOTENCY_MISMATCH`, never a silent replay ([store/src/wish/runs.rs](store/src/wish/runs.rs), [store/src/proc.rs](store/src/proc.rs)).

**Selectors (D04, frozen).** The read-set vocabulary — what a precondition, a wish input, a dependency record or a
reference may name inside an entity; `core/src/plan.rs` `resolve_cell` turns each into the version cell (a `rev`) or
content token (a hash) it stands for. New kinds are protocol changes.

| Kind | Names | Resolves to |
| --- | --- | --- |
| `entity` (default) | the whole entity | `content_rev` |
| `doc_key` `{ key }` | one top-level payload key | the key's rev |
| `table_members` | the set of live records | the members rev (any insert / delete) |
| `table_field` `{ field_id }` | a field's definition | `def_rev` |
| `table_field_type` `{ field_id }` | a field's type | `type_rev` |
| `table_field_values` `{ field_id }` | every value of a column | `values_rev` |
| `table_record` `{ record_id }` | one record | the record's rev |
| `table_cell` `{ record_id, field_id }` | one cell | the cell's value rev |
| `table_view` `{ cell_id }` | a saved table view of this table (the Block's filter, sorts, group, fields, manual order and binding) | a hash over the view's keys, the members rev and every live field's type and values (the coarse token a wish input declares; the read set still records the fine cells) |
| `table_query` `{ filter?, sorts?, fields? }` | the rows a query projects | a hash over the members rev and the type and values of the projected fields (all live fields without `fields`) |
| `tree_children` | the live direct children of a node | a hash of their ids (a new member makes a folder input stale) |
| `tree_edge` | where the node sits (parent, order, placement) | `struct_rev` of its edge |
| `richtext_block` `{ block_id }` | one block of a rich text | the block's content hash |
| `folder_member` `{ name }` | one member of a resource directory (`buckyos.dir`, D01, D25) | the member's own rev (`m:<name>`); a removed member is `TARGET_DELETED` |

**Method levels (D11).** `stable` methods are the contract other Apps, Agents and SDKs may depend on: they change
only with `protocol_version`, and each has a contract test against a recorded request / response shape
(`server/tests/contract.rs`, `fixtures/protocol/stable-methods.json`; `AIWS_REGEN=1` re-records). `internal` methods serve the
Desktop and diagnostics and change with it. `experimental` methods are not frozen yet: `proc.*` become stable when M3
lands. The list is `server/src/lib.rs` `METHOD_LEVELS`; this table mirrors it.

| Method | Level | Params (besides `workspace_id`) |
| --- | --- | --- |
| `ws.create` | stable | `title`, `workspace_id?` |
| `ws.list`, `ws.get_info`, `ws.delete`, `ws.fork` | stable | |
| `ws.grant` / `ws.revoke` / `ws.list_grants` | stable | `subject`, `scope_entity_id?`, `capabilities[]`; `list_grants` returns every row (with `via_entry`) and the `owner` to a manager, only the rows that apply to the caller otherwise (`complete: false`, no owner). Subjects: see the table above |
| `ws.list_subjects` | stable | principals a grant can name (static identities / zone users and `group:` subjects) |
| `ws.create_entry` | stable | `capabilities[]`, `methods`, `scope_entity_id?`, `expires_at?` → `entry` (`manage` on the scope) |
| `ws.list_entries` | stable | → `entries` on scopes the caller manages |
| `ws.close_entry` | stable | `entry_id` (`workspace_id` optional, must match) → `entry` |
| `ws.join_entry` | stable | `entry_id` only → `workspace_id`, `entry_id`, `scope_entity_id`, `capabilities`, `joined`, `granted`, `path` |
| `entry.describe` | stable | `entry_id` only, no session → `EntryDescription` |
| `ws.get_user_state` / `ws.set_user_state` | stable | `entries: { key: value \| null }` — user work state per subject and Workspace in `local.sqlite`; never a Commit |
| `ws.set_wish_aicc` | stable | `mode: workspace_owner \| current_user` — whose AI resources wish runs started here use (`manage`; AI Center Quota PRD §3.3) |
| `ws.begin_import` → PUT → `ws.import` | stable | `upload_id`, `semantics: "restore" \| "new"`, `replace?` |
| `ws.release_retention` | stable | maintenance (`manage`, D19): `kind`, `ref` — end a retention root (`snapshot`, `export`, `clone`, `staged`) |
| `ws.delete_checkpoint` | stable | maintenance (`manage`): `snapshot_id` — the snapshot and its retention go; the collector reclaims what nothing else holds |
| `ws.purge_deleted` | stable | maintenance (`manage`): `entity_ids?` (default: every deleted entity) — content and references are given up for good (`entity.purge`), resources are reclaimable at once, undo and version restore answer `RESOURCE_RECLAIMED`, Block state is cleared |
| `ws.clear_staged` | stable | maintenance (`manage`): staged uploads nobody referenced go, expired or not |
| `doc.outline` | internal | — all readable entity envelopes in tree order |
| `doc.resolve` | stable | `reference` or `path` |
| `doc.read` | stable | `entity_id`, `selector?` — or `targets: [{ entity_id, selector? }]` |
| `doc.list_children` | experimental | `entity_id`, `include_deleted?` |
| `doc.list_annotations` | experimental | `target_ids?`, `parent_id?` — annotations anchored to those entities and/or placed under that parent, each with its resolved `anchor` |
| `doc.query` | stable | `view_id` or `source_id`; `filter?`, `sorts?`, `fields?`, `group?`, `limit`, `cursor?`, `consistency?`, `source_revision?` |
| `doc.freshness` | experimental | `entity_ids[]` — dependency-based freshness of results and wishes (computed in core) |
| `doc.relations` | experimental | `entity_id` — outgoing / incoming references, Blocks showing it, produced results; unreadable referrers only as `hidden_incoming` |
| `doc.list_versions` | experimental | `entity_id` — addressable versions (generated results, checkpoints) with their commits |
| `doc.restore_version` | experimental | `entity_id`, `content_rev` — the operations restoring that version (submitted as an ordinary Commit) |
| `doc.source_capabilities` | internal | `source_id` (URL query tables) |
| `doc.get_collab_state` | internal | `entity_id` |
| `doc.prepare` / `doc.commit` | stable | the Commit request (design §2.6) |
| `doc.get_submission` | stable | `epoch`, `idempotency_key` |
| `doc.undo` | stable | `epoch`, `commit_id`, `idempotency_key`, `mode?`, `plan_digest?`, `session_id?` |
| `doc.get_changes` | stable | `epoch`, `after_seq`, `limit?`, `filter: { detail: "touched" }?` — each change carries `touched`, and (unless `touched` only) `ops` and the row delta `rows` (D20); `after_seq` below `base_seq` is `BASE_TOO_OLD` |
| `doc.wait_changes` | stable | `epoch?`, `after_seq`, `timeout_ms?` (long poll) |
| `doc.checkpoint`, `doc.export` | experimental | `mode: "share" \| "personal_backup"`, `self_contained`, `include_notes?` |
| `asset.begin_upload` → PUT → `asset.finish_upload` | stable | `size`, `upload_id` |
| `asset.import_object` | stable | `workspace_id`, `obj_id` (ready NDM FileObject) → workspace `object_id`, `source_obj_id`, `size`, `media_type` |
| `replica.bootstrap` | internal | — then `GET /replica/<workspace_id>/<replica_id>` |
| `proc.start` / `proc.get` / `proc.apply` / `proc.cancel` | experimental | `program`, `params`, `idempotency_key`; `run_id`, `session_id?` — wish programs below |
| `proc.list` | experimental | `wish_id?`, `limit?` — wish runs, newest first |
| `proc.claim` | experimental | `run_id`, `claim_token`, `executor?` — an executor takes the run (M3): the next attempt, a `run_token` (bearer for `proc.heartbeat` / `proc.report` / reads of this Workspace) and a lease (90 s) |
| `proc.heartbeat` | experimental | `run_id`, `attempt`, `progress?` — renews the lease; a lapsed lease makes the run `interrupted` (D15) |
| `proc.report` | experimental | `run_id`, `attempt`, then `progress?`, or `error`, or `finished: true` (the stage host collected the candidate: T3-4), or the candidate itself: `results[]` + `read_set?` + `summary?` + `warnings?` + `assumptions?` (execute runs; the Mock executor's shape), or `analysis` (analyze runs). The service validates and plans it; a superseded attempt or a cancelled run gets `RUN_REVOKED` |
| `proc.tool` | experimental | `run_id`, `attempt`, `name`, `args` — one stage tool call of the run's executor (the claim of a `wish.xllm@1` run opens the stage and answers `stage: { kind: model, system, user, tools, model, iterations }` or `{ kind: program, source }`): `ws_*`, `submit_analysis`, `put_result`, `check_results`, `finish`, and `llm_map` for a program's `aiws.llm.map`. Answers `{ status: ok \| error, text }` — `error` is for the model to correct |
| `proc.program` | experimental | `run_id`, `attempt`, `source`, `results?` (what the program host wrote to `output/.aiws/results.json`), `log?`, `files?` (`{ "output/<path>": text \| { base64 } }`) — the executor ran the program; judged like the service's own `run_program` |
| `proc.files` | experimental | `run_id`, `attempt` — the stage directory (`context/`, `request.json`, `catalog/`, `lib/`) as `{ path: { text } \| { base64 } }` for the executor's program host |
| `ws.set_hosted_exec` | experimental | `config: { enabled, agent_id, allowed_wishes?, triggerers?, limits? }` (`manage`, D22) — the Owner's hosted execution; `ws.get_info.hosted_exec` returns it with its `digest` |
| `ws.publish` | experimental | `entity_id`, `name?` (`manage`, D26) — the `public` subject may read the entity; the name (default: the entity's, slugified; a new name keeps the old as an alias) is the last segment of the link. Returns `name`, `path` (`/<workspace_name>/<name>`), `url` (on `https://aiws-<workspace_id>.<zonehost>` when the deployment knows its host) and the reference object |
| `ws.unpublish` | experimental | `entity_id` (`manage`) — every name and the public grant go; referrers keep their records and read `unavailable` |
| `ws.list_publications` | experimental | the names, entities, `url` and referrer counts (what the caller may read) |
| `ws.list_referrers` | experimental | `entity_id` (read) — who registered as reading it (`{ zone?, workspace_id, entity_id?, principal? }`, weak: never blocks an unpublish) |
| `lock.acquire` / `lock.renew` / `lock.release` / `lock.break` / `lock.list` | stable | `entity_ids[]`, `session_id`; `lock_ids[]`; `entity_id` |
| `show.start` / `show.heartbeat` / `show.publish` / `show.command` / `show.watch` / `show.notes` / `show.end` | internal | see “Non-public shows” |
| `diag.list_unretained`, `diag.verify_refs` | internal | |
| `diag.gc` | internal | `dry_run?` (default true) — the service-wide collector (an owning account) |

HTTP routes (same port, `Authorization: Bearer <session token>`):

| Route | Use |
| --- | --- |
| `PUT /kapi/aiworkspace/upload/<upload_id>` | asset bytes or an import package |
| `GET /kapi/aiworkspace/asset/<workspace_id>/<object_id>` | verified asset bytes |
| `GET /kapi/aiworkspace/export/<workspace_id>/<export_id>` | export package (zip) |
| `GET /kapi/aiworkspace/replica/<workspace_id>/<replica_id>` | replica database built by `replica.bootstrap` |
| `GET /kapi/aiworkspace/schemas/richtext.basic.v1.json` | rich text schema definition (no auth) |
| `POST /kapi/aiworkspace/wish-host/<token>/llm_map` | `aiws.llm.map` of a running wish program; the token is the run's (no session) |

Operations (`operations[]` of a Commit): `entity.create|delete|restore|rename|set_keys|unset_keys|set_write_policy|set_derived|clone`,
`data.pin`, `tree.move|place`, `dir.set_member|remove_member|replace`, `table.insert_records|delete_records|set_values|unset_values|set_body|add_field|update_field|delete_field|add_option|update_option|delete_option|migrate_field`,
`richtext.apply_update|insert_blocks|replace_block|delete_blocks|move_block`. Parameters are exactly those of the design §3;
`core/tests/planner.rs` and `store/tests/*.rs` are executable examples of every one of them.

## Where this implementation differs from the design text

- **Export/replica download routes carry the `workspace_id`** (`/export/<workspace_id>/<export_id>`): there is no service-level database to look an export id up in.
- **Object storage is a content-addressed folder with CYFS ids and file names**, verified against ndn-lib in `store/tests/kernel.rs` (V01). It is not yet backed by the zone named store, so nothing is pinned there and assets are not fetchable through NDN. `FsObjectStore` is the boundary to replace. It is service-wide, so a show's clone shares every object without copying (第三期规划 §8.3 asked for the named store here; see the third phase report).
- **Files are single `mix256` chunks (≤ 32 MiB)**; ChunkList for larger assets and record files is not written.
- **Undo `partial` mode** determines the applicable part by probing each inverse operation on its own.
- **URL query sources** only reach adapters registered in the service (`fixture://` generated source for tests). There is no general HTTP fetcher: a URL in a document cannot make the service issue arbitrary requests.
- Extra methods: `doc.outline`, `doc.source_capabilities`, `diag.verify_refs`.
