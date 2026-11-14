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
store/     aiworkspace-store  SQLite storage, objects in the named store retained by bucket bindings, packages, Mock runs, URL sources;
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
keeps Workspace folders under the service data folder and objects in the node's named store, retained by
the service's buckets at nfs-server (`spec_config.buckets`, see Storage), listens on port 4120, is reached
through the generic gateway route `/kapi/aiworkspace`. Every request is authenticated by the service itself
(verify-hub session token); the gateway does not do it.

`AIWS_FAILPOINT=before_txn|in_txn|after_commit[:abort][:N]` injects a fault into the N-th commit.

Standalone objects: `--nfs-server <origin> --named-store-config <json>` use a standalone nfs-server started with
`--named-store <dir> --bucket-specs <json>` (the specs declare the `aiworkspace` buckets of `buckyos_api::aiworkspace_bucket_decls`;
the config is NamedDataMgr's `stores` array on that same directory). It is the NDM gateway of uploads and the `/bucket/v1`
data plane of downloads. Without `--nfs-server` the data folder gets a named store and a bucket index of its own (`store/`):
everything but NDM uploads and bucket downloads works (the Rust tests). The Desktop Playwright backend
(`tests/aiworkspace/start-backend.mjs`) starts the nfs-server and supplies these options; the server tests start one in-process
(`server/tests/common/nfs.rs`).

## File and folder imports

Nothing is copied: the FileObject the browser uploaded is the asset (`object_id` is its id). A single file is uploaded through WebSDK NDM with `bind` into the caller's own `user/aiworkspace/<user>/assets` (key `<workspace_id>/<object_id>`), then `asset.import_object { workspace_id, bucket_id, key }` registers it: the service reads the binding (`bucket.get`), checks workspace update/structure access, binds the object under `<workspace_id>/<object_id>` in the Workspace owner's `assets` bucket (a collaborator's upload moves there and its own key goes) and lists it as staged. A folder or several files are uploaded without `bind` (NDM's 24 h hold covers them) and each FileObject is registered with `asset.import_object { workspace_id, obj_id }`: the service authorizes the object at the trusted NDM gateway with the caller's own token, then binds it. The per-asset limit is `max_asset_bytes` (32 MiB), including each leaf of a directory; an audio or video asset (by the sniffed media type) may be up to `max_media_bytes` (2 GiB, 控件机制 TODO Q16) and is played by Range from `asset.stream_url` (`services/aiworkspace/settings` holds both). Such an upload is the browser's multi-chunk FileObject: a self-contained `doc.export` lists it as missing (packages carry single chunks only).

Directory roots and descendants become `buckyos.container` folders, and files become AssetRefs in one normal business commit. A canvas import also creates corresponding groups and asset cells. Online imports may be cancelled; after failure or refresh, reselecting the source uses NDM deduplication. There is no offline upload queue or automatic recovery of browser File handles. FileList-based folder pickers cannot enumerate empty folders; callers with an explicit directory tree can preserve them through the SDK. draw.io and workspace ZIP retain their format validation, while image replacement and generated snapshots use the same asset stage/commit semantics.

Table data (表格数据导入, `notepads/done/aiworkspace-table-import-todo.md`) is imported on the client: [import-table/](import-table/README.md) turns pasted HTML / TSV, CSV, xlsx and SQLite into a TableSource (header, field names, inferred types including `currency`, values in their stored form) and commits `entity.create` + `table.insert_records` (≤ 500 records / 1 MiB each) as **one Commit per import**, whatever its size: the client sends a Commit larger than one request in parts (`doc.stage`, see “Commit size” below). The service adds nothing for it: every value is checked by the usual write rules. The Desktop's wizard and the Agent command `import_table` (src/tools/buckyos-agent) run the same module.

Bytes the service produces itself (Source fetches, wish results and program sources, the assets a package brings, test fixtures) are staged by `Workspace::stage_asset`: stored as the single-chunk FileObject of `canonical::file_object` and bound the same way. A workspace package is uploaded with `bind` into the importer's own `exports` bucket and imported with `ws.import { bucket_id, key, semantics, replace? }`; a successful import consumes the package (its binding goes), so the same upload cannot be replayed onto newer work. A restore journal recovers interruption between backing up the old directory and promoting the new one. Restore holds the workspace writer lock while rechecking permission, copying ACLs and replacing its state; queued writers observe the new epoch; the restored Workspace keeps its owner (whose buckets hold its objects). Final asset submission rechecks current workspace permission.

## Tests

```bash
cargo test -p aiworkspace-core      # values, canonical ids, filters, rich text codec, planner, annotation anchors, phase2 (two trees, wishes, freshness),
                                    # tests/ink.rs: pen strokes (fixtures/ink) committed and read back, malformed ink refused
cargo test -p aiworkspace-store     # V01–V22, V24, write locks, Mock — against real SQLite files; tests/sso.rs: caller classes,
                                    # group / contact subjects, share entries;
                                    # tests/replica.rs: the offline engine incl. the incremental rows a replica persists
cargo test -p aiworkspace           # V23 + crash recovery — against the real process over HTTP; tests/sso.rs: the same
                                    # with static Limited / External identities, entry.describe, ws.join_entry, denials;
                                    # tests/wish.rs, tests/quality.rs: wish runs through the real xllm loop and Deno
                                    # with a scripted model (needs `deno` on PATH or AIWS_DENO)
(cd frame/aiworkspace/aiws && deno test -A)   # the program host (temp dirs, a local test server)
# extension controls: core tests/extension.rs, tests/composite.rs (composite definitions and instances, slots, deps,
# REFERENCE_CYCLE, replay without the business schema), tests/clone.rs (tree.clone), tests/apply_log.rs;
# store `cargo test -p aiworkspace-store --lib pack` (the ext.pack archive); server tests/ext.rs
# (build, library, .pikg install, composite_controls_with_dependencies incl. ext.pack → ext.install)
# standard tables and Workspace names: core tests/standard.rs (record rules on final values, undo, replay without them),
# `--lib standard` (the RRULE subset); store tests/names.rs (ws.ensure under concurrency, conflicts, delete, packages)
```

## The two trees (phase two §4)

Every Workspace has the fixed system nodes `root` → `data` (the data tree) and `surfaces` (the Surface
collection); `canvas-content` is a system folder under `data` holding one folder per Surface, `shows` another one
holding the presentation paths and Viewports (below). They exist from
`seq` 0 and cannot be created, deleted, moved or renamed by operations. Structure rules (`core/src/plan.rs`
`child_allowed`): `data` / `folder` take folders and data entities (TableSource, RichText, Record, AssetRef,
Annotation, `buckyos.wish`, `buckyos.block-def`); `surfaces` takes `surface` containers and canvas folders; `surface` / `group`
take Cells, groups and controls only (a control's Cells may hold Cells, see “Block-controls”). A **canvas folder** (UI 框架 PRD
§4.1.1, Q5) is a `folder` under `surfaces`, at any depth: it holds Surfaces and canvas folders only, never carries `system`,
cannot move into the data tree (nor a data folder into `surfaces`) and is deleted only when empty (`FOLDER_NOT_EMPTY`).
A Surface's path runs through its folders (`/surfaces/日记/2026-11/11-13`); grants on a folder cover the Surfaces in it
(structural inheritance); links name the Surface by id, so moving it keeps them working. Its content folder stays flat
under `canvas-content`. A Surface names its content folder (`content_folder_id`); the folder may point back
(`surface_id`). A Surface may carry `icon`: the id of a client-side preset icon (`[a-z][a-z0-9-]{0,31}`, Surfaces
only; the client owns the preset list), a shared, undoable key projected into the outline like `title`. A Surface may also
carry `config`, its page settings (`background`, `page_size`, `toolbar`; an object of at most 64 KiB the client checks, read with
`doc.read`, not projected; draw.io 导入 TODO §2.5; changing `config.toolbar` needs `structure` on the Surface, see
“Workspace settings”). `placement` is `{ x, y, w, h, rotation? }` relative to the parent (`rotation`: degrees clockwise about the centre,
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

Format version is `1.0` (内核架构改进 FORMAT 1.0: `schema_id`, the resource directory, the reference catalogue with `mode` / `integrity` and `owns`, the pure-application log, `$formula`, the `run` capability, delegation fields, the usage ledger, `user_state.revision`, `base_seq`; protocol `1.0`). Stores and packages of an older format are refused with `UNSUPPORTED_VERSION` (no migration). From 1.0 on, an incompatible change of the format needs an explicit conversion (D07); within 1.0 only additions an older implementation can ignore are allowed. Protocol 1.1 added Block-controls (container kind `mindmap`, `"auto"` placement components, Cells holding Cells inside a control, `slot`; see “Block-controls”); requests speaking 1.0 are still accepted. Protocol 1.2 (storage schema 6) is “Finding, reading and querying”: `doc.list`, `doc.find`, `doc.read { as, path }`, `doc.query` with `text` or `sql` only, `doc.source_query`, every entity named; Commit requests of 1.0 / 1.1 are still accepted. Extension controls P1 added, within the same versions, the container kind
`composite` (`def_ref`, `bindings`, `slot` on Block containers), the block-def keys `deps` / `composition`, the reference
kind `def_dep` (live, strong), the error code `REFERENCE_CYCLE`, the operation `tree.clone`, `tree.move { slot }` and
directory member names holding `/` (see “Extension controls”). 1.1 also added the value type `currency` (表格数据导入 TODO §6, Q2: format and protocol versions unchanged): a field `{ type: "currency", currency: "<ISO 4217>", scale?: 0–18 (default 2) }` stores `{ "amount": "<decimal of that scale>", "currency": "<code>" }` — the code always written out (a bare amount or number takes the field's default; changing the default, `table.update_field { changes: { currency } }`, never changes stored values); symbols and names (`$12`, `USD 12`) are refused. Values order by (code, amount); `lt / lte / gt / gte` only meet amounts of the operand's currency; `eq` / `ne` / `in` need the same code; no `unique`, no grouping. Migrations: `number | decimal → currency` (the target's default), `currency → decimal` (drops the code), `currency → text` (`"USD 12.00"`), `currency → currency` (another scale).

The value type `json` (控件机制 TODO G8, versions unchanged): a field `{ type: "json" }` holds a JSON **object**, replaced whole (no path edits; concurrent writes of one cell conflict on its version) — at most 16 KiB canonical (`LIMIT_EXCEEDED`), nesting at most 8 levels (the object is one), every string within the text limit; `null` only on a nullable field; a `{ "$formula" }` object is refused (`INVALID_SCHEMA`), formulas compute no extension fields. Not sortable or groupable, no `unique`, `scale`, `options`, `currency` or `target_types`; filters only `is_empty` / `is_not_empty`; no migration to or from it.

**Records owned by their submitter** (控件机制 TODO G20, Q14; versions unchanged). A `text` field may be declared `submitter: true` (at most one per table, never on a record property). The service fills it with the inserting principal on `table.insert_records`; a client value for it — on insert, `table.set_values` or `table.unset_values` — is `INVALID_OPERATION` (“the submitter is filled by the service”), whoever writes: undo, import and replay keep the recorded value. It is unique: a principal's second insert is `INVALID_OPERATION` / `SUBMITTER_EXISTS` with `data.record_id` of the record it already has. A caller holding `append` but not `update` on the table may `set_values` / `unset_values` only records whose submitter is itself, and one holding `append` but not `delete` may `delete_records` only those (others `PERMISSION_DENIED`); with `update`, any record's other fields. `submitter` is set or cleared (`table.add_field`, `table.update_field { changes: { submitter } }`) only while the table has no live records (`SCHEMA_CONFLICT`). Reading is unchanged: no record-level read grant exists, so an `append`-only caller without `read` sees the record it inserted in the Commit answer (`touched`) only, never through `doc.query` / `doc.read` (a vote readable by everyone who can read the canvas, Q13, needs `read` anyway).

A `multi_select` field may declare `max_items` (1–1000): a value with more distinct options is `INVALID_SCHEMA` (“at most N options”). Setting or lowering it with `table.update_field { changes: { max_items } }` (`null` removes it) re-scans the live records like `required`.

**Standard business tables** (控件机制 TODO G8–G10, [schemas/standard-tables.md](schemas/standard-tables.md)): a table whose `schema_id` is `buckyos.todo@1` or `buckyos.calendar@1` has standard fields with fixed ids and types and the business rules of that document, checked at admission only ([core/src/standard.rs](core/src/standard.rs), schema class, see “Validator classes”). The record rules judge every record a Commit inserted, changed or restored once, on its **final** values after the whole Commit (moving `start` and `end` together, or turning a timed event into an all-day one over a `table.set_values` and a `table.unset_values`, is one change; its undo replays the inverse operations in another order); a failure is `INVALID_SCHEMA` naming the record and field (`data: { entity_id, record_id, field_id }`) and the last operation that touched the record (`op_index`). `null` counts as absent. Of `ext`, only the keys the document gives a meaning are checked (an RRULE outside the subset is refused with a message naming the part); other keys pass.

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
  dropped. Objects are in the named store and the clone reads them through the source's bindings, nothing is copied or bound again (10,000 × 20 table: ~45 ms,
  release build). A clone is not listed by `ws.list`, cannot be prepared for offline or presented itself.
- The relay, the lock and the clones live in memory: the lease is 60 s (`AIWS_SHOW_LEASE_MS` shortens it for tests);
  a sweeper ends shows that were not renewed (lock released, clone deleted); a restarted service drops every clone left
  on disk. A prompter authenticates with `params.prompter_token` instead of a session token, and only for
  `show.command`, `show.watch` and `show.notes` of its show.

## Presentation sessions (第四期规划, `notepads/done/aiworkspace-phase4-todo.md`)

A **session** is a public presentation room of one Workspace's presentation path: a row of the service's registry
(`sessions.sqlite`: state, owner, presenter, policy, baseline, archive, links, participants, posts) and a **run
Workspace** (`purpose = session`) the service keeps for it. `session.create { workspace_id, path_id, title?, presenter?,
policy?, readers_of? }` (workspace-level `read` + `update` on the source; the caller becomes the owner) clones the source whole
(`VACUUM INTO` in its writer) and, before anyone opens the clone: gives it a new epoch, cuts its history to the clone
point (`base_seq`; earlier commits, versions and snapshots go; rich texts become shallow snapshots), resets its grants to
the owner (all) and the presenter (all but `manage`), drops entries, publications, runs, locks, user state and uploads,
binds every live asset under the run Workspace's own key in the owner's `assets` bucket, and moves every Frame
`presentation.notes` and Viewport `notes` into a presenter-scoped table in the `session` folder (one Commit of origin
`system`). Its full rows at that point are kept as `session/baseline.json` (the replay baseline) and checkpointed. Run
Workspaces are not listed, not swept as show clones and not counted among them.

`policy`: `guest_interact` (false: guests watch only), `voice` (true), `show_names` (true), `expected_audience` (50),
`media_limit_mib` (2048), and `readers_of` — set by the service, never in the `policy` parameter: the session's
permission object (see “Readers of the permission object”). States: `prepared → live → ended → archived`, or `expired`
(an ended, unsaved session past `session_retention_days`, 7 by default: the run Workspace goes, the row stays); `access`
(`open | closed`: new audience) and `posts` (`open | closed`) are switches of their own, `session.delete` a third.
`session.set_access { access?, posts?, guest_interact?, readers_of? }` (owner, presenter or prompter; `guest_interact`
the owner only; `readers_of` the owner or the presenter in person, not a prompter link) answers `{ access, posts,
policy, readers_of }`.

**Roles and links (Q14).** The presenter controls the stage and runs tasks; the owner (when not the presenter) manages,
saves, deletes and reads everything; participants joined through a link. `session.invite { kind, label?, config? }`
(owner or presenter) makes a link independent of the Workspace's sharing: `at_…` (audience: `session.join { session_id,
link, name? }` with the visitor's own token — an account, an External DID, or a `gt_` guest — writes a `read` grant on the
run Workspace with `via_entry = <invite id>`), `dt_…` (a public display: the token itself is the session token, the
caller is `link:<invite id>`, it reads under that subject; `config.voice`), `pt_…` (a prompter: the presenter's remote —
commands and notes — and a reader like a display). `session.revoke_invite` deletes the link and every grant it gave.

**Scopes.** Entities of a run Workspace may be `presenter` (read by Workspace-level writers and managers only:
`Access::reviewer`) or `participant:<principal>` (that participant, reviewers, and the principals of its `readers`); both
exist only in the `session` system folder, created on demand by the operation `system.ensure { id: "session" }` (origins
`session` / `source` and internal paths only). Reads, change streams, replica bootstrap and replay all apply them.

**The stage (Q01-C).** `session.command { command: { command_id, kind, args? }, rect?, terminal_id? }`: `next prev first
last goto{step_id} black{on?} back free{on?} media{entity_id, action: play|pause|seek|rate|ended, media_pos_ms, rate} sync`;
the service runs the step rules on the run Workspace's path (enabled steps with live targets), decides the transition
(the step's own, else fly on the same Surface, fade across) and keeps the stage in `workspace_meta.session_stage`. One
command id is one command. One presenter terminal controls (the first to connect; `session.takeover` moves it); a prompter
link always commands. `session.sample { kind: pointer|camera|ink|buffer, data, t_rel_ms? }` records a continuous state
(the presenter's pointer, camera, a stroke in progress; an audience member's own camera or buffering on its own track).

**Tasks (Q07–Q09).** Types `buckyos.task`, `buckyos.response`, `buckyos.result` live in the `session` folder and are written
only with origin `session` (`SESSION_ONLY` otherwise): `session.task { op: create|update|assign|revoke|close|reopen,
task_id?, task?, principals?, command_id }` (presenter; `task.mode: open | assigned | claim`, `slots`, `pick`, `ext` are the
extension's); `session.claim { task_id }` (the atomic slot: inside the run Workspace's single writer, `CLAIM_FULL` when
taken; claiming again is the same claim); `session.respond { task_id, content, command_id }` (one response entity per task
and participant, scope `participant:<p>`, a new `version` each time; `NOT_ASSIGNED`, `NOT_CLAIMED`, `ANSWER_REVOKED`,
`TASK_CLOSED` — the late attempt is recorded); `session.review { response_id, review }` (private until `published`);
`session.publish { task_id, title?, content, command_id }` (a result entity: what was made public, never the answers).
Guests interact only with `policy.guest_interact` (`GUEST_NOT_ALLOWED`), and their responses say `level: guest`.
`session.react { kind: feedback|danmaku, text?, target_id?, x?, y?, command_id? }` records a reaction.

**Feeds (§5.3, Q19, Q20).** A Table with `source { kind: "session", feed, mode: "live" }` is a session feed: `participants`,
`stage`, `tasks`, `my_responses`, `responses`, `results`, `reactions`, fields fixed in `store/src/session/feeds.rs`. Like
every Source it is read-only for people and Blocks; the session writes it — in the same Commit as the task or answer it
reflects (origin `session`), or on its own (presence and reactions at most once a second). `session.feed_sample {
workspace_id, feed }` places a courseware's sample (one per feed; `participants` and `responses` in the presenter scope).
In a session `my_responses` is one table per participant (`participant:` scope, id derived from the sample's), named in
`session.join`'s `my_feeds: { <sample id>: <own id> }` for the client to redirect the binding.

**Timeline (§6).** A run Workspace's `doc.sqlite` has `session_events` (`seq`, `t_rel_ms`, `received_at`, `actor`, `scope`
`public | presenter | participant:<p> | diag`, `track`, `type`, `command_id`, `schema`, `data`, `commit_seq`); every Commit
writes its `commit` event inside its own transaction (no "data written, event not" window). Demonstration time is the
service's clock since `session.start`; it stops at `session.end` (the length of the replay and the archive's `duration_ms`). `session.events { after, limit?, until_t_rel_ms? }` pages it as the caller may read
it (a Commit only when it touched something readable); `session.watch { after, timeout_ms? }` is the long poll.

**The channel (Q04).** `GET /kapi/aiworkspace/session/ws`: first `hello { session_id, token?, link?, terminal_id, purpose:
presenter|audience|display|prompter, after?, name? }`, answered by `welcome { session, role, principal, level, controller,
server_ms, t_rel_ms }`, the timeline after `after`, then live `event`s (reviewers get everything; others public events and
their own; a Commit's details only to reviewers). The client sends `ping { c }` (→ `pong { c, server_ms, t_rel_ms }`; online
= a heartbeat within 30 s), `follow { on }`, and `call { id, method, params }` for any `session.*` method (→ `result { id,
result | error }`). Presence (`presence.join` / `presence.leave`, presenter track) follows the first and last connection
while the session runs; after `session.end` people watching the replay are not recorded.

**After the class (§8).** `session.save` (owner, after `session.end`) writes the archive manifest
(`buckyos.ai-workspace/session-archive/1`: baseline, final snapshot, event range, voice segments, gaps, missing objects,
`complete`) and binds a `session` retention set (baseline, final content and every asset) under
`<run ws>/session/<session id>` in the owner's `objects` bucket — no 30-day history rule applies; deleting the session
releases it. `session.replay` returns the baseline rows as the caller may read them now; the client applies
`doc.get_changes` rows of the run Workspace up to any `seq` (the WASM replica) and the timeline's samples on top.
`session.post { kind: comment|rating|moment, body?, score?, t_rel_ms? }`, `session.list_posts` (ratings only to the
presenter and their author), `session.delete_post`.

**Lists.** `session.list { workspace_id }` (历次演示): the sessions of one courseware the caller owns, presents or joined
(all of them for its managers). `session.list_mine { limit? }` (我的公开课, 100 by default, at most 500): the sessions the
caller owns, presents or joined across Workspaces, newest first, each with `my_role` (`presenter`, else `owner`, else
`audience`) and, when the deployment knows its public base, `url` — the room on its courseware's origin
(`<public_base>/<slug>?session=<id>`). `session.list_open { limit? }` (所有公开课, UI 框架 PRD §9.1, K7; same limits): every
session the caller may open without a link, across Workspaces — those it owns, presents or joined (expired ones too) and,
while their room is open (`access: open`) and not expired, those whose permission object it reads — live ones first, the rest by
start (else creation) time, newest first. Each carries `my_role` (`presenter`, `owner`, `audience`: joined before,
`member`: a reader of its permission object who has not), `presenting` (the caller presents it), `opened_at` (the caller's last time in
it, or null), `online` (connections now, while it has a room in memory; else null), `cover` (null: not generated yet) and
`url`. None of the three is a channel call.

**Readers of the permission object (K7, UI 框架 PRD Q19).** Each session has a permission object: an entity of the
source Workspace — a Surface, a folder, a Block, or the Workspace itself (`root`) — kept by id in `policy.readers_of`, so
renaming or moving it changes nothing. `session.create`'s `readers_of` names it as a path or an entity id the caller may
read there (resolved like an entry's resource); by default it is the Surface of the path's first playable step (the
Workspace when no step is on a Surface). `session.set_access { readers_of }` chooses another, resolved the same way for
the caller. A person (not a guest, a link or a run) whom the source Workspace names in its own right — what `ws.list`
lists — holding `read` on the object (path inheritance, deny first, as every read) may open the session without a link
while the room is `open` and not expired: `session.get` answers them as `audience`, `session.join` makes them
participants and puts them in the run Workspace's `source-readers` group (`read` on the whole run Workspace, like a
link's group). An object that is deleted or gone admits nobody this way until another is chosen; those who joined stay.
Their memberships (`group:` / `contact:`) are settled against the courseware's entries. The presenter and the owner see
the object in `session.create`, `session.get` and `session.set_access` as `readers_of { entity_id, path, title, type_id,
kind, alive }`; nobody else is told.

## Presence and the work log (UI 框架 PRD §4.2, §4.5, §12, K2, K3)

**Presence** is in memory, like the shows: nothing of it reaches the document, its history or a package, and a restart
forgets it. Every window of a Workspace calls `presence.heartbeat { workspace_id, conn_id, surface_id?, name? }` when it
opens and then every `interval_ms` (10 s); a connection without a heartbeat for `ttl_ms` (30 s, `AIWS_PRESENCE_TTL_MS` in
tests) is gone, and `presence.leave { conn_id }` removes it at once (a connection id is its principal's own). `surface_id`
is kept only when it names a live Surface the caller may read; `name` only from a guest (`≤ 32` characters, the name it
chose); an account is shown by its principal. The answer — and `presence.list` — is `{ members, interval_ms, ttl_ms,
server_ms }`: one member per principal (`devices` counts its windows; its `surface_id` is that of the window that changed
Surface last), the caller first, then by arrival. Each member: `principal`, `self`, `guest`, `name`, `surface_id` (only a
Surface the reader may read, else `null`), `devices`, `since`, `last_seen`, and — when the reader manages the Workspace,
or about the reader itself (Q6) — `owner` and `groups: [{ group_id, title }]` (the logical groups it belongs to now; the
client shows `owner` as “所有者” and no group as “访客”).

**The work log** `ws.work_log { workspace_id, before?, limit?, filter? }` is one timeline, newest first, read from what is
recorded anyway — never a copy of its own (`store/src/worklog.rs`):

| `kind` | From | Who sees it |
| --- | --- | --- |
| `commit` (`import` for origin `import`) | `commits` (with `message`, `origin`, `seq`, `commit_id`, `run_id`, `undoes`) | when it touched something the reader may read now (as `doc.get_changes`) |
| `run` (`phase: start \| end`, `stage`, `state`, `error_code`) | wish runs in `runs`: the start at `created_at`, the end when the state last left `queued … validating` (`detail.state_at`) | as `proc.get`: one's own, or any with `update` / `manage` on a readable wish |
| `acl` (`action`, `detail`) | `acl_audit` | managers |
| `session` (`phase`, `title`, `session_id`) | the session registry (`started_ms`, `ended_ms`) | as `session.list`: owner, presenter, participants, managers |
| `show` (`phase`) | the non-public shows since the service started (in memory, 200 per Workspace) | readers of the presentation path |

Every entry has `id`, `at`, `actor`, `actor_kind` (`agent` for origin `agent`, `macro` for origin `program` and for runs,
`system` for origins `source` / `session`, else `person` — an undo, a purge or an import is that person's), `mine` and
`target` (`{ entity_id, type_id, name, title, kind, deleted, surface_id }`: the first readable non-system entity a Commit
touched, a run's wish, a show's path; `surface_id` is the Surface it is on — itself, the Surface above a Block, or the one
whose content folder holds it — when readable) or `null`; a Commit also has `targets` (how many readable entities it
touched). A run of the Mock executor (`wish.mock@1`) and a Commit of its results (or of `mock.*`) carry `simulated:
true`: no macro ran (UI 框架 PRD Q27), the client does not call it one. `filter: mine` keeps the caller's own entries; `automation` keeps Commits of origin `agent` / `program` and the
runs. Pages: entries strictly older than `before` (the `next_before` of the previous page, `"<at>|<id>"`); each source
looks at no more than 1 000 rows per call, so a page after a long stretch of unreadable rows may come back short (even
empty) with `more: true` and a `next_before` to go on from. Commit messages are the author's text in the author's
language (the Desktop writes the label of the edit); the client says the service's own messages and every other kind
of entry in the reader's language.

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

## Block-controls (脑图方案, `notepads/done/aiworkspace-mindmap-todo.md` M0)

Design: [脑图对象方案讨论](<../../../doc/workspace/脑图对象方案讨论.md>) v0.4. A **control** is a BlockTree container
whose `kind` is a control id (`mindmap`; `composite` — the host of every composite extension control, see
“Composite controls” below; and the data controls `todo` and `calendar`, see “Data controls” at the end of this
section; the list is `controls/core.json`, read by the core with `include_str!` and by the
Desktop): it lays out the Blocks it holds with a named algorithm the clients run (the shared module `controls/`), the
core only judges shapes. A mind map's nodes are ordinary Cells: `parent_id` is the
tree (a node's children are Cells under it), `order_key` the sibling order; edges are drawn, never stored.

- **Container** (`types.rs` `validate_container`): `kind: "mindmap"`, `layout` implied (`{ mode: "tree" }`, any
  other value refused), `config` checked (`direction` right / left / both / down, `edge` curve / elbow / straight,
  `spacing { sibling?, level? }` in [0, 2000], `branch_colors` ≤ 64 colours; ≤ 64 KiB) and projected into the
  outline, `locked` as on a group. A control sits on a Surface, in a group, or inside another control (R24).
- **Children** (`plan.rs` `child_allowed`): a control holds Cells, groups and controls (a data control holds none); inside a tree control (the
  mind map) a Cell may hold Cells and containers too (its nearest container, walking up through Cells, is a control
  laying out a tree — `types::in_tree_control`). Anywhere else, a composite included, a Cell stays a leaf; `tree.move`
  of a Cell that holds Blocks out of a tree control is `CHILD_NOT_ALLOWED`. Connectors are drawn on the canvas, never
  created inside a control.
- **`"auto"` placement** (`plan.rs` `check_placement`, protocol 1.1): each of `x`, `y`, `w`, `h` is a number or the
  string `"auto"` — not stored, computed by the control when shown; a number pins it. `x` / `y` may be `"auto"` only
  on a Block laid out by a control; `w` / `h` only on a control (its box wraps its children) or on a text view laid
  out by one (`text_views` in `controls/core.json`: `mindnode`, `shape`, `richtext`, `label` — their size is their
  title's).
  A non-text view keeps a numeric size (also when its `view` changes). A control's own `x` / `y` name its origin
  (the first root's top-left), not its box. `"auto"` cannot leave a control (`tree.move` judges the new place). The
  children of a composite whose definition uses the `fixed` layout are not laid out: their `x` / `y` stay numbers.
- **Slots** (R22, reference class — `types::check_slot`): a Block inside a control may carry `slot`
  (`^[a-z0-9][a-z0-9_-]{0,31}$`); projected into the outline. Inside a mind map (any non-composite control) it is unique
  among the live siblings including those staged in the same Commit (`SLOT_CONFLICT`); paths like `bkg.text` resolve
  one level per segment. Inside a composite it is required on every child (Cells, groups, controls) and names a slot of
  the definition; several children may share one (see “Composite controls”). `tree.move { slot }` rewrites it in the
  same step (a name, or `null` to drop it).
- **Outline**: a control's envelope carries `config`; a Cell's carries `slot`, and `side` / `branch_color` when its
  `config` has valid ones (a node's layout hints: the layout runs on the outline alone); a composite's carries
  `def_id`, a Block container's `slot`.
- **Versions** (R12): new kinds and the `"auto"` literal are additions within FORMAT 1.0; PROTOCOL is 1.1
  (`model.rs` `supports_protocol`: requests of the same major and an older minor are accepted). A client that does
  not know a control kind draws it as a group.
- Tests: `core/tests/mindmap.rs` over `fixtures/mindmap/commits.json` (a minimal map, the four directions, a pinned
  node, a nested control, a non-text node, slots); `core/tests/composite.rs` for composite containers; the layout
  itself is tested in `controls/` (`deno test -A`).

The shared module `controls/` (TypeScript, no dependencies, browser and Deno, alias `@controls/*` in the Desktop):
`tree.ts` (the `tree` layout: four directions, folded and pinned nodes), `measure.ts` (node text sizes),
`solve.ts` (a control from the outline, nested controls bottom-up), `edges.ts` (implicit edges, branch colours),
`outline.ts` / `opml.ts` (Markdown outline and OPML ⇄ a map), `svg.ts` (static drawing), `drawio.ts` (materialised
for draw.io), `catalog.ts` + `mindmap.ts` (the control catalogue with each control's self-description for Agents,
generated into `aiws/controls.json` by `deno run -A tools/build_catalog.ts`). Layout snapshots:
`fixtures/mindmap/layout/*.json` (`MINDMAP_REGEN=1 deno test -A` after an intended change; the formula is not
versioned, R18); outline cases shared with the service's parser: `fixtures/mindmap/outlines.json`.

### Data controls (控件机制 TODO G2–G6, G13, G14)

`todo` and `calendar` (`data_controls` in `controls/core.json`; `types.rs` `is_data_control`) show the records of the
tables they bind as **projected Blocks**: a template of `label` / `shape` / `asset` nodes the clients instantiate per
item, never stored, identity `<control>~<node>~<item>` (`controls/projection.ts`; an occurrence of a repeating event is
the item `<record>@<original start>`). The container: `layout` implied (`{ mode: "data" }`), a numeric size (never
`"auto"`), `bindings` checked as on a Cell (no `def_ref`, no `on`), no children at all (`child_allowed`); tables created
with it in the same Commit under the canvas content folder are owned (`owns`, `plan.rs` `add_owns_edges`). The outline
projects `bindings` (name → entity id) on data controls and composites. Layouts ship with the host
(`controls/datalayout.ts`: `list`, `columns`, `month`, `time_grid`, `agenda`); gestures, writes, pop-overs, per-person
views and the single-control page are the Desktop's (`ui/controls/data/`, `ui/controls/todo/`, `ui/controls/calendar/`).
Their tables are the standard business tables (“Standard business tables” above). The public contracts are
`aiws/contracts/todo.json` and `calendar.json` in `component.json`'s shape (`kind: "builtin"`, `version`, `config_schema`,
`user_state`, `bindings` with `schema_id`, `templates`, `layouts`, `gestures`, `actions`, `agent`), generated from
`controls/contracts.ts` by `deno run -A tools/build_catalog.ts` and returned by `control.describe { kind }` as
`control.contract`.

## Extension controls (扩展控件 P0 + P1, `notepads/done/aiworkspace-extension-p0-todo.md`, `notepads/done/aiworkspace-extension-p1-todo.md`)

Design: [扩展实现思路](<../../../doc/workspace/BuckyOS AI Workspace 扩展实现思路.md>) v0.9 (D6 edit → build, A1–A7,
Q15–Q25). A **control** of this section is a Block definition (`buckyos.block-def`) whose source lives in its
developer's home folder and is **built** into a Workspace: a render control (`kind` `declarative` or `html`, its
instances are Cells) or, since P1, a **composite** (`kind: composite`, its instances are containers laying out real
Blocks, see “Composite controls”). Block-controls (the previous section) are the code-registered hosts these run on.
The flow: write a control in home → `ext.build` it into a Workspace (instances update) → export one or more controls
(with what they build on) as a bundle or a `.pikg` → install it into the caller's **library** (this service, per user)
→ import it into another Workspace and instantiate it. Events and actions, per-property writes, source upgrades,
HTML `parent`, composite wish results and Workspace templates (P2 / P3) are the next section.

**Where things live.**

| Place | Content | Role |
| --- | --- | --- |
| home working copy | `/home/<owner>/AI Workspace/<workspace>/controls/<name>/` (a suggestion; the definition never stores a path, each person remembers theirs in `user_state` `aiws:src:<def_entity_id>`) | the editing entry: any editor, IDE or Agent |
| Workspace snapshot | under a folder (default `data/extensions`, created by the first build): the definition entity (`name` = its identity) and its unnamed source directory (`buckyos.dir`, one member per file) | what a build accepted: instances, exports, clones and `pull` read it |
| library | the caller's bucket `user/aiworkspace/<owner>/library` (key `<pkg_id>/<version>/<path>`) and `library.sqlite` in the service data folder | the controls this user may bring into any Workspace |

A source directory holds at most 64 files; sub-folders are allowed (Q23): a file's name is its relative path
(`icons/check.svg`) and becomes one member of the flat `buckyos.dir` (member names may hold `/`, `check_member_name`:
segments non-empty, neither `.` nor `..`, no leading, trailing or doubled `/`, ≤ 255 bytes). No segment starts with
`.`: clients keep their own state in dot files and folders (e.g. `.aiws-build.json`).

### Identity

`ident { author_did, name }`, written `<author_did>#<name>`: `name` is a renderer id (`[a-z0-9][a-z0-9._-]{0,63}`, it
is also the definition's `def_id`), `author_did` is `did:<method>:<id>` without `/`, `#`, whitespace or control
characters, the whole string at most 128 bytes. The build sets the definition entity's `name` to that string, so
the core's “sibling names are unique” rule deduplicates concurrent builds and imports of one identity into one folder
(one Workspace, one identity, one definition: Q1). The author is part of the identity, not the caller: whoever pulls
and builds Alice's control keeps `author_did` Alice's. `def_id` stays the short host-local id.

### `component.json` (`manifest_version: 4`, [schemas/component.schema.json](schemas/component.schema.json))

| Key | Shape |
| --- | --- |
| `manifest_version` | `1` … `4` (required). A build accepts every version up to its own and reads older ones as their newest form; newer versions only add optional keys (Q19). Version 2 adds `kind: composite` and non-empty `deps`, version 3 `events`, HTML `host_api: 3`, the declarative `button` and composite `events` / `grid`, version 4 (控件机制 TODO) `actions`, HTML `host_api: 4` and the composite `projections`, `media`, slot `visible`, events `from { projection }` and `on` (see “Manifest 4” below): an older file using one fails the build with a hint to raise it |
| `ident` | `{ author_did, name }` (required) |
| `kind` | `declarative` \| `html` \| `composite` (required). A composite has no `accepts` / `allow_no_source` (it is a container, not a Cell) and no `host_api` (it runs no code) |
| `title` / `description` | ≤ 256 / ≤ 4000 characters (`title` required) |
| `host_api` | positive integer: the host API the control was written for (`html.api_version` of an HTML control: 2, the default, 3 or 4; the declarative interpreter is version 1) |
| `events` | (version 3) `{ <name>: { title?, description?, params? } }`: the control's public events (next section, “Events and actions”); a composite declares its events in `composition.json` |
| `actions` | (version 4) `[{ id, title, icon?, modes?, needs?, run }]`, ≤ 16: what the host offers on an instance's near toolbar (see “Manifest 4” below) |
| `accepts`, `allow_no_source`, `default_size` | as on the definition: data types the main source may have, whether a Cell may have no source, `{ w, h }` |
| `config_schema` | instance properties, the JSON Schema subset `type` / `enum` / `default` / `required` / `title` / `description` / `minimum` / `maximum` / `minLength` / `maxLength` (+ `properties` / `items` to nest); other keywords fail the build (A7). The Desktop validates values with it (`z.fromJSONSchema`) and builds the instance form from it; the core never checks values (Q15) |
| `bindings` | named data inputs `{ <name>: { accepts?: [data types], required?: bool, title?, description? } }`, names `[a-z][a-z0-9_]{0,31}`, at most 32. An instance binds them in its Cell `bindings` (`{ <name>: { entity_id } }`); declarative items read them as `<name>.<expr>` (Q18), HTML through `aiws.input(name)` |
| `entry` | `html`: `{ html, css?, js? }`, file names in the directory, inlined into the definition's `html { html, css, js, api_version }`; `declarative`: one file name, a JSON document that becomes the definition's `declarative`; `composite`: the composition file (`composition.json`), checked and inlined as the definition's `composition` |
| `assets` | file names carried with the source (pictures, data) — part of the snapshot and the package, not inlined |
| `catalog` | `{ group?, icon?, description? }`: how the insert catalogue lists it |
| `agent` | file name of a Markdown self-description (≤ 64 KiB) for Agents, inlined as the definition's `agent` |
| `deps` | `["<author_did>#<name>", …]` (version 2; ≤ 32, no repeats, not itself): the controls this one builds on. The build resolves each to the definition of that identity in the same folder (a diagnostic with `code: DEP_MISSING` when there is none) and writes the definition's `deps` |

Every file the directory holds is part of the snapshot whether or not `component.json` names it; a named file that is missing fails the
build. The definition stays ≤ 512 KiB with everything inlined.

### The definition entity (`buckyos.block-def`)

Besides the keys it always had (`def_id`, `version`, `kind`, `title`, `description`, `accepts`, `allow_no_source`,
`default_size`, `declarative`, `html`, `config_schema`, `actions`, `inspector`), a definition may carry
([schemas/block-def.schema.json](schemas/block-def.schema.json)):

| Key | Shape |
| --- | --- |
| `ident` | `{ author_did, name }`, checked as above; `name` equals `def_id` |
| `manifest_version` | the `component.json` version it was built from (positive integer ≤ the core's) |
| `source_dir` | `{ entity_id }`: the live `buckyos.dir` holding the build's source snapshot; a strong `def_source` reference (the directory cannot be deleted while the definition uses it; exports and clones carry it). Not called `source`: that key is a Source (D12), which makes an entity read-only |
| `bindings`, `catalog` | as in `component.json` |
| `agent` | the self-description text, ≤ 64 KiB |
| `provenance` | `{ pkg_id?, version?, digest? }`: the package an import came from (absent for a local build) |
| `deps` | `[{ ident, entity_id }]` (≤ 32): the definitions it builds on, written by `ext.build` / `ext.import` only — any other Commit that changes it is `INVALID_OPERATION` (as the build record). Each entry is a strong `def_dep` reference: the target cannot be deleted while used (`REFERENCE_BROKEN`), exports and clones carry it, and a dependency that would close a cycle is `REFERENCE_CYCLE` (`data { entity_id, cycle }`) |
| `composition` | a composite's checked `composition.json`, inline (≤ 64 KiB). The core keeps its shape only (an object, ≤ 16 slots named `[a-z][a-z0-9_]{0,31}`, `events` shaped as below); what it means is the build's and the clients' (Q15) |
| `events` | (P2) `{ <name>: { title?, description?, params?, from? } }`, written by builds and imports only (as `deps`); ≤ 16 names, ≤ 16 KiB |

Definitions without these keys (wish results, templates, older Workspaces) read and work as before. A build writes
`version` = the previous `version` + 1 (the build revision; a Cell's `view.version` is the host's, not this) and the
dependency record `derived: { build: { input_digest, builder_version, built_at } }` in the same Commit
(`entity.set_derived`; a `build` record is accepted from build and import Commits only, Q14). The Commit's origin is
`build` and its message `build <ident>`. The outline projects `def_id`, `def_kind`, `ident` (string), `manifest_version`,
`provenance_pkg`, `deps` (the identities) and, for a composite, `composition` and `config_defaults` (each property's
`default` of `config_schema`): its instances are laid out from the outline alone.

### Composite controls (P1, [schemas/composition.schema.json](schemas/composition.schema.json))

A composite lays out Blocks the user (or an Agent) puts into its slots and draws a frame around them (R23: the
Chrome is drawn, the content is real Blocks — selectable, annotatable, connectable, addressable). Its
`composition.json` says what the table lists and nothing else; it never creates or deletes user Blocks:

| Key | Shape |
| --- | --- |
| `layout` | `{ algorithm: "stack" \| "fixed" \| "grid", direction?: "column" \| "row", spacing?, padding?, columns?, gap? }`. `stack` puts the slots' children one after another (`direction`, `spacing` px; default column); `fixed` keeps every child at its own `x` / `y`; `grid` (version 3) fills `columns` (1…12, required) cells per row, `gap` apart, in slot order (its slots' placement is `stack`). `padding` is a number or `{ t, r, b, l }`; numbers in [0, 2000]. The algorithms ship with the host (K2: `controls/`); none is loaded from a package |
| `slots` | `{ <name>: { accepts?, min?, max?, placement?, title? } }`, ≤ 16 names `[a-z][a-z0-9_]{0,31}`. `accepts` lists `cell:<author_did>#<name>` (an instance of that render control: in `deps` or already in the Workspace, never a composite), a view type (`richtext`, `shape`, …) or `container` (a group or a control); absent: any Block. `min` (0…64) is what inserting an instance creates, `max` (1…1000) what the clients allow. `placement`: `stack` (default), `top` / `bottom` (before / after the stacked slots) or `free` (children keep their `x` / `y`; the only placement of a `fixed` layout) |
| `chrome` | `{ background?, radius?, border?, title?: { text, slot? }, empty? }`: colours ≤ 64 characters, `radius` in [0, 200], `title.text` ≤ 256 (drawn while `slot`, when given, is empty), `empty` ≤ 1000 (what an empty instance shows) |
| `events` | (version 3) `{ <name>: { from: { slot, event }, title?, description? } }`: re-raises an event of a child in that slot (next section); version 4 adds `from: { projection, role? }` and a default handler `on` |
| `projections`, `media`, slot `visible` | (version 4) Blocks projected from data, a slot's media played from a config anchor, a slot shown to each viewer for a while: see “Manifest 4” below |

String values may hold `{{config.<key>}}` (a whole `spacing` / `padding` / `radius` may be one); the key must be a
property of `config_schema` (the instance's `config` value, its default when unset). The build checks all of it
(`store/src/ext.rs` `check_composition`; clients pre-check with buckyos-websdk `checkComposition`).

**Instances.** `buckyos.container { kind: "composite", def_ref: { entity_id }, config?, bindings?, layout?: { mode:
"def" }, title?, locked? }` (`types.rs` `container_primitive` / `composite_refs`): `def_ref` names a live definition of
`kind: composite` when the instance is created and never changes afterwards (`INVALID_OPERATION`: insert a new
instance); `bindings` are checked as on a Cell; `config` is an object ≤ 64 KiB whose values the clients check against
the definition's `config_schema` (the core never does, Q15); `layout` is implied (`{ mode: "def" }`). `def_ref` and
`bindings` are `def` / `bind` references, so a definition with instances cannot be deleted. The container is a Block
container like a group or a mind map: it sits on a Surface, in a group or in another control; its `w` / `h` may be
`"auto"` (its box wraps its children).

**Children.** Every child (a Cell, a group or a control) carries `slot` naming a slot of the definition, judged when
it is written (creation, `slot` changed, `tree.move` in) — a later rebuild of the definition does not re-judge existing
children. Several children may share a slot; `accepts` and `max` are the clients' to enforce, the core neither counts
nor matches them (Q15). A Cell inside a composite stays a leaf. Under a `fixed` definition no child may have an `"auto"`
`x` / `y`. The Desktop copies an instance with `tree.clone`.

**Copying a subtree (`tree.clone`, Q17; `plan.rs` `tree_clone`).** One operation copies a Block subtree — a Cell, a group
or a control with everything under it — so the service, the WASM replica and an offline client plan the same copy:
`{ op: "tree.clone", source_id, parent_id, order_key?, placement?, slot?, content?: "private", id_seed? }` (`structure`
on the target, `read` on the source). Every copy gets the id `c` + the first 26 hex digits of
`sha256("<id_seed>\n<old id>")` (`plan::clone_id`; `id_seed` defaults to `<seq>.<op index>` — pass one to know the ids
beforehand); parents, order keys, placements and `slot`s carry over, the root takes `order_key` (default: last under the
parent), `placement` and `slot` (`null` drops it) when given. `def_ref` is kept (instances share their definition). Data
a copied Block owns (`owns`) is cloned with `entity.clone` into the target Surface's content folder and the copy's
`source_ref` / `bindings` follow it (the copy owns the clone; rich text as a new lineage); data bound from elsewhere stays
shared. A connector inside the subtree is copied when each bound end is in the subtree (rebound to the copies) and left
out when one crosses its border. The normalized operation records `id_seed` and `new_ids { <old>: <new> }` (data clones
included); the Commit result lists them as `cloned: [{ op_index, new_ids }]`. Undo deletes the copies and their cloned
data. `ws.import` keeps ids and does not use it.

### Methods

| Method | Caller, rights | Params → result |
| --- | --- | --- |
| `ext.build` | `structure` on the folder (a new definition), `update` on the definition and its directory | `workspace_id`, `files: [{ name, obj_id } \| { name, base64 }]`, `folder_id?`, `expect?: { def_rev, dir_rev }`, `force?`, `provenance?`, `dry_run?`, `idempotency_key?` → `ok`, `status: accepted`, `commit_id`, `seq`, `def_entity_id`, `dir_entity_id`, `ident`, `version`, `def_rev`, `dir_rev`, `build`, `diagnostics` (warnings) — or `unchanged: true` (same input digest as the current build, nothing committed) |
| `ext.export` | `export` on each definition (`purpose: "pull"`: `read`) | `workspace_id`, `entity_ids[]` (definitions, or Cells / composite containers whose `def_ref` names one; deduplicated), `purpose?: package \| pull` → `manifest` (the AIWS manifest), `controls: [{ entity_id, ident, kind, version, def_rev, dir_rev, digest, dir, deps, built_at, files: [{ name, obj_id?, size, text? }] }]`, `files: [{ path, obj_id?, size, text? }]`. A package follows `def_dep`: what the chosen controls build on is added once (it needs `export` too); a pull names the chosen ones only |
| `ext.install` | the caller's own library (`mode: dev`); the control panel installing for `owner` (`mode: installer`; a service caller is known by its session's `appid`) | `pikg: { bucket_id, key }` (a `.pikg` the caller bound into its own `user/aiworkspace/<user>/exports`, consumed once installed) or `pikg: { obj_id }` (a `.pikg` FileObject the caller may read, judged at the NDM gateway; the Installer passes its staged object) or `from: { workspace_id, entity_ids[] }` + `package: { name, version, title? }` (export straight into the library), `mode`, `owner?` → `pkg_id`, `version`, `action: installed \| satisfied \| replaced`, `controls` |
| `ext.list_library` | any account: its own library | → `packages: [{ pkg_id, version, title, author, mode, installed_at, controls: [{ ident, name, kind, version, digest, title, description, catalog, dir, deps }] }]` |
| `ext.uninstall` | its own library | `pkg_id` → `removed` (Workspaces keep what they imported, D4) |
| `ext.library_bundle` | its own library | `pkg_id`, `idents?` → `manifest`, `controls: [{ ident, …, files: [{ name, size, obj_id, base64 }] }]` (the bytes, so a client can build them into a Workspace of another Zone) |
| `ext.import` | as `ext.build` | `workspace_id`, `folder_id?`, `items: [{ pkg_id, ident }]` → one Commit for all items and what they build on (`status`, `commit_id`, `controls: [{ ident, def_entity_id, action: created \| satisfied }]`, dependencies first) |
| `ext.pack` | an account (`export` on each definition, through `ext.export`); the package goes into its own `exports` bucket | `workspace_id`, `entity_ids[]` (definitions or instances; what they build on comes along), `package: { name, version?, title? }` (`name` `[a-z0-9-]`, ≤ 63; `version` default `0.1.0`) → `bucket_id`, `key` (`<name>-<version>.pikg`), `obj_id`, `size`, `digest` (`sha256:` of the file), `app_doc_object_id`, `app_did`, `pkg_id`, `version`, `payload_digest`, `tar_digest`, `download` (`{ bucket_id, key, version, obj_id, token }`, as `doc.export`) |
| `ext.upgrade`, `ext.export_template`, `ext.create_from_template` | see the next section | `ext.pack` and `ext.install { from }` also take `templates: [{ surfaces, data?, template }]` (`entity_ids` may then be empty), `ext.list_library` lists each package's `manifest_version` and `templates`, `ext.build` takes `upgrade` and `migration` |

- **Build.** The service never reads home paths: the client lists the directory over NFSP and passes each file either as
  an `obj_id` — only a named-store FileObject (`cyfile:…`, e.g. a file the file service holds as an object), which the
  service authorizes at the NDM gateway with the caller's token (as `asset.import_object`), stages and reads — or as
  `base64` bytes the client read over NFSP with its own session (≤ 4 MiB inline per build). Files of a native home
  folder take the second way: the `obj_id` NFSP reports for them, when it reports one, is a `sha256:` content hash, not
  an object the service can fetch. When the inline bytes would pass 4 MiB the client uploads the largest files first
  as FileObjects it may read and names them by `obj_id` (Q22; buckyos-websdk `buildFiles` / `packBuildFiles` with an
  uploader — the Desktop's asset upload, the CLI's PUT into the caller's own `exports` bucket); the parameters do not
  change. Then: parse and check `component.json` and the entry files (a composite's `composition.json` included),
  resolve `deps` and the slots' `cell:<ident>` against the folder (and the controls built in the same Commit, deps
  first), compose the definition, check compatibility with the instances and the dependents (below), and commit once
  — folder (when missing), directory
  (`entity.create` or `dir.replace`), definition (`entity.create` or `set_keys` / `unset_keys`), `entity.set_derived` —
  with read-set preconditions on the definition's and the directory's `content_rev` (`expect`). Nothing is committed
  on any failure. `dry_run` stops before staging and answers the diagnostics.
- **Base revisions (git-like).** A build of an identity that already exists in the folder needs `expect` (the revisions
  the client last built or pulled: `def_rev`, `dir_rev`) or `force: true`; without either it is `REVISION_CONFLICT` /
  `BUILD_BASE_REQUIRED`, with stale ones `REVISION_CONFLICT` / `BUILD_BASE_STALE` (`data: { def_entity_id, def_rev,
  dir_rev }`): pull, merge, build again.
- **Diagnostics.** A failed build is `BUILD_FAILED` with `data.diagnostics: [{ file, line?, col?, severity: error |
  warning, message, code?, instances?, dependents? }]` (JSON syntax errors carry line and column). `code: DEP_MISSING`:
  a `deps` entry, or a slot's `cell:<ident>` that is not in `deps`, names no definition of the folder. A slot's
  `cell:<ident>` that names a composite fails too (a composite is a container, never a Cell).
- **Compatibility with instances and dependents.** Rebuilding an identity fails when it narrows the contract what uses
  it relies on: `kind` changes, `accepts` loses a type, a binding disappears or becomes required,
  `config_schema.required` gains a name, and for a composite a slot disappears or accepts less. The diagnostics list
  the instances (Cells and composite containers, `instances`) and the definitions that build on it (`def_dep`: one
  diagnostic per dependent, `file` = its identity, `dependents: [{ ident, def_entity_id }]`). Rebuilding a definition
  into a composite also fails while a dependent's slot accepts it as `cell:<ident>`. Layout and Chrome changes of a
  composite are free: its instances are laid out anew and their Blocks stay as they are. Migration is the user's (P2).
- **Input digest.** `sha256:` of the canonical JSON `{ files: [[name, obj_id], …] (sorted by name; inline files by the
  `obj_id` of their bytes), manifest_version, builder_version }`; `builder_version` is `aiws-ext-build/1`.
- **Pull** is client-side: `ext.export { purpose: pull }` names the files (a definition that was never built from
  source — a wish result, a template — answers synthesized `component.json` and entry files as `text`), the client
  fetches `obj_id` files from `GET /kapi/aiworkspace/asset/<ws>/<obj_id>` and writes them into home over NFSP, and keeps
  `{ workspace_id, def_entity_id, def_rev, dir_rev }` in `.aiws-build.json` for its next build.
- **Import.** `ext.import` builds each item from the library files (no gateway round trip: they are in the service's
  store) with `provenance { pkg_id, version, digest }`, together with what the items build on (from the same package,
  each once, dependencies first). An identity already in the folder with the same provenance is `satisfied` (no
  change); with other content it is `INVALID_OPERATION` / `EXT_IDENT_EXISTS` (`data.library_version`: change versions with
  `ext.upgrade`, next section; local changes are never overwritten) — except for a dependency brought along: whatever definition
  of that identity the folder holds satisfies it, and the dependent builds on that one. Across Zones the client takes
  `ext.library_bundle` from its own Zone (which carries the dependencies as well) and calls the target's `ext.build`
  with `base64` files and `provenance`, dependencies first.
- **Pack** (`ext.pack`, Q20; `store/src/pack.rs`): the package export of the chosen controls written as an `aiws`
  PIKG by the service, with the CLI's rules (`cli/modules/pikg_archive.ts`, `pikg.ts`): a ustar archive of the bundle
  (in each folder its entries sorted by name, a folder as `<path>/` before its content, files 0644, folders 0755,
  owner 0, mtime 0, no links), gzip, and a stored ZIP of `APPDOC.json`, `PACKAGE_META.json` and `aiws.tar.gz`. The
  AppDoc is `did:bns:<name>.<owner label>` (the owner is the caller's `did:bns` DID), `app_type: aiws`, one
  sub-package `aiws` = package `all.aiws.<AppId>`, `create_time` the newest build time of the bundle's controls (the
  same bundle packs the same bytes); it is not signed (the service holds no author key: signing stays with the author's PIKG tooling). The tar archive
  is the one the CLI writes for the same bundle (`tar_digest`); the gzip bytes are not (flate2 level 6 here, Node zlib
  `Z_RLE` there), so the payload digest, the package and AppDoc object ids derived from it and the `.pikg` file differ
  from a CLI pack of the same controls, while the bundle, its manifest digests and every check an installer runs are
  the same. The file is
  bound under `<name>-<version>.pikg` in the caller's `exports` bucket: download it with `download.token`, or install
  it with `ext.install { pikg: { bucket_id, key } }`.

### AIWS manifest and the `aiws` package

A bundle (and the `aiws` sub-package of a `.pikg`) is `aiws-manifest.json` plus `controls/<dir>/<file>`
([schemas/aiws-manifest.schema.json](schemas/aiws-manifest.schema.json)):

```json
{ "manifest_version": 2, "purpose": "aiws", "host_api": { "html": 2, "declarative": 1 },
  "controls": [{ "ident": "did:bns:alice#kpi-card", "name": "kpi-card", "kind": "declarative", "version": 3,
                 "digest": "sha256:…", "dir": "controls/kpi-card/", "title": "KPI card" },
               { "ident": "did:bns:alice#kpi-board", "name": "kpi-board", "kind": "composite", "version": 1,
                 "digest": "sha256:…", "dir": "controls/kpi-board/", "deps": ["did:bns:alice#kpi-card"] }],
  "workspace_templates": [] }
```

`digest` is `sha256:` of the canonical JSON `[[file name, sha256 hex of its bytes], …]` sorted by name (a name in a
sub-folder is its relative path): an installer recomputes it from the files. `deps` must equal the control's
`component.json` `deps`, and every one must be another control of the same manifest: an open closure is
`LIBRARY_PACKAGE_INVALID` (`library.rs` `verify_bundle`; the CLI's `pikg build` / `pack` / `info` check the same).
`workspace_templates` (manifest_version 3) lists Workspace templates carried under `templates/<id>/` (next section,
“Workspace templates”). The PIKG side: AppDoc `app_type: "aiws"`, exactly one sub-package `pkg_list.aiws` (payload
`aiws.tar.gz` = the bundle), no docker / web / native packages; the package id is the AppId of the AppDoc `did`, its
version the AppDoc `version`. Installing one (`buckyos app install x.pikg` → control panel → `ext.install` with
`mode: installer`; or `ext.install` with `mode: dev` from a client) creates no AppSpec and starts nothing; the
library records it. The same package id at the same version with other content is refused
(`LIBRARY_PACKAGE_INVALID`); a new version replaces the old one in the library (Workspaces keep their copies).

CLI (`buckyos aiws`, buckyos-websdk `cli/modules/aiws.ts`; `aiws import` / `aiws export` stay the Block-control outline
verbs): `build <dir> --workspace <ws> [--folder <id>] [--watch] [--dry-run] [--force] [--zone-path]` (`<dir>` is a local
folder when one exists there — its files go inline — or a Zone path, e.g. `/home/alice/AI Workspace/Demo/controls/kpi-card`;
`.aiws-build.json` there keeps the base; `--watch` follows a Zone folder; sub-folders are part of the source; past 4 MiB
inline the largest files go to your `exports` bucket and are sent by reference), `pull <def> --workspace <ws> --out <dir>
[--local]`, `ext-export <def,…> --workspace <ws> --out <local dir> [--name <pkg>] [--version <v>] [--owner <did>]
[--pikg] [--install]` (writes `<out>/aiws/`, then `pikg init/build/pack` in `<out>`; `--install` binds the `.pikg`
into your `user/aiworkspace/<you>/exports` bucket and calls `ext.install`), `library list | install <file.pikg> | remove
<pkg_id>` (`list` shows each control's `deps`), `ext-import <pkg_id>#<ident>,… --workspace <ws> [--folder <id>]` (what
the items build on comes along); `pikg init --kind aiws-controls [--composite]` writes a package skeleton with one
declarative (or composite) control, and `pikg build` / `pack` / `info` check an `aiws` bundle as the library does. The
drivers are buckyos-websdk `src/aiws_ext.ts` (shared with the Desktop: `checkComponent`, `checkComposition`,
`collectSources`, `buildFiles` / `packBuildFiles`, `unbuiltChanges`, `controlSkeleton({ kind: 'composite' })`), the
client call `AiWorkspaceClient.extPack`.

### HTML host API additions (P1)

Host API 2 (`window.aiws`, `html.api_version` 2) gains one member without a version change: `aiws.asset(name) →
Promise<string>`, a blob URL of a file the definition lists in `assets` (read by the host from the source snapshot
with the session, revoked when the Block unmounts; any other name is refused). A control checks
`typeof aiws.asset === 'function'` and keeps working on hosts without it. Events and `parent` are host API 3 (next section).

## Extension controls P2 / P3 (`notepads/done/aiworkspace-extension-p2p3-todo.md`, Q26–Q33)

One version step for both phases: `manifest_version` **3** (`events`, the declarative `button`, composite `events`, a
non-empty `workspace_templates`), HTML host API **3** (`aiws.emit`, `aiws.createBlock { parent, slot }`,
`aiws.context.interactive`; hosts run 2 and 3, the members of 2 are unchanged), the definition key `events` and the
instance key `on`. `PROTOCOL_VERSION` does not change. Files of versions 1 and 2 read as before (Q19).

### Per-property writes (`entity.set_props`, Q28) and the definition revision check (Q29)

`{ op: "entity.set_props", entity_id, key: "config" | "bindings" | "on", props: { <name>: <value> | null }, expect?: { <name>: <rev> } }`
(class overwrite, compensable, capability `update`; `plan.rs` `entity_set_props`):

- Every property of one of these object keys is its own version cell `key_revs["p:<key>.<name>"]`, named by the
  selector `{ kind: "entity_prop", key, name }` (a property never written reads 0). `null` removes the property; an
  object left empty removes the key.
- `expect` is per property. Stale ones answer `REVISION_CONFLICT` with one conflict each (`selector` `entity_prop`,
  `expected_rev`, `current_rev`, `current_value`); a property without `expect` merges (the later write wins). Two people
  changing two properties of one Block both keep their change; the same property conflicts.
- The key's own cell (`doc_key`) and the entity's `content_rev` move with every write. A whole-key `entity.set_keys` /
  `entity.unset_keys` of `config`, `bindings` or `on` moves the `p:` cells of every name in the old and the new value
  (as `dir.replace` moves `m:`): builds, migrations and imports replace the key, and a per-property `expect` from
  before conflicts.
- The inverse is an `entity.set_props` of the old values (`null` where there was none) with `expect` = this Commit's
  `seq` for each: undo restores only what this write changed and conflicts when someone changed one of them since.
- Primitive class (every path, replay included): `key` is one of the three, ≤ 64 names per operation matching
  `^[A-Za-z_][A-Za-z0-9_-]{0,63}$` (Q28 names `[a-z][a-z0-9_]{0,63}`; existing `config` keys use capitals and dashes),
  each value ≤ 64 KiB. Whether the key belongs on that entity, and its content, is judged by the entity's validators as
  for `set_keys` of the whole key.
- **Definition revision (Q29).** No new mechanism: a client writing an instance (a Cell or a composite naming a
  `def_ref`) adds the read-set precondition `{ target: { entity_id: <definition> }, expect: { rev: <its content_rev in
  the client's outline> } }`. When the definition was rebuilt meanwhile the Commit is `REVISION_CONFLICT` with a conflict
  whose `entity_id` is the definition (`detail: "read-set precondition failed"`); the Desktop says “the control was
  updated, the form now follows the new definition, try again”. Offline replicas replay through the same planner and
  meet the same check.

### Events and actions (P2, Q26)

**Definition side.** `component.json` (`manifest_version: 3`) may declare `events: { <name>: { title?, description?,
params?: { <p>: { type: "string" | "number" | "boolean" | "object" | "entity", required?, description? } } } }`
(names `[a-z][a-z0-9_]{0,31}`, ≤ 16 events, ≤ 16 params each; `entity` = an entity id). Raising one:

- an HTML control (`host_api: 3`): `aiws.emit(name, params) → Promise<{ handled, event_id }>`;
- a declarative control: the item `{ kind: "button", label, emit, params? }` (`emit` names a declared event; `params`
  values are JSON, strings may hold `{{config.<key>}}`);
- a composite declares none in `component.json`: `composition.json` `events: { <name>: { from: { slot, event },
  title?, description? } }` re-raises an event of a child in that slot. The slot must list at least one
  `cell:<ident>` whose definition declares `event`; otherwise the build fails with a diagnostic `code: EVENT_UNDECLARED`.

The build writes the definition key `events` for every kind (`{ <name>: { title?, description?, params?, from? } }`;
a bubbled event carries the params of the child event it re-raises). Like `deps` only build, import, internal and
replay Commits change it; the core keeps its shape (object, ≤ 16 names, each an object, ≤ 16 KiB). The outline projects
`def_events` (the names) on a definition and `on_events` (the names an instance binds) on a Cell or composite.

**Instance side.** A Cell (not a connector or ink) or a composite container may carry `on: { <event>: Handler |
[Handler, …] }`, written with `entity.set_props { key: "on" }` or with the Block:

| Handler | Effect |
| --- | --- |
| `{ kind: "write", target, what: "recordSet" \| "upsert" \| "setMarkdown", args }` | the typed write of the host API under the instance's binding `target` (`source` = its `source_ref`): `recordSet` `args { props }` = `aiws.record(target).set(props)`, `upsert` `args { rows, key? }` = `aiws.table(target).upsert(rows, { key })`, `setMarkdown` `args { markdown }` = `aiws.text(target).setMarkdown(md)` |
| `{ kind: "wish", wish_id, inputs? }` | runs the wish's next step (as the run button does) as the clicker; the run's `request.event` is `{ name, instance_id, params, inputs }` |
| `{ kind: "open", target? }` | opens a binding (default `source`) or an entity id |
| `{ kind: "notify", text }` | a local notice |

Strings anywhere in a handler may hold `{{event.<param>}}` and `{{config.<key>}}` (the instance's value, else the
default of `config_schema`); a string that is exactly one placeholder takes the value's type. The core checks only the
shape (primitive class: an object, ≤ 16 events named as above, each one handler or ≤ 4, each an object whose `kind` is
one of the four, ≤ 16 KiB in all; `tree.clone` copies it). That the event is declared and the binding exists is the
business of the Desktop form and of `ext.build`, which lists bound instances when an event they bind is removed or loses
a param (`narrowing`).

**Execution (§6.7).** A click is an action of the person who clicks, with their rights; the host executes an event's
handlers in order, all `write` handlers as **one** Commit (`idempotency_key: evt/<instance>/<event>/<event_id>`, label
“<control>: <event>”, undoable), a `wish` with the same key, `open` / `notify` locally. A composite re-raises a child's
event: after the child instance's own `on`, the host walks up through `slot` to the composite whose definition's
`events.<name>.from` matches (slot, event) and runs that instance's `on.<name>` with the same `event_id`. Other clients
only see the result; mounting a Block, receiving a sync event or reloading never raises anything again (rebuilds remount
with the new `html`, a late `emit` of the old runtime is dropped by its nonce). Before the first pointer or key gesture
inside a mounted HTML Block, `emit` is refused (the extension gets `{ handled: false }` and a notice).

### Design and run mode (Q27)

The canvas has a local, unpersisted `interactive` flag (not in `user_state`), switched with ▶ / ✎ on the toolbar.
Defaults: `edit` → design, `view` → run, a non-public `show` → run only for Blocks marked `presentation.live`,
replay → off and locked; guests and phones run like anyone in view mode (owner 2026-11-12). A canvas mode not chosen
yet is `view` for a guest without `update`, else `edit`. In view mode an HTML Block runs
by itself while the canvas runs (within the `html` budget, over it its snapshot) and keeps running whatever is clicked
or selected; in edit mode's run mode clicking an HTML Block activates it (also within the budget). A
declarative `button` is enabled and handlers run; a composite's frame does not catch its children's clicks. In design
mode a click selects and moves, an `emit` runs nothing and the Block shows a corner note “event X (design mode, not
run)”; “Run once” stays. Writes from handlers and from `aiws.record().set` etc. need `policy.writes`, **or** view mode
in run mode with `update` on the target (view's `writes: false` no longer refuses every write). The flow page no
longer mounts HTML Blocks automatically while editing: the same rules as the canvas.

### HTML host API 3

`html.api_version` 3 (component.json `host_api: 3`, manifest_version 3). Additions to API 2, which keeps every member:

| Member | |
| --- | --- |
| `aiws.version` | the version the definition declares (2 or 3) |
| `aiws.context.interactive` | whether the canvas is in run mode |
| `aiws.emit(name, params?) → Promise<{ handled: boolean, event_id }>` | raise a declared event (an undeclared name rejects); in design mode `handled: false` and a corner note |
| `aiws.createBlock({ renderer, parent?, slot?, placement?, data?, config?, title? })` | `parent`: `"self"` (inside this Block — only within a tree control, `in_tree_control`), an entity id, or `{ binding }` (the entity a binding names); a composite parent needs `slot` (one of its definition's slots, `child_allowed_here`), a tree control's node gets `placement` `"auto"`; without `parent` a sibling as in API 2 |

A host older than the definition (`api_version` above what it runs) shows the static snapshot with “needs host API
n” instead of failing (Q33).

### Manifest 4 and HTML host API 4 (控件机制 TODO G13, G16–G22, H1–H6)

`manifest_version` 4 (`types.rs` `MANIFEST_VERSION`; files of versions 1–3 read as before). Examples of everything
below: [doc/workspace/control-examples](../../../doc/workspace/control-examples/README.md) (TV screen, time-limited,
photo vote, listen together; built by `server/tests/ext.rs` `control_examples_and_manifest_4`).

**Actions** (`component.json` `actions`, `store/src/ext.rs` `check_actions`; the build writes them into the definition's
`actions`, the outline projects a composite definition's as `def_actions`): ≤ 16 of `{ id, title, icon?, modes?: ["edit"
| "view"], needs?: [capabilities], run: Handler | [Handler, ≤ 4] }`. The Desktop offers each on an instance's near
toolbar in the modes it names (default both) to whoever holds `needs`, also in view mode and before an HTML control runs
(`ui/canvas/CanvasView.tsx`, `ui/extensions/actions.ts`). Handlers: `write` / `wish` / `open` / `notify` as in `on`;
`config { props }` — the instance's own properties, an `entity.set_props` with per-property `expect`; `forward` — handed
to the running HTML as the request `action:<id>` (a control that is not running is started first). Strings may hold
`{{now}}` (the service clock) and `{{config.<key>}}`. The `write` and `config` operations of one action are one Commit,
as the person who clicked.

**Composite (`composition.json`, `check_composition`, `check_projections`, `check_event_keys`)**:

| Key | Shape |
| --- | --- |
| `projections` | `{ <name>: { source, mark?, template, layout, placement?, max_items? } }`, ≤ 8. `source.kind`: `records` (a bound table), `members` (a bound directory: `name`, `object_id`, `media_type`, `size`), `counts` (the elements of a list field of a bound table counted, most first: `key`, `count`, `rank`; a select's elements by option label; `of` joins another projection's items by key, ties in its order; `top` 1…100) or `viewer` (`principal`, `first_open`, `remaining_ms`). `mark { binding, field, as }`: `item.<as>` is true when the viewer's own (submitter's) record lists the item. `template.nodes` 1…32 of `{ id, view: label \| shape \| asset, box?, config?, when?, role? }`, placeholders `{{item.…}}`, `{{viewer.…}}`, `{{config.<key>}}`. `layout { algorithm: list \| grid, columns? (grid), gap?, item: { w?, h } }`; `placement` `top` / `stack` / `bottom` places the region like a slot; `max_items` 1…1000. Never stored; identities `<control>.<projection>~<node>~<item>` |
| `media` | `{ slot, anchor, loop? }`: the media Block in `slot` plays where `config.<anchor>` = `{ pos_ms, at_ms, playing }` says (at server time `at_ms` it was at `pos_ms`), for every viewer by the service clock, by Range from `asset.stream_url`, without activation (`ui/media/BroadcastMedia.tsx`); a viewer's own pause / mute / “back to live” stay local; the end holds the last frame unless `config.<loop>` is true. An empty slot offers whoever has `update` and `structure` “choose a video or sound” through the system file picker |
| slot `visible` | `{ from: "first_open", for_ms }` (`for_ms` ≤ a year or `{{config.<key>}}`): each viewer sees the slot's Blocks for `for_ms` from their first opening, recorded once in their user state `block:<id>/first_open` by the service clock (`ui/controls/composite/visibility.ts`). A presentation, not access control: edit mode shows everything with the frame marked “time-limited”, static views and exports do not hide |
| event `from { projection, role? }` | raised in run mode by a press on a projected node of that role (default `body`), params `{ item, projection }` |
| event `on` | the default handler(s), run when the instance binds none for that event (`ui/extensions/events.ts` `runHandlers`) |

**HTML host API 4** (`html.api_version` 4, `host_api: 4`, manifest 4; hosts run 2, 3 and 4, `aiws.version` is the
declared one; `ui/extensions/htmlRuntime.ts`, `bridge.ts`):

| Member | |
| --- | --- |
| `aiws.onConfig(cb) → unsubscribe` | the instance's `config` as it changes (also refreshes `aiws.context.config`) |
| `aiws.serverNow()` | the service clock (`ws.time`, calibrated by half the shortest of three round trips) |
| `aiws.userState.get(key)` / `set(key, value)` | this person's state of this Block, key `block:<entity_id>/<key>` (`[A-Za-z0-9_.:-]{1,64}`) |
| `aiws.mediaUrl(binding, member?)` | a Range-playable address of a bound audio / video asset or of a member of a bound directory (`asset.stream_url`; a blob URL when no stream address is available) |
| `aiws.on('action:<id>', handler)` | a declared action forwarded by the host (`forward`) |

### Source upgrades and migrations (`ext.upgrade`, Q30)

| Method | Caller, rights | Params → result |
| --- | --- | --- |
| `ext.upgrade` | as `ext.build` | `workspace_id`, `items: [{ def_entity_id, pkg_id? }]`, `migration?: { <ident>: Migration }`, `force?`, `dry_run?`, `idempotency_key?` → `status: accepted \| unchanged \| checked`, `commit_id?`, `controls: [{ ident, def_entity_id, action: upgraded \| unchanged, from, to, migrated: [instance ids] }]`, `plan?: [{ instance, ops }]` (dry run) |

- **Where the new version comes from.** The caller's library (same Zone): `pkg_id` or else the definition's
  `provenance.pkg_id`. The library holds one version per package. Across Zones the client calls the target's
  `ext.build { files, provenance, upgrade: true, migration? }` with the library bundle of its own Zone.
- **Unchanged.** The library version equals `provenance.version` and the digest equals `provenance.digest` → nothing to
  do (`action: unchanged`).
- **Local changes.** When the snapshot's content digest differs from `provenance.digest` (it was built locally after the
  import), the upgrade is `INVALID_OPERATION` / `EXT_LOCAL_CHANGES` (`data: { ident, def_entity_id, imported_digest,
  snapshot_digest, files: { added, removed, changed } }` — the snapshot compared with the new version) unless `force`.
  Local changes are never overwritten silently.
- **The rebuild.** Otherwise the upgrade is a build of the library files (`BuildOpts.upgrade`: the identity may exist
  with another provenance, no base `expect`) with the new `provenance`, origin `build`, message
  `upgrade <ident> <old version>→<new version>`. Definitions of the same package the item builds on are upgraded with
  it, dependencies first. Instances keep their overrides and content.
- **Migration** (`schemas/migration.schema.json`), for a version that narrows the contract instances rely on:
  `{ config?: { rename?: { <old>: <new> }, drop?: [names], fill?: { <name>: value } }, bindings?: { rename?, drop?,
  fill?: { <name>: { entity_id } } }, slots?: { rename?: { <old>: <new> }, move?: { <old slot>: <slot> } },
  events?: { rename?, drop? } }`. The service writes, into the same Commit, for every affected instance an
  `entity.set_props` per key (`config`, `bindings`, `on`) and a `tree.move { slot }` per child of a renamed or moved
  slot; then judges compatibility per instance with the new definition. An instance that still does not fit fails the
  whole upgrade (`BUILD_FAILED`, `instances`); a dependent definition (`def_dep`) that does not fit fails it too
  (migrating dependents is not in this phase). `ext.build` takes `migration` as well (a local build that narrows its
  own contract). `dry_run` answers `plan`.
- **Compatibility per instance.** With P2 a rebuild fails only for the instances the narrowing touches: a removed
  binding they bind, a binding becoming (or arriving) required that they leave unbound, a required `config` name they
  neither set nor get a default for, an `accepts` type their source has, a removed or narrowed slot their children use,
  a removed event (or event param) their `on` binds; `kind` changes always fail while instances exist.
- `ext.import` refusing `EXT_IDENT_EXISTS` answers `data.library_version` (the version in the caller's library), so
  a client offers “upgrade” right away.

### HTML parent, `grid` layout, fallbacks (P3, Q31, Q33)

- `composition.json` `layout.algorithm` gains `grid` `{ columns: 1…12, gap?: [0, 2000] | {{config.<key>}} }` (K2: the
  host ships it, `controls/composite.ts`); children fill cells row by row in slot order. A host that does not know an
  algorithm lays the children out as a group (free) and marks the frame “layout <name> needs a newer host”.
- The library refuses a package whose `manifest_version` is above what the service reads; `ext.list_library` reports
  each package's `manifest_version`, so a client older than a package marks it “needs a newer version” and does not
  import it.
- On phones the children of a composite take touches and an HTML Block can be activated as on a computer (控件机制 TODO G22, H3: only guests and replay never run them).
- Replay and public / guest views keep `interactive` false: HTML Blocks show their snapshot, events run nothing,
  composites lay out as usual.

### Agent composite results (P3)

A wish `control` result may name a composite definition of the Workspace instead of a built-in kind:
`aiws.result.control(name, { def: "<author_did>#<name>", config?, slots: { <slot>: [ "result:<n>" | "input:<n>" |
{ view: "richtext", markdown } ] }, under? })` — the result is a composite instance of that definition (missing:
“ext-import it first”), each `result:<n>` result's Cell goes into the slot, each `input:<n>` Block is cloned into it
(`tree.clone`), an inline item becomes a new rich text there. `under` may name a composite and one of its slots
(`{ input, slot }`). Agents read and write with their own grants.

### Workspace templates (P3, Q32)

A template is a **snapshot**: one `materialize` of chosen content, carried in the control package next to the controls
it needs, and loaded as a new Workspace — never merged into an existing one (out of this phase).

- **Bundle layout.** `templates/<id>/snapshot.json` (`{ format: "aiws-template/1", content_root, entities }`) plus the
  content closure as `templates/<id>/objects/<object id>` (NamedObjects, canonical JSON) and
  `templates/<id>/chunks/<chunk id>` (the bytes of files and assets; ids in their file-name form, as in a `.bcanvas`
  package). The manifest's `workspace_templates` lists
  `{ id, title, description?, snapshot: "templates/<id>/snapshot.json", digest, requires: { host_api, controls:
  [idents] }, params?: { <k>: { type: "string" | "number" | "boolean", title?, default? } }, reset?: { tables: [entity ids] } }`
  ([schemas/template.schema.json](schemas/template.schema.json)); `digest` is the content digest of every file under
  `templates/<id>/` (names relative to that folder: `snapshot.json`, `objects/<id>`, `chunks/<id>`; as a control's), `requires.controls` are controls of the same manifest.
- **`ext.export_template`** (`export` on the chosen Surfaces): `workspace_id`, `surfaces: [ids]`, `data?: [ids]`,
  `template: { id, title, description?, params?, reset? }` → `template` (the manifest item), `controls` (the
  definitions it needs: `def_ref`s of the content, with their `deps`), `files: [{ path, base64 }]`. The snapshot holds
  the system nodes, the chosen Surfaces with their subtrees and content folders, what their Blocks reference
  (`source_ref`, `bindings`, `def_ref`, data shown by `on` handlers), the `data/extensions` closure of those
  definitions and the chosen data entities with their parents; never runs, shows, the session folder or personal
  entities. Strings may hold `{{params.<k>}}`.
- **Packing and the library.** `ext.pack { …, templates: [ { surfaces, data?, template } ] }` and `ext.install { from:
  { …, templates } }` export the templates with the controls; installing records them in `library_templates` (no
  Workspace is created). `verify_bundle` checks each item, its digest, that every object of the closure is present and
  hashes to its name, and that `requires.controls` are in the package. `ext.list_library` lists `templates: [{ id,
  title, description, params, requires }]` per package.
- **`ext.create_from_template`** (an account that may own Workspaces): `pkg_id`, `template_id`, `title?`, `params?` →
  `workspace_id`. The snapshot loads through `load_workspace` (new id, the caller as owner, rich texts as new lineages:
  the path of `ws.import new`), then one `internal` Commit replaces `{{params.<k>}}` in titles, Cell / container
  `config` and record / table values and clears the rows of `reset.tables`. No id is remapped (the snapshot is a whole
  new Workspace); two creations are independent.

## Ink (手写笔, `notepads/aiworkspace-handwriting-todo.md` D0)

A pen stroke is drawn as one or more Cells with `view { type: "ink", version: 1 }` on a free Surface — one per
segment, consecutive segments sharing their boundary point, all created in one commit. The Desktop writes them
(`desktop/src/app/aiworkspace/ink/model.ts`, which is also the reference encoder); `core/src/types.rs`
`validate_ink` judges version 1 on create, import and any change of `view` / `config` (other versions are left to
their renderer):

| `config` key | Shape |
| --- | --- |
| `format` | `1` |
| `stroke` | the logical stroke's id (an entity id): its segments are selected, erased and copied as one; a copy gets a new one |
| `seg` / `segs` | segment index below the segment count, `segs` 1…64 |
| `color` / `width` / `pressure` | `#rrggbb`; base width in canvas units, (0, 256]; whether pressure shapes the width |
| `frame` | `{ w, h }` > 0: the segment's own coordinate frame, which the placement box shows (a resize scales it) |
| `points` | channels of equal length, 1…16 384 points: `x` in [0, w], `y` in [0, h], `t` whole ms since the pen came down (non-decreasing); optional `p` in [0, 1], `tilt_x` / `tilt_y` in [-90, 90] |

Unknown keys or channels are refused (`INVALID_SCHEMA`), so are `source_ref`, `bindings` and `def_ref`; the config
limit of every Cell applies (≤ 64 KiB canonical JSON, `LIMIT_EXCEEDED`; the Desktop cuts segments at 60 KiB). The
outline projects `ink_stroke` (the `stroke` id) on an ink Cell's envelope. The fixture
`fixtures/ink/strokes.json` (a mouse line, a pen tap, a pressure stroke with tilt, a three-segment stroke) is
written by `desktop/tests/datamodel/aiworkspace-ink.test.ts` and committed and read back by `core/tests/ink.rs`.

## Wish runs (许愿格; AI Native 第一期: analyze → code → run)

Two `proc` programs (design §13; `notepads/aiws-ai-native-phase1-todo.md` §2). Runs live in `local.sqlite` `runs` (never
exported); stage folders in `<ws>/runs/<run_id>/work`, the AI answer cache in `<ws>/cache/llm_map/`. The service runs no
model and no program: every stage of `wish.xllm@1` executes on an executor — the starter's browser (`local`) or the Owner's
Agent (`hosted`). The service takes the stage snapshot, answers the stage's tools, judges what the executor hands in, plans the
candidate and commits exactly the previewed plan.

| Program | `params` |
| --- | --- |
| `wish.xllm@1` | `wish_id`, `stage: analyze \| code \| run`, `parent_run_id?` (code: a failed run to repair, or the previous round) + `feedback?` (a feedback round), `location?` (`{ surface_id, cell_id, selection, viewport }`, the task location of the map), `request?` (`aiws.request`), `execution?: { mode: local \| hosted }` |
| `wish.mock@1` | `wish_id`, `stage: run`, `provided` — the browser Mock executor's results, planned and applied like a real run |

States: `queued → snapshotting → running → validating → waiting_confirmation → applying → succeeded`, or
`failed | cancelled | interrupted | conflict | rejected`. An executor's lapsed lease makes its run `interrupted` (resumed as a new
attempt); an `applying` run is reconciled through the Commit idempotency record `run/<run_id>/<plan_digest>`. `proc.get { run_id,
choices? }` returns progress, the candidate and — while waiting — the preview of the plan for `choices` (`{ results: { <name>: keep |
replace | new }, confirm_structure? }`); `proc.apply { run_id, plan_digest }` applies exactly that previewed plan in one Commit
(origin `program`).

- **analyze** (`analyze_model`): the model reads `WORKSPACE.md` (the context map) and the read-only tools
  `ws_outline / ws_find / ws_profile / ws_neighbors / ws_read / ws_query`, and delivers through `submit_analysis` (checked on the
  spot). Applying writes `analysis` (`wish.analysis.v3`, no `approach`: every result comes from the macro; `basis.digest` is
  computed by core) and the named `inputs`. An analysis made elsewhere (Jarvis in a conversation) is handed in with
  `proc.report { analysis }` on an analyze run the Agent started itself (`buckyos aiws run-start`).
- **code** (`execute_model`): needs a current, ready analysis (`NEEDS_ANALYSIS`, `NEEDS_INPUT` otherwise). The read tools plus
  `run_program { source }` (the executor runs the macro and hands it in with `proc.program`), `check_results`, `finish`; there is
  no direct writing. `finish` needs a successful trial run with a manifest. The candidate is the trial run's: applying writes the
  macro (`program`: `{ language: js, api_version: 3, source, digest, manifest, produces }`) and its results in one Commit. A repair
  or a feedback round starts from the parent run's macro, else from the stored one.
- **run**: needs that and a stored macro that still fits its inputs — `macro_check` (core) on the manifest: a declared field
  deleted or of another type, an input gone or rebound, another program API → `NEEDS_CODING` with `data.reasons`; a renamed field
  or table is no reason. The brief is `{ kind: program, source, manifest, budget }`; no model loop. Only the declared inputs and
  fields are materialized, under the names they had when coded. A failed macro ends the run (`PROGRAM_FAILED`; an HTML result whose
  code differs from the coded `code_digest`: `MACRO_CODE_CHANGED`; a refused AI call: its `AI_*` sub-code); running never
  rewrites an HTML definition, only its bindings move.

**The macro and its bridge.** The program host is `aiws/aiws.js` (API v3; guide: `aiws/README.md`). A macro starts with
`aiws.declare({ inputs: { <name>: [fields] }, ai?, zone?, params?, resources? })`; reading anything undeclared throws. The
service resolves the declaration into the **manifest** (`{ inputs: { <name>: { entity_id, type_id, fields: [{ field_id, name,
type }] } }, results: { <name>: { type, code_digest? } }, ai?, zone?, params?, resources? }`, `core::wish::check_manifest`), shown in
the wish's `body`. Every executor runs `lib/run.js` exactly the same way; what leaves the snapshot goes through its bridge
(`globalThis.__aiwsBridge(op, req)`; the hosted runner speaks it over stdio, `lib/run.js --bridge`), answered by `aiws/host.js`:
`sql` → `proc.tool sql`, `ai.complete` / `llm.map` → the executor's own AI session with `proc.tool llm_cache` (kept per AI-paying user:
one person's executor never answers another's runs), `zone.fetch` → the executor's session, the Zone's hosts only (`ZONE_ONLY`), GET / HEAD
only (`ZONE_READ_ONLY`), under a prefix of `declare.zone` only (`ZONE_NOT_DECLARED`; 权限需求 R34), `ai.image` / `ai.video` → one AICC
generation with the executor's session (`helper.text_to_image`, `image.img2img` after `route.resolve` when the macro gives reference
pictures, `helper.video_generate`; a running AICC task is followed in the task manager up to `budget.media_timeout_secs`), whose
output (a named object) the executor imports with `asset.import_object { obj_id }`: the macro gets a media reference `{ kind,
object_id, media_type, size, … }`, never the bytes, and writes it with `aiws.result.file(name, media)` — a file result
`{ object_id }` that staging accepts only for an asset this Workspace has or staged (else the run fails: `… is not a file of this
Workspace`), re-staged with its sniffed image size and a file name. Generations are cached in `llm_cache` by the request (the hit is
imported again for the run). Waits for generated media do not count against the macro's time limit. Limits: the smaller of what the
macro declares and the stage's `budget`; an undeclared AI capability is `AI_NOT_DECLARED`, a spent one `AI_BUDGET_EXCEEDED`.

**Stage tools of an executor** (`proc.tool { run_id, attempt, name, args }`, answers `{ status: ok | error, text?, code? }`): the
stage's model tools, and

| Tool | Args → answer |
| --- | --- |
| `sql` | `{ query, params?, declare? }` → `{ sql: { columns, rows, truncated } }`: one SELECT over the declared table inputs only (table = input name; columns `_id`, the declared fields under their declared names, `"<f>#raw"`); a view input is its view's rows; coding resolves `declare` by field name, running uses the stored manifest. Per run at most `budget.sql_max_calls`. |
| `llm_cache` | `{ get: [key…] }` → `{ hits: { key: value } }`; `{ put: [{ key, value }] }` → `{ ok }`. Keys `^[0-9a-f]{64}$` (at most 2000 a call), values up to 64 KiB. The service only keeps the answers. |

`proc.claim` answers the stage brief with `budget { iterations, program_timeout_secs, max_output_tokens, map_model, complete_model,
llm_map_max_items, ai_max_calls, ai_max_tokens, sql_max_calls, image_model, video_model, image_max_calls, video_max_calls,
media_timeout_secs }`; `proc.files` answers `{ files, stage: { budget, manifest } }` —
`context/` (only the inputs; no data or Block tree), `request.json`, `catalog/`, `lib/` (`aiws.js`, `run.js`, `host.js`),
`program/main.js` when there is one — and over `snapshot_download_max_bytes` fails the run with `SNAPSHOT_TOO_LARGE`
(`data { bytes, limit }`). `proc.program { source, results, log, files }` judges a macro run: coding answers what the model reads
(previews, checks, manifest, problems); running, a failure ends the run. `proc.report { finished | analysis | error | progress }`;
results are never reported directly.

**Execution modes (内核架构改进 M3; 浏览器优先).** `proc.start { params: { …, execution: { mode: local | hosted } } }`
(`wish.default_execution` when absent; any other configured value counts as `local`). Every run records its execution context
(`detail.context`: trigger, executor, location, AI and compute attribution, the bound task digest, the declared outputs, expiry) or is
refused with `EXECUTION_CONTEXT_MISSING`. `local`: the starter is the executor — the `proc.start` answer carries a one-time
`claim_token`; it claims, heartbeats and reports; its model calls are its own (`aicc: { mode: current_user, user_id: <starter> }`;
an Account, or an Agent that started the run itself; anybody else is `AI_ATTRIBUTION_UNAVAILABLE`). `hosted`: the Owner's Agent
(`ws.set_hosted_exec`) is given a Goal with the run reference and claim token (`goals.create` + `goals.activate` on its OpenDAN
service), uses the owner's AI resources and works through `buckyos aiws run-*`; without a Zone runtime the dispatch is recorded
`unavailable` and the run waits to be claimed. Applying checks the delegation: a changed wish configuration is `DELEGATION_STALE`, a
candidate of a superseded attempt or a cancelled run is `RUN_REVOKED`, and results may only land where the task declared
(`OUTPUT_NOT_DECLARED`). The hosted configuration's details in a run (`context.executor.limits`, `config_digest`, the Agent's goal
in `dispatch`) show to managers only; `limits.max_tool_iterations` counts the model's stage tools, never a macro's `sql` /
`llm_cache`.

**The macro identity (权限需求 §4.5, Q31–Q32).** Coding and running read and write as the macro identity: the trigger's permissions
as they are now, bounded by the wish's group when it is bound with `intersection` (`acl.bind_macro`; `union` adds nothing until
trusted executors). It bounds the stage snapshot, the macro's `sql`, the run token's reads (an input the group cannot read fails the
run: `DEPENDENCY_UNAVAILABLE`), the preview's pre-check and the apply; an analysis is written as its analyst. A run is applied as
that identity whoever clicks, and it always writes the wish (`last_run`, results) — so `proc.start` of `code` / `run` refuses with
`PERMISSION_DENIED` / `MACRO_CANNOT_WRITE` (`data { missing, group_id }`) when the identity cannot update the wish or write where its
results go: someone holding only `run` cannot run anything yet. An HTML page a run writes (origin `program`) needs no `manage`, but is
`trusted` only when the identity manages the definition; a manager vouches later (`entity.set_keys trusted: true`), which is no
hand edit of the result. `trusted` is this Workspace's state, not content: packages, templates, clones and versions carry the code
without it, and whoever brings it in decides again. Only writes of `program` on a bound wish are audited (`macro.changed`). `aiws.zone.fetch` carries the
runner's session: a macro written by someone with `update` reads, when a manager runs it, what that manager may read (a known risk,
Q33). A wish's freshness shows each viewer what it may read: an input it cannot read is `input_problems: [{ reason: unreadable }]`
and never a `needs_coding` reason about its fields; core `main_status` calls that `no_access` (next action `none`).

Configuration: `AiWorkspaceSettings.wish` — `analyze_model`, `execute_model`, `map_model`, `complete_model`, `image_model`,
`video_model` (AICC selectors), `analyze_tool_iterations`, `execute_tool_iterations`, `max_output_tokens`, `program_timeout_secs`,
`llm_map_max_items`, `ai_max_calls`, `ai_max_tokens`, `sql_max_calls`, `image_max_calls`, `video_max_calls`, `media_timeout_secs`,
`snapshot_download_max_bytes`, `default_execution`: what every stage hands its executor. Standalone: `--wish-config <json>` with the same keys.

## Workspace settings and personal preferences (UI 框架 PRD §4.6, §12, K4)

Shared configuration is content. The **Workspace's settings** are the root's `config` (an object of at most 64 KiB; the
client owns the keys, `toolbar` first): written with `entity.set_keys` / `entity.unset_keys` / `entity.set_props { key:
config }` on `root`, which needs `manage` on `/` besides the usual `update`; committed, undoable, carried by clones,
packages and templates like the root's title. Every member reads them in `ws.get_info` (`settings`, `settings_rev`: the
`config` key's version for `expect`), whatever it may read of the tree. A **Surface's** toolbar is `config.toolbar` on the
Surface; changing it needs `structure` on that Surface. A load (package, clone, template) and a replay take both as they
are.

**Personal preferences** are not: `user.get_prefs` / `user.set_prefs` keep one JSON object per principal in the service's
`users.sqlite` (each Workspace has its own origin, so browser storage cannot carry a preference from one to another, nor
to another device). They follow the person into every Workspace and are never exported, cloned or carried by a template.
`ws.get_info { record_open: true }` (a Workspace session opening) records when the caller opened it there too; `ws.list`
returns it as `opened_at`.

## Storage

Durable data schema: [BuckyOS AiWorkspace Durable Data Schema](<../../../doc/workspace/BuckyOS AiWorkspace Durable Data Schema.md>).

```text
<data>/workspaces/<workspace_id>/doc.sqlite     the portable document (design §4.2)
<data>/workspaces/<workspace_id>/local.sqlite   grants, share entries, locks, runs, staged uploads — never exported
<data>/workspaces/<workspace_id>/runs/, cache/  wish run working folders, the llm.map cache
<data>/trash/                                   deleted / replaced Workspaces (30 days)
named store                                     every object: assets, snapshots, versions, packages (CYFS ids)
user/aiworkspace/<owner>/assets   (data, app_data)  <ws>/<object_id>                  staged and referenced assets
user/aiworkspace/<owner>/objects  (data, app_data)  <ws>/snapshot/<id>, <ws>/version/<epoch>/<seq>   retention sets
user/aiworkspace/<owner>/exports  (data, temp 7 d)  <ws>/export/<id>.bcanvas, <ws>/replica/<id>.sqlite, <ws>/fixed/<root>
```

**Retention and quota (内核架构改进 D06, D19).** What a Workspace keeps is a binding in its owner's buckets; the
named store reclaims what no binding holds. An asset is bound from its upload on: while staged (24 h unless a Commit
references it), while an alive entity references it (`assets.ended_seq IS NULL`) and through its retention period as
history (`history_retention_days`, 30 by default); `ws.purge_deleted` ends it at once. Every snapshot (checkpoints, exports,
clone sources, import checks) and the versions a Commit generates are bound as one **retention set**: a DirObject naming every
object of their closure, so the named store's cascade (which follows Dir / File / ChunkList, not `jobj`) keeps all of it.
`ws.delete_checkpoint` / `ws.release_retention` unbind; the hourly sweep (`Service::sweep`) unbinds expired uploads and
history and, after 30 days, what a trashed Workspace held. The `usage` ledger (`ws.get_info.usage`: `content`, `assets`,
`clones`, `staged`) moves with every transaction; with a quota (`quota_bytes`) a Commit, an upload, an export or a show clone
that would pass it is refused with `QUOTA_EXCEEDED` (retryable: the maintenance methods can release enough) or `QUOTA_LIMIT`,
with `data { needed_bytes, reclaimable_bytes, quota_bytes, used_bytes, owner }`. The maintenance methods above are accepted
over quota. An undo or version restore that needs a released asset answers `RESOURCE_RECLAIMED`. The service's buckets also
count on the owner's app pool (`storage_quota_mib`).

**Shows (D17).** A clone judges access by the grants of the Workspace it was cloned from as they are now
(revoking there ends the audience's access here); any write right, a subtree's included, locks the Workspace
for the show; a Workspace runs at most 4 clones at once and each counts on its quota (`clones`).

`storage_schema_version` is 6 (FORMAT 1.0: `commit_deltas`, `usage`, `user_state.revision`, `refs.mode` / `integrity`, `entities.schema_id`, `grants.task_digest` / `outputs_json`; 5: `bucket_key` / `bucket_version` on `assets`, `snapshots`, `entity_versions` and `staged_assets`, no `retentions`; 6: every entity named — a version-5 folder is opened and its unnamed entities named in tree order, no Commit, see “Paths and names”); folders older than 5 are refused, not migrated —
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

## Finding, reading and querying (核心 API 调整, `notepads/aiws-core-api-unification-todo.md`, design `notepads/aiws-core-api-design.md`)

One interface for the vague reads — finding, plain-text reading, text and SQL queries — and the common properties;
precise reads and writes stay with each type (`doc.read` with a selector, `doc.commit` with typed operations). Code:
`core/src/naming.rs`, `path.rs`, `body.rs`, `search.rs`, `meta.rs`, `sqlview.rs`; `store/src/sql.rs`.

### Paths and names

- **Every entity but the root has a `name`** (R14, R19), unique among the live siblings that share its namespace (shared
  names are seen by everyone; a personal or participant entity also meets its own scope). A name nobody chose
  *follows* the entity: its title's name (`title`; a record's `p:title` / `p:name`; an asset's `title` or `file_name`) —
  control characters dropped, `/` → `-`, no leading `~`, at most 64 characters — moved aside with `-2`, `-3`… when
  taken; without a title a counter, `<prefix>-<n>` one above the largest `n` of that prefix among the siblings (deleted
  ones too): a Block's view type (`text-3`, `table-1`), a container's kind (`group-2`), else a short type name
  (`table`, `doc`, `record`, `file`, `note`, `wish`, `def`, `dir`, `viewport`, `path`, …). Tables and rich texts have
  no title: their name is their label. Whether a name follows is not stored: it follows while it is still what the
  rule gives. The planner names on `entity.create` (the normalized operation records the name, so replay and replicas
  agree), renames with the title in the same Commit (`entity.set_keys`), moves a following name aside on `tree.move`
  and `entity.restore`, and `entity.rename { name: null }` gives the name back to the rule. A name given on
  `entity.create` that is just the title's name moves aside like a following one; any other name that is taken is
  `NAME_CONFLICT`. Names: 1–128 characters, no `/`, no control characters, not starting with `~`, not `.` / `..`.
  Storage schema 6: a version-5 store gets names for its unnamed entities when it is opened (`naming::backfill`, tree
  order, no Commit); an imported package of an older build is named after its content root is verified.
- **One path syntax** (R5) for `doc.resolve`, `doc.list` / `doc.find` / `doc.query`, `doc.read { path }`, the object
  paths, the stage tools, the CLI and the HTML host's `aiws.resolve`: `/` is the root, each step a name
  (`/data/销售`, `/surfaces/看板/text-3`); `~<entity_id>` starts at an entity (`~sales/…`). Every step must be readable
  and live, otherwise `NOT_FOUND` (not readable and not existing look the same). A printed path starts at the highest
  ancestor the caller reaches by names: an unreadable ancestor is never named, the path starts at `~<id>` below it.
  When a caller sees several siblings of one name (a shared one and its own personal one; a reviewer and several
  participants'), the name reaches the shared one, then the caller's own, then the first; the others are *shadowed*
  (`shadowed: true` in a listing) and reached by `~<id>`.
- **Type filters** (`type`, `not_type`: a comma list or an array): short types `table`, `richtext`, `record`,
  `annotation`, `block`, `container`, `asset`, `dir`, `wish`, `def`, `viewport`, `path`, `task`, `response`, `result`,
  full `type_id`s, or `container:<kind>`. `doc.find` and text queries leave out `annotation`, `viewport`, `path`,
  `task`, `response` and `result` unless `type` names them.

### Common meta and body

`doc.read { as: "meta" }` (also the summary property `meta` of an entity object) is what an entity is, without reading
its text or its records: `entity_id`, `path`, `name`, `name_follows`, `title`, `type`, `type_id`, `kind` (container
kind, Block view type, annotation kind), `schema_id`, `schema_version`, `scope`, `parent { entity_id, path }`,
`version { content_rev, meta_rev, life_rev, struct_rev, epoch, head_seq }`, `body { state, format, format_version,
etag, records, bytes, reason? }`, `data { mode: local | url_query | synced, write: edit | refresh | none, source?, derived? }`
(data entities; `write` says who writes the content: Commits under `capabilities`, only the Source's refresh — edits are
`SOURCE_READ_ONLY` — or nobody here, the rows being remote; `capabilities` and `ops` stay the ACL's and the type's),
`children`, `capabilities` (the caller's, as everywhere), `ops` (the Commit operations its type takes: the `types` column
of `schemas/operations.json`), `shows` (a Block: what it shows `{ entity_id, path, type }`, a table view with
`view_sql`, `view_count_sql`, `view_group_sql?`, `view_diagnostics?` or `view_error`; a control: its bindings),
`definition`, `ref`, `degraded?`, `lock_holder?`, `availability?`.

`doc.read { as: "body" }` (the property `body`) is the entity in plain text (R6) — what Agents read, text queries match
and a screen reader will speak: a sequence of parts joined by newlines, paged by parts (`max_bytes` 64…262 144,
default 32 768; a part longer than a page is cut on a character boundary and marked `partial`), `next_cursor` bound to
the `etag` (a changed body is `SNAPSHOT_EXPIRED`); `selector` reads one part; `parts: true` adds each part's
`{ selector?, entity_id?, start, end }` (UTF-8 byte offsets in `text`). `state`: `full`, `definition` (a URL table:
its definition only, rows are remote), `fallback` (the native JSON of an unknown or degraded entity, and of viewports,
paths and session entities) or `unavailable` (`reason: binary | too_large`). Per type (`BODY_FORMAT_VERSION` 1):

| Type | Format | Text |
| --- | --- | --- |
| rich text | `text/markdown` | each top-level block's Markdown (`richtext_block` parts) |
| table | `text/csv` | header `_id,<field names>[,_body]`, then one record per line (`table_record` parts) in `record_id` order; values as shown (§“Display values”) |
| record | `text/plain` | `<property>: <value>` per property (`doc_key` parts) |
| annotation | `text/plain` | `Note on <target path>`, the quote, the text |
| folder, data, surfaces | `text/plain` | one line per readable child: `<name>[/] <type or kind> "<title>"` |
| canvas, group | `text/markdown` | `# <title>`, then each Block in reading order (top to bottom, left to right) as `[<name>] <Block text>` |
| Block | `text/markdown` | its title (a text view: the title itself), what it shows by path (a table view: the table's path, the view in words, its columns and its SQL; a note: the note's text), `config` text keys. It never inlines the content of the data it shows |
| control | `text/markdown` | a mind map's Markdown outline of its Blocks; a data control names the tables it shows |
| asset | `text/plain` | file name, media type, size; the content of a text asset up to 256 KiB |
| directory | `text/plain` | one line per member (`folder_member` parts) |
| wish | `text/markdown` | title, prompt, inputs by path, output, last run state |
| definition | `text/markdown` | title, description, agent text, config properties, bindings |

**Display values** (body, the display columns of SQL): select → label; multi_select → labels joined `; ` (SQL: a JSON
array of labels); object_ref → `<title> (<path>)` (SQL: the path), `~<id>` when unreadable; currency → `<code>
<amount>` (SQL: the amount as REAL, the code in `<name>#currency`); decimal → the stored string (SQL: REAL); boolean →
`true` / `false` (SQL: 1 / 0); date, datetime, text → as stored; json → compact JSON; formulas and empties → empty. The
`etag` covers the field definitions and the referenced entities' versions: renaming an option or a referenced entity
changes the body, not the record.

### Discovery

`doc.list { at?, type?, not_type?, schema?, limit?, cursor? }` lists the readable direct children of a node (default
100, at most 1000); `doc.find { at?, depth?, name?, type?, not_type?, schema?, limit?, cursor? }` walks under a node
in pre-order (not into what the caller cannot read; depth ≤ 16; default 50, at most 200; at most 20 000 entities per
call, `complete: false` and a cursor otherwise), keeping the entities whose name or title contains `name`
(case-insensitive, literal; `match: name | title`). Items: `entity_id`, `name`, `path`, `type`, `type_id`, `kind?`,
`title`, `children`, `content_rev`, `body: { state, format }`, `schema_id?`, `scope?`, `shadowed?`. Cursors are bound
to `head_seq` (a moved Workspace is `SNAPSHOT_EXPIRED`) and to the request.

### Text and SQL queries

`doc.query` takes exactly one of `text` or `sql` (R3, R11); the old table query (`view_id`, `source_id`, `filter`,
`sorts`, `fields`, `group`, …) is refused, and a URL table's rows are `doc.source_query`.

- **Text** `{ text, case?, at?, depth?, type?, not_type?, limit?, cursor? }`: a literal (1–256 characters; `regex` and
  the like are refused), case-insensitive by Unicode lower-casing unless `case: true`, in names, titles and bodies —
  what `body` reads, so a select is found by its label and a hit's `selector` (`table_cell`, `richtext_block`,
  `doc_key`, `folder_member`) points into the text the caller reads. Containers are matched by name and title only
  (their bodies are their children). The entity at `at` (unless the root) and its subtree in pre-order. One hit per
  matching text, at most 5 per entity (`more` counts the rest), 20 by default, at most 100; `snippet` is 40 characters
  on each side. No index: a call scans at most 8 MiB of text or 1 second; `coverage { complete, scanned, bytes,
  skipped: [{ entity_id, reason: definition | binary | too_large }] }` says what was covered (`definition`: a URL
  table, whose name, title and definition were searched but not its remote rows), and an empty answer means
  "not there" only when `complete` is true (otherwise `next_cursor`, resuming inside a table at its record).
- **SQL** `{ sql, params?, tables?, max_rows?, expect_head? }`: one SELECT run by SQLite itself (R12) on a read-only
  connection, over **logical tables**: the temporary view of a local table, created the first time the statement names
  it — by its path (`FROM "/data/销售"`), `"~<entity_id>"`, its id, or an alias of `tables: { alias: path | id }` (with
  `tables`, only those). Columns: `_id` (record id), `_rev`, `_body` (the path of the record's body), `_record` (the
  record as `doc.read` gives it, as JSON text: what the Desktop renders and edits), and per live field `"<name>"` (the
  display value) and `"<name>#raw"` (the stored value: option ids, the exact decimal, a reference or currency object as
  JSON text); a currency field also `"<name>#currency"`; a field named like a system column is `"<name>#field"`.
  Joins, CTEs, sub-queries and window functions work. A logical table's column is an expression over JSON, so a join
  on it cannot use an index: with large tables put the smaller side in `WITH d AS MATERIALIZED (SELECT … FROM "/data/部门")`
  and join to `d` — SQLite then indexes it (100 000 × 1000 records: ~0.12 s instead of past the budget; the time-limit
  error of a query over several tables says so). The store keeps `sqlite_stat1` of `table_records` for the planner
  (refreshed at open when missing and after writes passing a tenth of the records; the STAT4 samples are dropped, they
  make SQLite scan the large side once per row of the small one). An authorizer allows only SELECT, reads through the logical
  tables and functions (not `load_extension` and the like): kernel tables, `table_records` directly, writes, `ATTACH`,
  `PRAGMA` and recursive CTEs are `INVALID_OPERATION` / `SQL_NOT_ALLOWED`, a syntax error `SQL_ERROR`, a table the
  caller cannot read or that does not exist `NOT_FOUND`, a URL table `NOT_MATERIALIZED`. Budgets: 16 KiB of SQL, 2
  seconds (`LIMIT_EXCEEDED`, `data.limit: time`), `max_rows` (default 1000, at most 5000; more is `truncated: true`),
  4 MiB of result (`LIMIT_EXCEEDED`, `bytes`). A query runs with the Workspace held: commits to it wait for it (at most
  the 2 seconds; a text query at most its 1 second). Answer: `columns`, `rows`, `truncated`, `read: [{ table, entity_id,
  path, content_rev }]` (the tables actually read, with their versions), `head_seq`. Comparisons, ordering and NULLs
  are SQLite's; the functions `aiws_dec_cmp(a, b)`, `aiws_label`, `aiws_labels`, `aiws_pick` and the collation
  `aiws_dec` (exact decimal order) are there for the compiled views. `expect_head: { epoch, seq }` refuses a Workspace
  at another head (`SNAPSHOT_EXPIRED` / `HEAD_MISMATCH`): the Desktop sends its confirmed head and runs no SQL while it
  has unconfirmed changes (R16). SQL runs on the service and on a wish stage's snapshot, never on a browser replica.
- **Table views as SQL.** A table view Block keeps its definition (`filter`, `sorts`, `group`, `manual_order`,
  `fields`); `sqlview::compile_view` turns it into the SELECT that gives the rows, the order and the group counts of
  the evaluator (`filter::run_query`, kept for the browser replica, R16) — `store/tests/sql.rs` holds them side by side.
  It appears in the Block's `meta.shows.view_sql`, is what the stage tools and wish inputs (`table_view`,
  `table_query`) read through, and what the Desktop runs when a tab holds no replica (WASM `view_sql`).

### Objects

`obj://aiworkspace/<ws>` is a directory: its index (one index, R1) runs `op=ls` (default), `op=find&name=…` or
`op=grep&text=…` with `at`, `type`, `not_type`, `schema`, `depth`, `case` in the URL query or in the `query` object of
the action (`page_size`, `cursor` page it; method `doc.index`); rows link to entity objects (a body hit with its
selector) and have the columns `type, kind, name, title, path, parent, on, selector, snippet`. An entity object's
summary property is `meta`, then `body` (`?cursor=&max_bytes=&parts=true`, `?selector=`) and `content` (the native
content; `?mode=fixed&version=<json>` reads that version or fails, R2). HTTP status by error code: `PERMISSION_DENIED`
403; `NOT_FOUND`, `TARGET_DELETED`, `VERSION_NOT_RETAINED`, `REFERENCE_BROKEN` 404; `INVALID_OPERATION`, `INVALID_SCHEMA`,
`UNSUPPORTED_VERSION` (a fixed version of `meta` / `body`) 400; others 500.

### Writing

Precise writes are Commit operations by type through `doc.commit` (R8): `meta.ops` lists what a type takes and
`schemas/operations.json` describes each (`types`, `capability`, `class`); the twelve the helpers build have `params`, a
JSON Schema in `commit-request.schema.json` `$defs` (`core/tests/schemas.rs` holds them against the planner: what a
helper builds conforms and is accepted, an unknown parameter is refused by both). The websdk builds the frequent ones
(`aiws_write.ts`: rows and cells by field name and option label, fields, options, rich text blocks from Markdown,
rename, move, properties — with `expect` from what was read and an idempotency key) and `buckyos aiws` has the verbs.

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
nothing: `ws.create`, `ws.import`, `ws.clone` and `ws.list_subjects` refuse them
(`session_kind_not_allowed`). `ws.list` shows anyone only the Workspaces it owns or whose entries or groups name it —
never one it could only enter anonymously.

**Permissions** (权限需求 `notepads/aiws-permission-system-prd.md` §4; `store/src/acl.rs`, `core/src/access.rs`). One ACL of
entries `subject × resource → allow, deny` (9 capabilities), kept in `local.sqlite` (`acl_entries`), never in the
document's history. Interfaces name a resource by the core API path (`/`, `/data/销售`, `~<id>/…`) or an entity id;
storage binds the entity id (`""` = `/`), so a rename or a move keeps the entry with its entity. Each step of a path must
be readable; personal and session scoped entities are not targets. Subjects:

| Subject | Who |
| --- | --- |
| `<user id>` / `<did>` / `guest:…` / `link:…` | that single caller (an Agent's DID too: naming it anywhere here makes the Agent a subject of its own, Q29) |
| `@<group id>` | the members of a logical group of this Workspace (`acl_groups`, `acl_members`) |
| `@owner` | the owner (`local_meta.owner`): always everything, takes no entry, cannot be refused |
| `@all` | everyone who may enter: group members, callers with an entry, `@guest` when it exists; never `manage` / `run` |
| `@guest` | anonymous visitors and signed-in callers in no group — only while anonymous access is on (`acl.set_anonymous`); never `manage` / `run` |
| `@creator` | for each entity, the principal that created it (`entities.created_by`); never `manage` |
| `@pub:<entity id>` | the readers of that entity's inner-path publication (`ws.publish`), `read` only; they come through the publication routes, never into the Workspace |

Group members (never another group): a principal, `group:<id>` (a system user group: `users/<id>/settings.groups`), `contact:<owner_did>#<tag>`
(that owner's contact group, msg-center `contact.check_membership`; only the owner, caller DID = `<owner_did>`, adds one),
`*` (every unrestricted account; not Limited, External, guests or services).

A capability on entity E is decided by the nearest layer — E, each ancestor, then `/` — with an entry mentioning it; in that
layer by the most specific tier mentioning it — single caller, then groups and `@creator`, then `@guest` and `@all` —
and inside the tier a `deny` wins. Nothing mentions it: refused. Personal and session scopes are judged as before
(`user:` / `presenter` / `participant:`). An **Agent** acts as itself; while no entry or membership names its DID here, it
is judged as its owner (from its AgentSpec, never from the request) and Workspaces it creates belong to that owner (R16).

Group and contact membership is settled per request, before the Workspace's writer is locked: the server lists the
Workspace's `group:` / `contact:` members (every Workspace's for `ws.list`) and asks system-config (the account's
groups; its DID from `users/<id>/doc`) and msg-center — for an Agent judged as its owner, the owner's. A membership that
cannot be checked refuses the request with `authorization_unavailable` when that group holds any entry (one group less
may turn a refusal into an allowance), never for the owner; nothing is cached. A show's notes are read with the
presenter's memberships settled again at each `show.notes`.

Only the owner (R17): giving or taking `manage` (an entry touching it, the members of a group holding it), `ws.delete`,
`ws.set_hosted_exec`, a restore over the Workspace (`ws.import restore`). Managers do the rest: groups, members, entries
without `manage` (denials included), anonymous access, share entries, publications, macro bindings, maintenance. Every
change is one record in `acl_audit` (who, judged as whom, when, the object with its path then, before, after; `acl.audit`),
bumps `acl_rev` (`ws.get_info`), wakes `doc.wait_changes { after_acl_rev }` and publishes the kevent
`…/aiworkspace/<ws>/acl`. A purged entity takes its entries along; a deleted one keeps them for a restore.

**The permission design** — group definitions, the entries of groups and fixed groups, macro bindings — travels with a
template (`templates/<id>/permissions.json`, written when a manager exports), a Clone and a package (`permission_design`
in the manifest) made by a manager, and loads into the new Workspace with no members and anonymous access off. Members,
single callers' entries, the anonymous switch and share entries never travel; a restore over a Workspace keeps that
Workspace's own.

**Guests.** A link's visitor has no session: `sys[1]` (or the bearer of the asset route) carries a guest
token instead, `gt_` + base64url of 32 random bytes (`name_lib::bearer_token` form) the browser made up and keeps. The caller is
then `guest:<26 chars>` (derived from the token's SHA-256, never the token itself), reaches `@guest`'s entries while
anonymous access is on and what a share entry it joined gives, and cannot own anything. `ws.get_info` returns `principal`
(and `judged_as`) so a guest learns who it is.

**Share entries** (`local.sqlite` `share_entries`, R8): a link that offers ways in — `methods`
`{ guest, password, self_register, did_login }` (`self_register` implies `password`) — into one logical group (never one
holding `manage` or `run`), optionally until `expires_at` (unix seconds) and for at most `max_joins` people. The id names
its Workspace, `<workspace_id>~se_<26 chars>`, so a link resolves without a service-wide index; the visitor's page is
`/sso/entry?service=aiworkspace&entry=<entry_id>`.

- Nobody gets anything from an entry without presenting it: `ws.join_entry` makes the visitor a member of the group
  (`via_entry`). A signed-in visitor (account or External) joins if the entry ∩ the Zone switches (`system/sso/settings`)
  admit the session's sign-in method — `did_federation` needs `did_login`, any local method `password` / `self_register`
  — or if the entry is open to guests; a guest (its token) joins an entry open to guests. The entry page joins a signed-in
  visitor itself; "continue as a guest" goes to the workspace's page with `?entry=<entry_id>`, which joins with that origin's
  own guest token before the workspace opens (Desktop `StandalonePage`). Joining again changes nothing;
  nobody becomes the owner. A member of a group is no longer in `@guest`.
- `ws.close_entry { entry_id, remove_members? }` stops new joins; with `remove_members` those who joined through it leave
  the group too.
- `entry.describe` is public (no session): `buckyos_api::EntryDescription` (`state`, `resource: { kind: "workspace",
  title, owner_name, path }` — `path` is the workspace's page on its own origin, `https://aiws-<id>.<zonehost>/<workspace
  slug>`, or `/` without a public base —, the entry's own `methods`, `expires_at`) plus `ok: true`; a missing,
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
schedule); `doc.read { as: "meta" }` gives `data { mode: synced, write: refresh, source }` (a URL table: `url_query`,
`none`), the body and SQL of a sourced table are its last valid content. Publishing (`ws.publish`) is the other side: the `public` subject reads the entity, and its link
`https://aiws-<workspace_id>.<zonehost>/<workspace_name>/<name>` (or `GET /kapi/aiworkspace/pub/<workspace_id>/<name>`) answers the
publication — JSON (`format: buckyos.ai-workspace/pub/1.0`, `reference`, `content`; `?fixed=1` adds `content_root`; `?fixed=<root>` reads
that pinned version) or, for a browser or crawler (`Accept: text/html`), a static HTML page with the title and the table or text.
`POST …/referrers` registers a consumer of another Zone; same-service consumers are registered by their refresh.

**The Workspace's own host** (T4-8, `server/src/pages.rs`): everything else on `aiws-<id>.<zonehost>` is the AI Workspace page itself —
the Desktop build's `aiws.html` entry and its files (`assets/…`, `aiworkspace/pdfjs/…`, `sw.js`), served from the directory `--web` names
(standalone) or, in a Zone, `web/` beside the executable, else `../control-panel/web/` (both are kernel services of one node). `/` and
`/<workspace slug>` are the page; `/<workspace slug>/<name>?id=<surface id>` is the page on that Surface (the names are for people and
crawlers, the id decides); `/<workspace slug>/<name>` without `id` answers a publication of that name, else sends a Surface named by name,
id or title slug to its canonical `?id=` address (the rest of the query, `block=` say, travels along), else the page. The workspace slug is
the title slugified, `workspace` when empty or one of the first segments the gateway routes elsewhere (`home`, `kapi`, `s3`, `bucket`,
`ndm`, `nfs`, `assets`, `aiworkspace`, `api`, `messagehub`). Without a web directory the page is 503. The page signs in on its own: the
Zone host's `/sso_callback` sets host-only cookies there (the Control Panel resolves `aiws-*` hosts to the `aiws` gateway entry and the
`System(aiworkspace)` login target; `auth.prepare_app_launch { service_id: "aiworkspace", redirect_url }` delegates a Desktop session).

### Validator classes (D01, D04)

Every built-in type is checked by a Rust validator in [core/src/types.rs](core/src/types.rs) (tables and rich text also in
[core/src/plan_table.rs](core/src/plan_table.rs) and [core/src/plan_richtext.rs](core/src/plan_richtext.rs)). The checks fall into
three classes: **primitive** — the shape of a storage primitive itself (tree node, keyed object, Table, RichText, AssetRef);
**reference** — a target exists, is readable and is of the admitted kind; **schema** — a business rule of the built-in type
(value ranges, counts, which Surface a thing may sit on). Only primitive and reference checks are part of replay semantics; schema
checks run at admission only (D01 方案 C, Q15). `init_entity` / `validate_update` go through two entries: `validate_primitive`
(every path) and `validate_schema` (only when `PlanEnv.schema_checks`: admission of a Commit). Replay never enters the second: a
replica re-applying accepted operations and `ws.import` (a package another deployment admitted) plan with `schema_checks` off, and
the pure-application log applies row deltas without a planner at all. `types::schema_validations()` counts this thread's entries
into `validate_schema` — `core/tests/composite.rs` `replay_runs_no_business_schema` proves replay leaves it unchanged, and
`core/tests/apply_log.rs` that a log breaking a business rule but keeping the primitives replays. The reference checks of the
schema-class validators below (a Cell's `source_ref` / `bindings` / `def_ref`, record `object_ref`s, wish inputs, annotation
targets, Viewport and step targets) still sit inside their validators and so run at admission only; on a replica and on import
they were judged where the Commit was admitted. The doc comment of each validator starts with `Validator class:`.

| Type | Validator | Class | Checks |
| --- | --- | --- | --- |
| table-source | `init_entity` (`TYPE_TABLE` branch) | primitive | field ids unique (`ID_CONFLICT`), field names unique (`NAME_CONFLICT`), order keys, `FieldDef` shape |
| table-source | `validate_table_meta` | primitive | `title_field_id` names a live text field; `data_mode` never changes; `embedded` carries no `source_ref` |
| table-source | `check_query_reference` | schema | `url_query` source: absolute URL without credentials, `query` object without cursor, version `live_head` / `fixed_revision{source_revision}` |
| table-source | `plan_table.rs` `check_unique` | primitive | `unique` is a kernel-enforced cross-record constraint (`UNIQUE_VIOLATION`) |
| table-source | `plan_table.rs` `required` / `nullable` checks (insert, set/unset, add/update/migrate field) | primitive | required values present, nulls only where allowed, tightening (`required`, `nullable`, `max_items`) re-scans the live records (`INVALID_OPERATION`, `data { field_id, violating_records, total }`; `unique`: `UNIQUE_VIOLATION`); restoring a required field over records created meanwhile is `SCHEMA_CONFLICT` |
| table-source | `plan_table.rs` submitter (`insert_records`, `set_values`, `unset_values`, `delete_records`, `restore_records`, `add_field`, `update_field`, `restore_field`; `init_entity`) | primitive | at most one `submitter` field, `text` only; the service fills it, clients never write it (undo, import, replay keep it); one live record per submitter (`SUBMITTER_EXISTS`, `UNIQUE_VIOLATION` on internal paths); without `update` / `delete` an `append` holder writes only its own records; set or cleared only on an empty table (`SCHEMA_CONFLICT`) |
| table-source | `FieldDef::normalize` (`max_items`) | primitive | a `multi_select` value holds at most `max_items` options (`INVALID_SCHEMA`) |
| table-source | `plan_table.rs` `check_body` | primitive | a record body is a live rich text under this table and the body of no other record |
| table-source | `plan_table.rs` `delete_field` / `migrate_field` | primitive | the title field cannot be deleted and stays a text field |
| table-source (standard) | `standard::check_table` (from `validate_schema`) | schema | `schema_id` `buckyos.todo@1` / `buckyos.calendar@1`: created with every standard field in its type; `title_field_id` is `title` |
| table-source (standard) | `plan_table.rs` `delete_field` / `migrate_field` → `standard::check_field_change` | schema | a standard field is never deleted and keeps its type |
| table-source (standard) | `standard::check_records` (end of `plan_commit`; records noted by `insert_records`, `set_values`, `unset_values`, `restore_records`) | schema | per record, on the Commit's final values: required fields; a timed event has `start` (and no dates), an all-day one `start_date` (and no times); `end` / `end_date` not before the start; `order` an order key; the `ext` keys with a meaning (RRULE subset, `exdate`, `recurrence_id` never with `rrule`, `tz`, `reminders`, …) |
| richtext | `plan_richtext::create` / `apply_update` | primitive | document conforms to `buckyos.richtext.basic.v1`, block ids unique, CRDT update applies to a known base |
| richtext | `entity_refs` (`richtext_ref_edge`) | reference | `object_embed` / `object_link` targets are indexed as references; existence judged by the planner's `check_ref_target` |
| asset | `validate_asset` | primitive | `object_id` is a `type:hex` object id; `media_type` / `size` come from the verified content, never the client; `image { width, height }`; `poster` (an uploaded PNG / JPEG / WebP / GIF asset picturing the content: a video frame, a PDF's first page, a cover), `duration_ms`, `page_count` (positive integers); a new `object_id` without new values drops `image`, `poster`, `duration_ms`, `page_count` (`ASSET_CONTENT_KEYS`; doc/workspace/BuckyOS AI Workspace 多媒体对象.md) |
| asset | `validate_asset` (object lookup) | reference | the object was uploaded and verified (`DEPENDENCY_UNAVAILABLE`); import tolerates a listed but missing object |
| container | `container_primitive` (keys, kind, layout, system nodes) | primitive | tree node kinds `folder` / `surface` / `group` and the control kinds (`mindmap`, `composite`, `todo`, `calendar`); `root` / `data` / `surfaces` only internal; kind never changes; `layout.mode` (implied by a control's kind; `def` for a composite, `data` for a data control); `system` only on a folder, `surface_id` only on a folder; `def_ref` / `on` only on a composite, `bindings` on a composite or a data control |
| container | `container_primitive` (`content_folder_id`) | reference | a Surface's content folder exists, is a folder and sits under `canvas-content` |
| container (composite) | `composite_refs` | reference | `def_ref` is required, local, names a live `kind: composite` definition when created and never changes; `bindings` targets are data entities |
| container | `check_slot` (`slot` of a group or a control) | reference | only inside a control; inside a composite required and a slot of the definition; elsewhere unique among siblings |
| container | `container_schema` (`title`, `icon`, `config`, `locked`) | schema | preset icon id only on a Surface; `config` ≤ 64 KiB only on a Surface or a control, a mind map's values checked (`check_control_config`), a composite's left to its `config_schema` in the clients; `locked` only on a group or a control |
| record | `record_schema` / `validate_record` (values) | schema | `schema.properties` declare the keys; every `p:` value normalizes under its property type; required properties present |
| record | `validate_record` (`object_ref` values) | reference | an `object_ref` property points at an existing entity of the declared target types |
| cell | `validate_cell` (`view`, keys) | schema | known keys, `view.type` is a renderer id, `view.version`, title ≤ 256, `options` / `config` objects, config ≤ 64 KiB, `bindings` ≤ 32 identifiers |
| cell | `validate_cell` (`source_ref`, `bindings`, `def_ref`) | reference | the bound data source exists and has the view's data type; bindings point at data types; `def_ref` is a block definition (judged when written; dangling configuration stays editable) |
| cell | `check_slot` (from `validate_primitive`, `check_position`) | reference | `slot` only inside a control; inside a composite required and a slot of the definition (several Blocks may share one); elsewhere unique among the live siblings (`SLOT_CONFLICT`); re-judged by `tree.move`; a connector never turns into a box or back |
| cell, container (composite) | `check_on_shape` (from `validate_primitive` / `container_primitive`) | primitive | `on` (扩展控件 P2): ≤ 16 event names, each one handler or ≤ 4, each an object whose `kind` is write / wish / open / notify, ≤ 16 KiB; never on a connector, ink, or a container other than a composite |
| cell, container | `entity.set_props` (`plan.rs`) | primitive | `key` config / bindings / on, ≤ 64 property names `^[A-Za-z_][A-Za-z0-9_-]{0,63}$`, values ≤ 64 KiB; the key then goes through the entity's validators like a `set_keys` of it |
| cell | `validate_cell` (table view configuration) | schema | `fields` / `filter` / `sorts` / `group` / `manual_order` only on table views, named fields live, ≤ 4 sorts, group field type |
| cell | `validate_cell` (`config.snapshot`) | primitive | an embedded snapshot is an AssetRef (checked by `validate_asset`) |
| cell (connector) | `validate_connector` (shapes) | schema | `flip`, `route`, ≤ 64 `controls`, `label.t` in [0, 1]; `start` / `end` are `{ entity_id, anchor { kind: named, id } }` |
| cell (connector) | `validate_connector` (endpoints) | reference | a bound endpoint is a readable Block or group on the connector's Surface, never itself or another line |
| cell (connector) | `validate_connector` (Surface rule) | schema | created and bound only on a free-layout Surface; a self loop needs two anchors and a non-straight route |
| cell (ink) | `validate_ink` | schema | version 1: `format`, logical `stroke` id, `seg` < `segs` ≤ 64, `#rrggbb`, width (0, 256], `frame`, equal-length point channels inside the frame, whole non-decreasing `t`, `p` in [0, 1], tilt in [-90, 90]; no data bindings |
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
| block-def | `block_def_primitive` (`source_dir`, `deps`, `events`) | primitive, reference | `events` an object of ≤ 16 event names (each an object, ≤ 16 KiB), written by build / import / internal / replay Commits only; `deps` is `[{ ident, entity_id }]` (≤ 32, no repeats) and only build / import / internal / replay Commits change it; `source_dir` a live directory, each `deps` target a live definition, no cycle through `deps` (`REFERENCE_CYCLE`); judged when written |
| block-def | `block_def_schema` | schema | known keys, `def_id` renderer id, `ident`, `version`, `manifest_version` ≤ 3, `kind` declarative / html / composite with the matching body (`composition`: an object ≤ 64 KiB, ≤ 16 slots named `[a-z][a-z0-9_]{0,31}`, `events` shaped like a definition's), `accepts` known data types, `default_size`, bindings, catalog, agent, provenance, ≤ 512 KiB |
| any (derived record) | `check_derived` | schema | the dependency record `{ wish_id, run_id, executor, inputs[], … }` of a generated result |
| any (keyed object) | `check_keys` / `opt_text` / `check_locked` | schema | helper checks used by the validators above: known keys, bounded strings, `locked` boolean |
| any (references) | `entity_refs`, `plan.rs` `check_ref_target` | reference | every indexed reference target is a live, readable entity of the admitted types (`REFERENCE_BROKEN`); a composite's `def_ref` is a `def` edge and its `bindings` `bind` edges, a definition's `deps` are `def_dep` edges (strong) |

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
| `entity_prop` `{ key, name }` | one property of an object key (`config`, `bindings`, `on`) of a Cell or container (扩展控件 P2, Q28) | the property's own rev (`p:<key>.<name>`; 0 when never written) |

**Workspace names (控件机制 TODO G9).** A Workspace may have a `name`, `^[a-z0-9][a-z0-9._-]{0,63}$`, given to `ws.create` or
`ws.ensure` and never changed (no method changes it); the title stays independent and editable. It is unique among the
Workspaces of one owner (`local_meta` `owner` and `name` of `local.sqlite`, never exported). `ws.ensure { name, title? }` returns
the caller's Workspace of that name or creates it; concurrent calls create one (the service serializes the lookup and the
creation over an `(owner, name)` index read from the folders on first use). `ws.delete` frees the name. `ws.clone`, `ws.import`
`new` and a restore that creates a missing Workspace carry no name; a restore over an existing Workspace keeps its name; a show
clone has none.

**Workspace clone (`ws.clone`, `notepads/aiws-workspace-clone-todo.md`).** A materialized copy, not copy-on-write
(`store/src/service.rs` `clone_workspace`): what the caller may read is materialized as a `clone_source` snapshot and loaded
as a new Workspace (the path of `ws.import new`): a new id and epoch, the caller as owner, one import Commit, rich texts
as new lineages. Immutable objects stay shared in the named store; the logical data (`doc.sqlite`) is a copy of its own
and its full content and every asset count on the new Workspace's `usage` (kernel doc 11: what a Workspace is charged does
not depend on deduplication below it). Entities the caller cannot read are left out and counted in `excluded_entities`.
`cloned_from { workspace_id, snapshot }` (kept in `workspace_meta`, shown by `ws.get_info`, written into every snapshot
root as `lineage.cloned_from`, so its checkpoints retain the source snapshot) is provenance only: no sync, no merge, and
deleting the source does not touch the copy. Nothing of `local.sqlite` comes along — grants, entries, publications, runs,
uploads, hosted execution, the name — and neither do personal entities. `ws.import new` records the package's snapshot
the same way; a restore keeps what its package recorded. The caller must be able to own Workspaces (above) and have
workspace-level `export` on the source. Not to be confused with `tree.clone` / `entity.clone` (a Commit copying a
subtree inside one Workspace), a show clone (`clone_for_show`, the frozen baseline of a live show) or a session's run
Workspace (`clone_for_session`).

**Method levels (D11).** `stable` methods are the contract other Apps, Agents and SDKs may depend on: they change
only with `protocol_version`, and each has a contract test against a recorded request / response shape
(`server/tests/contract.rs`, `fixtures/protocol/stable-methods.json`; `AIWS_REGEN=1` re-records). `internal` methods serve the
Desktop and diagnostics and change with it. `experimental` methods are not frozen yet (`proc.*` became stable with the
browser-first execution and the three wish stages). The list is `server/src/methods.rs` `METHODS`; this table mirrors its levels, and the authorization table below is generated from it.

| Method | Level | Params (besides `workspace_id`) |
| --- | --- | --- |
| `ws.create` | stable | `title`, `workspace_id?`, `name?` (see “Workspace names”; a name the caller already uses is `INVALID_OPERATION` / `NAME_CONFLICT` with `data.workspace_id`) → `workspace_id`, `epoch`, `head_seq`, `title`, `name?` |
| `ws.ensure` | stable | `name`, `title?` (default: the name), no `workspace_id` → `workspace_id`, `name`, `title`, `epoch`, `head_seq`, `created` — the caller's Workspace of that name, created when it has none (callers as for `ws.create`) |
| `ws.time` | stable | none → `server_ms` (unix ms of the service clock); any session, guests included |
| `ws.list`, `ws.get_info`, `ws.delete` | stable | (`ws.list` items and `ws.get_info` carry `name` when the Workspace has one; `ws.get_info.cloned_from` names the source of a clone or a `new` import. `ws.list` items also carry what the home page's cards show (UI 框架 PRD §9.2, K6): `owner_principal`, `updated_at` (the last accepted Commit, else the creation), `opened_at` (when the caller last opened it, or null), `public` (`anonymous` — anonymous access on and something granted to `@guest` — or `published` by path), `live_sessions` (live sessions of it as courseware) and `cover` (null: not generated yet). `ws.get_info` carries `settings` (the Workspace's shared settings, the root's `config`, see “Workspace settings”) and `settings_rev`; `record_open: true` records that the caller opened it now, for `opened_at`) |
| `ws.clone` | stable | none → `workspace_id`, `epoch`, `head_seq`, `content_root`, `cloned_from { workspace_id, snapshot }`, `excluded_entities` — a new Workspace the caller owns, copied from what the caller may read (workspace-level `export`; see “Workspace clone”) |
| `acl.set` | stable | `changes: [{ subject, resource?, allow?, deny? }]` (≤ 500): each replaces the subject's entry on the resource (path or id, default `/`; both empty removes it), all or none, one record; `manage` on each resource, the owner's when `manage` is touched |
| `acl.query` | stable | `by: entity` (`resource`, `expand?`: every subject with entries on its chain, their effective capabilities, each entry's source — direct, inherited, workspace — and what a denial overrides; macros bound to those groups and who may change them; managers), `by: subject` (`subject?`: its entries; for a group, its members; for a single caller, its judgment — groups and how, `@guest`, judged as), `by: explain` (`resource`, `principal?`: each capability allowed or refused and by which entry), `by: all` (every entry with who wrote it, `complete: true`, for a manager; otherwise the entries that apply to the caller — its own, its groups', `@all`, `@guest` while it is a guest — without who wrote them). Anyone may ask about itself; the rest is the managers' |
| `acl.groups` | stable | every group with its members (`via_entry`) for a manager; the caller's own groups otherwise |
| `acl.set_group` / `acl.delete_group` | stable | `group_id` (`[a-z0-9][a-z0-9_-]{0,63}`, never `owner`, `all`, `guest`, `creator`, `pub`), `title?`, `description?`; deleting takes the group's entries, members and macro bindings along and closes its entries |
| `acl.members` | stable | `group_id`, `add?`, `remove?`: principals, `group:<id>`, `contact:<your did>#<tag>`, `*` |
| `acl.set_anonymous` | stable | `enabled`: whether `@guest` exists here |
| `acl.bind_macro` | stable | `wish` (path or id), `group_id` (null unbinds), `mode: union \| intersection` (§4.5); whoever may change the wish holds the group's permissions through it, so the group never holds `manage` |
| `acl.audit` | stable | `after?`, `limit?`: the operation record (`manage`) |
| `ws.list_subjects` | stable | principals an entry or a group can name (static identities / zone users and `group:` member specs) |
| `ws.create_entry` | stable | `group_id`, `methods`, `expires_at?`, `max_joins?` → `entry` (`manage`) |
| `ws.list_entries` | stable | → `entries`, each with how many `joined` (`manage`) |
| `ws.close_entry` | stable | `entry_id` (`workspace_id` optional, must match), `remove_members?` → `entry`, `removed_members` |
| `ws.join_entry` | stable | `entry_id` only (a session or a guest token) → `workspace_id`, `entry_id`, `group_id`, `joined`, `path` |
| `entry.describe` | stable | `entry_id` only, no session → `EntryDescription` |
| `ws.get_user_state` / `ws.set_user_state` | stable | `entries: { key: value \| null }` — user work state per subject and Workspace in `local.sqlite`; never a Commit |
| `user.get_prefs` / `user.set_prefs` | experimental | no `workspace_id`. The caller's own preferences across Workspaces (UI 框架 PRD §4.6, K4), one JSON object kept by the service (`users.sqlite`), never part of a Workspace, never exported, cloned or carried by a template: `get` → `prefs`, `revision`, `updated_at`; `set` takes `prefs` (the whole object) or `merge: { key: value \| null }` (top-level keys; null removes), and `expect?` (the revision read: `REVISION_CONFLICT` with `data.current` when it moved) → `prefs`, `revision`, `updated_at`. ≤ 500 keys of ≤ 128 bytes, ≤ 256 KiB. A guest or a link's visitor keeps none (`PERMISSION_DENIED`) |
| `ws.import` | stable | `bucket_id` + `key` (the package the caller bound into its own `user/aiworkspace/<user>/exports`; a successful import consumes it) or `obj_id` (an unbound FileObject the caller may read, judged at the NDM gateway), `semantics: "restore" \| "new"`, `replace?` |
| `ws.release_retention` | stable | maintenance (`manage`, D19): `kind`, `ref` — unbind what holds a `snapshot`, an `export` package or an unreferenced `staged` upload → `released` |
| `ws.delete_checkpoint` | stable | maintenance (`manage`): `snapshot_id` — the snapshot row goes and its retention set is unbound; the named store reclaims what no binding holds |
| `ws.purge_deleted` | stable | maintenance (`manage`): `entity_ids?` (default: every deleted entity) — content and references are given up for good (`entity.purge`), resources are reclaimable at once, undo and version restore answer `RESOURCE_RECLAIMED`, Block state is cleared |
| `ws.clear_staged` | stable | maintenance (`manage`): staged uploads nobody referenced go, expired or not |
| `doc.outline` | internal | — all readable entity envelopes in tree order |
| `doc.resolve` | stable | `reference` or `path` (the one path syntax, see “Paths and names”) → the reference, its `canonical` form, `path` |
| `doc.read` | stable | `entity_id` or `path`; `as?: "content" \| "meta" \| "body"` (default `content`, the native content); `selector?`; `version?` (`{ epoch, seq }` / `{ object_id }`: a fixed version of the native content or `VERSION_NOT_RETAINED` / `REFERENCE_BROKEN`, never the head; `meta` / `body` of a fixed version are `UNSUPPORTED_VERSION`); `body` takes `cursor?`, `max_bytes?` (64…262 144, default 32 768), `parts?` — or `targets: [{ entity_id \| path, as?, selector?, … }]`. See “Common meta and body” |
| `doc.list` | stable | `at?` (path or `entity_id`, default `/`), `type?`, `not_type?`, `schema?`, `limit?` (≤ 1000, default 100), `cursor?` → `at`, `items: [{ entity_id, name, path, type, type_id, kind?, title, children, content_rev, body: { state, format }, shadowed? }]`, `next_cursor?`, `head_seq` — the readable direct children |
| `doc.find` | stable | `at?`, `depth?` (≤ 16), `name?` (a literal, case-insensitive, in the name or title), `type?`, `not_type?`, `schema?`, `limit?` (≤ 200, default 50), `cursor?` → `items` (with `match: name \| title`), `complete`, `visited`, `next_cursor?` — a pre-order walk of at most 20 000 entities per call |
| `doc.list_annotations` | experimental | `target_ids?`, `parent_id?` — annotations anchored to those entities and/or placed under that parent, each with its resolved `anchor` |
| `doc.query` | stable | exactly one of `text` (a text query: `text`, `case?`, `at?`, `depth?`, `type?`, `not_type?`, `limit?` (≤ 100), `cursor?` → `hits: [{ entity_id, path, type, on: name \| title \| body, selector?, snippet, more? }]`, `coverage: { complete, scanned, bytes, skipped }`, `next_cursor?`) or `sql` (one SELECT over the logical tables: `sql`, `params?`, `tables?`, `max_rows?` (≤ 5000), `expect_head?: { epoch, seq }` → `columns`, `rows`, `truncated`, `read: [{ table, entity_id, path, content_rev }]`). The old table query (`view_id`, `source_id`, `filter`, …) is refused. See “Text and SQL queries” |
| `doc.index` | internal | `op: ls \| find \| grep` and the parameters of `doc.list` / `doc.find` / `doc.query { text }` — the index of the Workspace object (`obj://aiworkspace/<ws>?op=…`) |
| `doc.source_query` | internal | `source_id` of a URL table (`data_mode: url_query`); `filter?`, `sorts?`, `fields?`, `limit`, `cursor?`, `consistency?`, `source_revision?` — its rows from the remote source (what `doc.query` did for URL tables) |
| `doc.freshness` | experimental | `entity_ids[]` — dependency-based freshness of results and wishes (computed in core) |
| `doc.relations` | experimental | `entity_id` — outgoing / incoming references, Blocks showing it, produced results; unreadable referrers only as `hidden_incoming` |
| `doc.list_versions` | stable | `entity_id` → `entity_id`, `content_rev`, `head_seq`, `versions: [{ content_rev, object_id, kind: generated \| checkpoint, created_at, derived, commit_id, author, origin, run_id, message, accepted_at }]` newest first — addressable versions (generated results, checkpoints) with their commits; versions whose content was reclaimed are left out |
| `doc.restore_version` | experimental | `entity_id`, `content_rev` — the operations restoring that version (submitted as an ordinary Commit) |
| `doc.source_capabilities` | internal | `source_id` (URL query tables) |
| `doc.get_collab_state` | internal | `entity_id` |
| `doc.prepare` / `doc.commit` | stable | the Commit request (design §2.6) |
| `doc.stage` | stable | one part of a Commit too large for one request: `{ epoch, idempotency_key, part, operations }` → `{ part, digest, operations, expires_at }`; the final `doc.commit` / `doc.prepare` names the digests in `staged` (design §2.6) |
| `doc.get_submission` | stable | `epoch`, `idempotency_key` |
| `doc.undo` | stable | `epoch`, `commit_id`, `idempotency_key`, `mode?`, `plan_digest?`, `session_id?` |
| `doc.get_changes` | stable | `epoch`, `after_seq`, `limit?`, `filter: { detail: "touched" }?` — each change carries `touched`, and (unless `touched` only) `ops` and the row delta `rows` (D20); `after_seq` below `base_seq` is `BASE_TOO_OLD` |
| `doc.wait_changes` | stable | `epoch?`, `after_seq`, `timeout_ms?` (long poll) → `epoch`, `head_seq`, `timed_out`, `show_lock`, `server_ms` (unix ms when answering: clients calibrate against half the round trip, 控件机制 TODO G16) |
| `doc.checkpoint`, `doc.export` | experimental | `mode: "share" \| "personal_backup"`, `self_contained`, `include_notes?`; `doc.export` → `export_id`, `manifest`, `download: { bucket_id, key, version, obj_id, token }` (the package in the owner's `exports` bucket, 7 days; `GET /bucket/v1/<bucket_id>/<key>?token=`) |
| `asset.import_object` | stable | `workspace_id` and either `bucket_id` + `key` (the uploader's own binding in `user/aiworkspace/<user>/assets`) or `obj_id` (an unbound, ready NDM FileObject the caller may read) → `object_id`, `media_type`, `size`, `expires_at`, `bucket_id`, `key`, `version` (staged under `<workspace_id>/<object_id>` in the owner's `assets` bucket) |
| `replica.bootstrap` | internal | → `replica_id`, `epoch`, `head_seq`, `principal`, `prepared_at`, `download` (as `doc.export`) |
| `proc.start` / `proc.get` / `proc.apply` / `proc.cancel` | stable | `program`, `params`, `idempotency_key`; `run_id`, `session_id?` — wish programs below |
| `proc.list` | stable | `wish_id?`, `limit?` — wish runs, newest first |
| `proc.claim` | stable | `run_id`, `claim_token`, `executor?` — an executor takes the run (M3): the next attempt, a `run_token` (bearer for `proc.heartbeat` / `proc.report` / reads of this Workspace), a lease (90 s), and the stage it works on: `{ kind: model, stage, system, user, tools, model, iterations, source?, budget }` (analyze, code) or `{ kind: program, stage: run, source, manifest, budget }` |
| `proc.heartbeat` | stable | `run_id`, `attempt`, `progress?` — renews the lease; a lapsed lease makes the run `interrupted` (D15) |
| `proc.report` | stable | `run_id`, `attempt`, then `progress?`, or `error` (`{ code, detail }`: `EXECUTOR_FAILED`, an `AI_*` refusal, …), or `finished: true` (what the stage collected becomes the candidate), or `analysis` (an analyze run whose analysis was made elsewhere). Results are never reported directly (they come from the macro). The service validates and plans; a superseded attempt or a cancelled run gets `RUN_REVOKED` |
| `proc.tool` | stable | `run_id`, `attempt`, `name`, `args` — one call of the run's executor: a stage tool (`ws_*`, `submit_analysis`, `check_results`, `finish`), `sql` (a macro's `aiws.sql`) or `llm_cache` (the AI answer cache) — see “Wish runs”. Answers `{ status: ok \| error, text?, code? }` — `error` is for the model to correct |
| `proc.program` | stable | `run_id`, `attempt`, `source`, `results?` (what the program host wrote to `output/.aiws/results.json`), `log?`, `files?` (`{ "output/<path>": text \| { base64 } }`) — the executor ran the macro: a trial run while coding (the model reads the answer), the run itself when running (a failure ends it: `PROGRAM_FAILED`, `MACRO_CODE_CHANGED`) |
| `proc.files` | stable | `run_id`, `attempt` — the stage directory (`context/`, `request.json`, `catalog/`, `lib/`, `program/main.js`) as `{ path: { text } \| { base64 } }` plus `stage: { budget, manifest }`; over `snapshot_download_max_bytes` the run fails with `SNAPSHOT_TOO_LARGE` |
| `ws.set_hosted_exec` | experimental | `config: { enabled, agent_id, allowed_wishes?, triggerers?, limits? }` (`manage`, D22) — the Owner's hosted execution; `ws.get_info.hosted_exec` returns it with its `digest` |
| `ws.publish` | experimental | `entity_id`, `name?` (`manage`, D26) — the `public` subject may read the entity; the name (default: the entity's, slugified; a new name keeps the old as an alias) is the last segment of the link. Returns `name`, `path` (`/<workspace_name>/<name>`), `url` (on `https://aiws-<workspace_id>.<zonehost>` when the deployment knows its host) and the reference object |
| `ws.unpublish` | experimental | `entity_id` (`manage`) — every name and the public grant go; referrers keep their records and read `unavailable` |
| `ws.list_publications` | experimental | the names, entities, `url` and referrer counts (what the caller may read) |
| `ws.list_referrers` | experimental | `entity_id` (read) — who registered as reading it (`{ zone?, workspace_id, entity_id?, principal? }`, weak: never blocks an unpublish) |
| `control.list`, `control.describe` | experimental | `kind` (describe) — the Block-control catalogue (`aiws/controls.json`: kind, summary, when, formats; describe adds the `agent` self-description) |
| `control.import` | experimental | `parent_id`, `kind?` (`mindmap`), `format?` (`markdown` \| `opml`), `text`, `at? { x, y }`, `title?`, `config?`, `idempotency_key?` — a new control holding the outline, one Commit as the caller (its answer plus `container_id`, `nodes`) |
| `control.export` | experimental | `entity_id`, `format?` — the control's Blocks as a Markdown outline or OPML (`text`), all expanded |
| `ext.build`, `ext.export`, `ext.install`, `ext.list_library`, `ext.uninstall`, `ext.library_bundle`, `ext.import`, `ext.pack`, `ext.upgrade`, `ext.export_template`, `ext.create_from_template` | experimental | see “Extension controls” and “Extension controls P2 / P3” |
| `lock.acquire` / `lock.renew` / `lock.release` / `lock.break` / `lock.list` | stable | `entity_ids[]`, `session_id`; `lock_ids[]`; `entity_id` |
| `show.start` / `show.heartbeat` / `show.publish` / `show.command` / `show.watch` / `show.notes` / `show.end` | internal | see “Non-public shows” |
| `presence.heartbeat` / `presence.leave` / `presence.list` | internal | `conn_id` (heartbeat, leave), `surface_id?`, `name?` (a guest's) — see “Presence and the work log” |
| `ws.work_log` | experimental | `before?`, `limit?` (≤ 200, default 50), `filter?: all \| mine \| automation` → `entries`, `next_before`, `more` — see “Presence and the work log” |
| `session.create`, `session.list`, `session.list_mine`, `session.list_open`, `session.get`, `session.join`, `session.start`, `session.end`, `session.save`, `session.delete`, `session.set_access` | experimental | see “Presentation sessions” |
| `session.invite`, `session.list_invites`, `session.revoke_invite`, `session.takeover`, `session.command`, `session.sample`, `session.notes`, `session.rtc` | experimental | see “Presentation sessions” |
| `session.task`, `session.claim`, `session.respond`, `session.review`, `session.publish`, `session.react`, `session.feed_sample` | experimental | see “Presentation sessions” |
| `session.events`, `session.watch`, `session.replay`, `session.post`, `session.list_posts`, `session.delete_post` | experimental | see “Presentation sessions” |
| `asset.stream_url` | stable | `object_id` (read access as for the asset route; an uploader its own staged upload) → `url` (`/kapi/aiworkspace/stream/<token>`, 6 h), `media_type`, `size`, `expires_at` |
| `diag.list_unretained`, `diag.verify_refs` | internal | |

How each method is authorized (权限需求 R21): dispatch enforces the generic checks (who may own, the owner, membership,
a capability on `/` or on a named entity) before the handler, which keeps its finer ones.

<!-- authorization-table:begin (generated from server/src/methods.rs; AIWS_REGEN=1) -->
| Method | Level | Authorization |
| --- | --- | --- |
| `ws.create` | stable | may own (unrestricted account or service) |
| `ws.ensure` | stable | may own (unrestricted account or service) |
| `ws.time` | stable | a session |
| `ws.list` | stable | a session |
| `ws.get_info` | stable | may enter |
| `ws.delete` | stable | the owner |
| `ws.clone` | stable | may own (unrestricted account or service); `export` on `/` |
| `acl.set` | stable | `manage` on each resource; the owner when `manage` is touched |
| `acl.query` | stable | may enter; anyone about itself, `manage` about others |
| `acl.groups` | stable | may enter |
| `acl.set_group` | stable | `manage` on `/` |
| `acl.delete_group` | stable | `manage` on `/` |
| `acl.members` | stable | `manage` on `/` |
| `acl.set_anonymous` | stable | `manage` on `/` |
| `acl.bind_macro` | stable | `manage` on `/` |
| `acl.audit` | stable | `manage` on `/` |
| `ws.list_subjects` | stable | may own (unrestricted account or service) |
| `ws.create_entry` | stable | `manage` on `/` |
| `ws.list_entries` | stable | `manage` on `/` |
| `ws.close_entry` | stable | `manage` on `/` |
| `ws.join_entry` | stable | a person presenting an active entry the Zone switches admit |
| `entry.describe` | stable | none (no session) |
| `ws.get_user_state` | stable | may enter |
| `ws.set_user_state` | stable | may enter |
| `ws.import` | stable | may own (unrestricted account or service); a restore over a Workspace: its owner |
| `ws.release_retention` | stable | `manage` on `/` |
| `ws.delete_checkpoint` | stable | `manage` on `/` |
| `ws.purge_deleted` | stable | `manage` on `/` |
| `ws.clear_staged` | stable | `manage` on `/` |
| `doc.resolve` | stable | may enter; `read` on each entity |
| `doc.read` | stable | may enter; `read` on each entity |
| `doc.list` | stable | may enter; `read` on each entity |
| `doc.find` | stable | may enter; `read` on each entity |
| `doc.query` | stable | may enter; `read` on each entity |
| `doc.prepare` | stable | each operation's capability on its target (`schemas/operations.json`) |
| `doc.commit` | stable | each operation's capability on its target (`schemas/operations.json`) |
| `doc.stage` | stable | may enter |
| `doc.get_submission` | stable | may enter |
| `doc.undo` | stable | each operation's capability on its target (`schemas/operations.json`) |
| `doc.get_changes` | stable | may enter; `read` on each entity |
| `doc.wait_changes` | stable | may enter |
| `asset.import_object` | stable | a write right anywhere (the Commit that references it is judged where it writes) |
| `lock.acquire` | stable | a write right on each entity |
| `lock.renew` | stable | may enter |
| `lock.release` | stable | may enter |
| `lock.break` | stable | `manage` on `entity_id` |
| `lock.list` | stable | may enter; `read` on each entity |
| `doc.outline` | internal | may enter; `read` on each entity |
| `doc.source_capabilities` | internal | may enter; `read` on each entity |
| `doc.source_query` | internal | may enter; `read` on each entity |
| `doc.index` | internal | may enter; `read` on each entity |
| `doc.get_collab_state` | internal | `read` on `entity_id` |
| `replica.bootstrap` | internal | may enter; `read` on each entity |
| `show.start` | internal | `read` on `/`; a Workspace-level write right locks it; anyone else presents read-only |
| `show.heartbeat` | internal | the presenter's read; a prompter token |
| `show.publish` | internal | the presenter's read; a prompter token |
| `show.command` | internal | the presenter's read; a prompter token |
| `show.watch` | internal | the presenter's read; a prompter token |
| `show.notes` | internal | the presenter's read; a prompter token |
| `show.end` | internal | the presenter, or `manage` |
| `presence.heartbeat` | internal | may enter |
| `presence.leave` | internal | a session |
| `presence.list` | internal | may enter |
| `ws.work_log` | experimental | may enter; `read` on each entity; runs as `proc.get` shows them, sessions as `session.list`, the permission record to managers |
| `diag.list_unretained` | internal | `manage` on `/` |
| `diag.verify_refs` | internal | `manage` on `/` |
| `doc.list_annotations` | experimental | may enter; `read` on each entity |
| `doc.freshness` | experimental | may enter; `read` on each entity |
| `doc.relations` | experimental | `read` on `entity_id` |
| `doc.list_versions` | stable | `read` on `entity_id` |
| `doc.restore_version` | experimental | `read` on `entity_id` |
| `doc.checkpoint` | experimental | `export` on `/` |
| `doc.export` | experimental | `export` on `/` |
| `proc.start` | stable | a Source: `update` on it; a wish: `update` for analyze / code, `run` or a write right for run, on the wish |
| `proc.get` | stable | one's own run, or `update` / `manage` on its wish |
| `proc.apply` | stable | one's own run, or `update` / `manage` on its wish; written as the macro identity ∩ the declared outputs |
| `proc.cancel` | stable | one's own run, or `update` / `manage` on its wish |
| `proc.list` | stable | may enter |
| `proc.claim` | stable | the executor the run's context names, with its claim token |
| `proc.heartbeat` | stable | the run's token, while its attempt is current and running |
| `proc.report` | stable | the run's token, while its attempt is current and running |
| `proc.tool` | stable | the run's token, while its attempt is current and running |
| `proc.program` | stable | the run's token, while its attempt is current and running |
| `proc.files` | stable | the run's token, while its attempt is current and running |
| `user.get_prefs` | experimental | a person's session (not a guest or a link): its own preferences only |
| `user.set_prefs` | experimental | a person's session (not a guest or a link): its own preferences only |
| `ws.set_hosted_exec` | experimental | the owner |
| `ws.publish` | experimental | `manage` on `entity_id` |
| `ws.unpublish` | experimental | `manage` on the entity (on `/` once it is deleted) |
| `ws.list_publications` | experimental | may enter; `read` on each entity |
| `ws.list_referrers` | experimental | `read` on `entity_id` |
| `control.list` | experimental | a session |
| `control.describe` | experimental | a session |
| `control.import` | experimental | each operation's capability on its target (`schemas/operations.json`) |
| `control.export` | experimental | may enter; `read` on each entity |
| `ext.build` | experimental | each operation's capability on its target (`schemas/operations.json`) |
| `ext.export` | experimental | may enter; `read` on each entity |
| `ext.install` | experimental | into the caller's own library (the control panel installs into anyone's) |
| `ext.list_library` | experimental | may own (unrestricted account or service) |
| `ext.uninstall` | experimental | may own (unrestricted account or service) |
| `ext.library_bundle` | experimental | may own (unrestricted account or service) |
| `ext.import` | experimental | may own (unrestricted account or service); each operation's capability on its target (`schemas/operations.json`) |
| `ext.pack` | experimental | may own (unrestricted account or service); may enter; `read` on each entity |
| `ext.upgrade` | experimental | may own (unrestricted account or service); each operation's capability on its target (`schemas/operations.json`) |
| `ext.export_template` | experimental | may enter; `export` on each Surface; the permission design with `manage` |
| `ext.create_from_template` | experimental | may own (unrestricted account or service) |
| `asset.stream_url` | stable | the session's roles and links |
| `session.create` | experimental | the session's roles and links |
| `session.list` | experimental | the session's roles and links |
| `session.list_mine` | experimental | the session's roles and links |
| `session.list_open` | experimental | a session; each listed session is one the caller may open without a link |
| `session.get` | experimental | the session's roles and links |
| `session.join` | experimental | the session's roles and links |
| `session.start` | experimental | the session's roles and links |
| `session.end` | experimental | the session's roles and links |
| `session.save` | experimental | the session's roles and links |
| `session.delete` | experimental | the session's roles and links |
| `session.set_access` | experimental | the session's roles and links |
| `session.invite` | experimental | the session's roles and links |
| `session.list_invites` | experimental | the session's roles and links |
| `session.revoke_invite` | experimental | the session's roles and links |
| `session.takeover` | experimental | the session's roles and links |
| `session.command` | experimental | the session's roles and links |
| `session.sample` | experimental | the session's roles and links |
| `session.task` | experimental | the session's roles and links |
| `session.claim` | experimental | the session's roles and links |
| `session.respond` | experimental | the session's roles and links |
| `session.review` | experimental | the session's roles and links |
| `session.publish` | experimental | the session's roles and links |
| `session.react` | experimental | the session's roles and links |
| `session.events` | experimental | the session's roles and links |
| `session.watch` | experimental | the session's roles and links |
| `session.replay` | experimental | the session's roles and links |
| `session.notes` | experimental | the session's roles and links |
| `session.feed_sample` | experimental | the session's roles and links |
| `session.post` | experimental | the session's roles and links |
| `session.list_posts` | experimental | the session's roles and links |
| `session.delete_post` | experimental | the session's roles and links |
| `session.rtc` | experimental | the session's roles and links |
<!-- authorization-table:end -->

**Commit size.** Two limits. One *request* (`doc.commit`, `doc.stage`, any kRPC call) is at most `Limits.max_commit_bytes` (8 MiB, the HTTP body limit). One *Commit* — the staged parts plus the final request, and a compensating Commit of `doc.undo` — is at most `max_commit_total_bytes` (32 MiB) and `max_ops` (50 000 operations, `aiworkspace-core` `Limits`, shared with the browser replica). A client sends a Commit that does not fit one request as `doc.stage { part: 0..N-2 }` + `doc.commit { staged: [digests], operations: <last part> }`; the service assembles it and runs the ordinary pipeline: one transaction, one `commit_id`, one change event, one undo unit. Parts live one hour under `(principal, idempotency_key, part)`, count as `staged` usage and are consumed by the final `doc.commit` whatever its outcome (`doc.prepare` does not consume); missing or expired parts answer `SNAPSHOT_EXPIRED` / `STAGED_PARTS_MISSING` (stage again), a part whose digest is not the one named `IDEMPOTENCY_MISMATCH`, an assembled Commit over the limits `LIMIT_EXCEEDED`. The websdk's `commitWithParts` (used by `AiWorkspaceClient.commit` and the Desktop client) does the cutting (6 MiB parts): callers make one `commit` call per Commit. Importers (PDF, Office, draw.io, tables) write one Commit per import and refuse an import over the limits before sending anything.

HTTP routes (same port, `Authorization: Bearer <session token>`):

| Route | Use |
| --- | --- |
| `GET /kapi/aiworkspace/asset/<workspace_id>/<object_id>` | asset bytes from the named store, authorized through an entity that references the asset — an AssetRef's poster too (collaborators, guests and run tokens have no rights on the owner's bucket); served with the sniffed type for pictures, PDF, plain text, audio and video, as `application/octet-stream` otherwise (SVG, HTML) |
| `GET /kapi/aiworkspace/stream/<token>` | the bytes of one asset by `Range` (no bearer: the token from `asset.stream_url` is the credential), for `<video>` / `<audio>` |
| `GET /kapi/aiworkspace/session/ws` | the session channel (WebSocket; the first message carries the session token or link) |

Uploads go through NDM (above); export packages and replica databases are read from nfs-server's data plane,
`GET /bucket/v1/<bucket_id>/<key>?token=<token>` with the `download` of `doc.export` / `replica.bootstrap`.
| `GET /kapi/aiworkspace/schemas/richtext.basic.v1.json` | rich text schema definition (no auth) |

Operations (`operations[]` of a Commit): `entity.create|delete|restore|rename|set_keys|unset_keys|set_write_policy|set_derived|clone`,
`data.pin`, `tree.move|place`, `dir.set_member|remove_member|replace`, `table.insert_records|delete_records|set_values|unset_values|set_body|add_field|update_field|delete_field|add_option|update_option|delete_option|migrate_field`,
`richtext.apply_update|insert_blocks|replace_block|delete_blocks|move_block`. Parameters are exactly those of the design §3;
`core/tests/planner.rs` and `store/tests/*.rs` are executable examples of every one of them.

## Where this implementation differs from the design text

- **Export/replica download routes carry the `workspace_id`** (`/export/<workspace_id>/<export_id>`): there is no service-level database to look an export id up in.
- **Objects are in the zone's named store**, retained by bindings in the owner's buckets (Storage above); ids and encodings are ndn-lib's, verified in `store/tests/kernel.rs` (V01). The `objects` bucket is a `data` bucket of retention DirObjects, not an `object` bucket of `jobj` roots: binding a `jobj` would keep only that object.
- **Files are single `mix256` chunks (≤ 32 MiB)**; ChunkList for larger assets and record files is not written. Larger audio / video uploads are the browser's chunk-list FileObjects: staged, referenced and streamed, but not packed by a self-contained export (listed as missing).
- **Undo `partial` mode** determines the applicable part by probing each inverse operation on its own.
- **URL query sources** only reach adapters registered in the service (`fixture://` generated source for tests). There is no general HTTP fetcher: a URL in a document cannot make the service issue arbitrary requests.
- Extra methods: `doc.outline`, `doc.source_capabilities`, `diag.verify_refs`.
