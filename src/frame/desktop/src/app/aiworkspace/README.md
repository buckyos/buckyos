# AI Workspace — Desktop app (phase one)

Front end of the `aiworkspace` service (`src/frame/aiworkspace`). Design:
`doc/workspace/BuckyOS AI Workspace 第一期内置对象详细设计.md` ("the design" below). App id `aiworkspace`,
panel `AIWorkspaceAppPanel.tsx`. It is unrelated to the `canvas` prototype next to it.

There is **no mock backend**. In the Desktop's mock runtime the app says so and stays empty unless the
dev override below points it at a real backend process.

## Structure

```text
AIWorkspaceAppPanel.tsx   entry: workspace list ↔ one open workspace
api/
  transport.ts            the only module that knows how to reach the service (zone path / dev override)
  client.ts               one typed method per kRPC method of the backend README
  session.ts              WorkspaceSession interface (+ SessionMode, OfflineControls) and
                          OnlineWorkspaceSession (online direct mode): commit envelope, idempotency
                          keys, per-tab session_id, unknown-result resolution, change-stream
                          follower, EPOCH_MISMATCH / BASE_TOO_OLD stop
  sample.ts               "create sample": replays fixtures/project-workspace.commits.json
  wasm.ts                 loader of the shared Rust core (order_key_between, rich text schema, canonical AST)
  ids.ts, types.ts, testHooks.ts
offline/                  the browser offline layer (design §6) — see "Offline" below
  protocol.ts             messages page ↔ Replica Worker, pending row / export shapes
  replica.worker.ts       Replica Worker: SQLite WASM on `opfs-sahpool` + the WASM `Replica` engine;
                          client tables, one-transaction rules, asset area, fault injection for tests
  client.ts               ReplicaClient: RPC to the Worker, timeouts, dead-Worker detection, StorageFailure
  holder.ts               Web Lock holder, "准备离线" (bootstrap → download → import → assets),
                          opening a prepared replica, the local index of prepared workspaces
  replicaSession.ts       ReplicaWorkspaceSession: reads from the working view, writes through the
                          pending queue, send loop and reconnect order of design §6.4
state/
  store.ts                per-workspace store: invalidation counters fed by the change stream, submit(),
                          pending submissions ↔ save states, export of everything not yet accepted
  edits.ts                save states of design §6.5 (未保存 / 已保存到本设备 / 已提交 / 需要处理)
  undo.ts                 UndoCoordinator (design §2.7), incl. `pending` entries (unsent submissions)
  locks.ts                write-lock leases (design §2.11)
  hooks.ts
richtext/
  schema.ts               ProseMirror Schema generated from the schema JSON (`richtext_schema()` of the core)
  collab.ts               confirmed doc + working doc, debounced `richtext.apply_update`, remote imports,
                          refused update → draft + rebuild (design §3.3.7)
  blockId.ts              block_id plugin
  RichTextEditor.tsx      editor view, toolbar, object_embed node view, static (read-only) renderer
  drafts.ts, loro.ts
ui/
  WorkspaceList.tsx       list / create / sample / import / export / fork / delete
  WorkspaceView.tsx       top bar, flow page, add menu, reorder
  panels.tsx              outline, annotations, Mock run, edit states
  cells.tsx               cell bodies per view type, lock bar, record, asset
  TableViewCell.tsx       virtualised table view, inline editing, filter / sort, conflicts
  tablePager.ts           keyset paging through doc.query, refresh from the change stream
  FieldManager.tsx        fields, options, migration pre-check
  ValueEditor.tsx, values.ts, creators.ts
fixtures/project-workspace.commits.json   copy of the backend fixture (the e2e suite asserts they are identical)
wasm/                     wasm-bindgen output of aiworkspace-wasm (src/frame/aiworkspace/wasm/build.sh)
```

Outside this directory: `src/serviceWorker.ts` (registration), `src/service-worker/sw.template.js`
(the worker) and the `desktop-service-worker` plugin in `vite.config.ts` (generates `dist/sw.js`).

**One seam.** Views never call the client. Everything goes through `WorkspaceSession`
(`api/session.ts`). `AIWorkspaceAppPanel.open()` picks the implementation: `ReplicaWorkspaceSession`
when this device has a prepared replica of the workspace and this window gets its holder lock,
`OnlineWorkspaceSession` otherwise (the top bar says which, and why).

## Offline (design §6, first-phase document §7)

**Supported environment.** Chromium-based browsers with OPFS synchronous access handles in
dedicated Workers, Web Locks and Service Workers, in a secure context (HTTPS or localhost).
Tested: Chromium 151 (Playwright's headless shell, Linux). Firefox and Safari are **not tested**;
there is no IndexedDB fallback — where a prerequisite is missing the app says "离线不可用" with the
reason and stays in online direct mode. Storage: the official `@sqlite.org/sqlite-wasm` (3.53.4) on
the **`opfs-sahpool` VFS**, one pool (`/.aiworkspace/replica-<workspace_id>`) and one database per
workspace; it needs neither COOP/COEP headers nor `SharedArrayBuffer`.

**Preparing.** "准备离线" in the top bar of an open workspace: persistent storage is requested
(`navigator.storage.persist()`; refused → offline unavailable), the holder lock is taken,
`replica.bootstrap` → download → import into OPFS, then every referenced asset up to 8 MiB is copied
into an OPFS asset area (larger or failed ones are listed as skipped). A backend refusal (no
workspace-level `read`) is shown as is.

**Single holder.** The Web Lock `aiworkspace-replica:<workspace_id>` (exclusive, `ifAvailable`),
held by the page for as long as its session lives. A window that does not get it never starts a
Replica Worker: it runs online direct, says "此窗口未启用离线", and is read-only while the backend
is unreachable. When the holder is closed the lock is free and "接管离线副本" in the other window
opens the replica there.

**What "saved" means.** The replica database is the imported confirmed layer plus
`pending_submissions`, `drafts` and `replica_meta`. The working view is not stored: it is the
confirmed layer with the pending submissions replayed by the WASM engine (first-phase document
§11-12; hence no `preimage_json`). A local edit resolves `saved_locally` only after the transaction
inserting its pending row committed; rows changed by `apply_remote`, `confirmed_seq` and
removals / state changes of pending rows are one transaction; if a transaction fails the engine is
rebuilt from the database. Rich text: editor updates are `richtext.apply_update` submissions, so
text typed offline survives as pending rows (there is no separate `richtext_working` table).

**Sending** follows design §6.4 literally (`replicaSession.ts` header). The network stays in the page
(the zone transport's session lives there); the Worker is engine + storage only.

**Application resources.** A production build has `sw.js` next to `index.html` (scope = the Desktop's
directory, so the control-panel directory handler needs no extra headers). It precaches every built
file into a cache named by a hash of the build, serves the entry network-first with the cached copy as
fallback, built files cache-first, and touches nothing else. A new build installs in the background
and takes over when the old pages are closed (or on "现在更新并重新载入"); old caches are deleted on
activation. If the entry or any built file is missing at an offline start, the worker answers with a
page that says so instead of starting a broken application. The Vite dev server registers no service
worker; the app then says that only already open windows keep working.

## Run against the standalone backend

```bash
# backend (from buckyos/src)
cargo build -p aiworkspace
mkdir -p /tmp/aiws && echo '{"tokens":{"tok-alice":{"principal":"alice"},"tok-bob":{"principal":"bob"}}}' > /tmp/aiws/tokens.json
$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys;print(json.load(sys.stdin)["target_directory"])')/debug/aiworkspace \
  --data-dir /tmp/aiws/data --listen 127.0.0.1:4120 --auth-file /tmp/aiws/tokens.json --fixture-sources

# desktop (from src/frame/desktop) — the service sends no CORS headers, so Vite forwards /kapi/aiworkspace
AIWS_BACKEND=http://127.0.0.1:4120 pnpm run dev
```

Open `http://localhost:5174/?scenario=normal&aiwsDevToken=tok-alice` and start "AI Workspace" from the
desktop. `aiwsDevToken` stores `localStorage['aiworkspace.dev'] = {"token":"tok-alice"}` (optional
`baseUrl`, `principal`); remove that key to go back to the zone path. Without `AIWS_BACKEND` the Vite
config is unchanged.

In a real zone nothing is configured: the app uses `buckyos.getServiceRpcClient('aiworkspace')` and
sends `Authorization: Bearer <session token>` to the upload / asset / export routes.

## Tests

```bash
# from src/frame/desktop; cargo must be on PATH (or set AIWS_BIN=<path to the aiworkspace binary>)
pnpm exec playwright test --config=playwright.aiworkspace.config.ts
```

One command runs everything. The config builds and starts the real backend on a free port with a
temporary data folder (`tests/aiworkspace/start-backend.mjs`), starts Vite with `AIWS_BACKEND`, and
runs `tests/aiworkspace/*.spec.ts`. The Desktop shell stays on its mock runtime; the app talks to the
real process. Assertions about persisted state go through kRPC directly, not only through the DOM.

The offline specs (`offline*.spec.ts`, V15–V17) additionally use a production build served by
`vite preview` (built by the config into `test-results/aiworkspace-dist`), because only that has the
service worker. Each of those tests has its own on-disk browser profile (Chromium grants persistent
storage only there, and only with the durable-storage permission the fixture sets) and reaches the
preview server through its own TCP relay (`offline-fixtures.ts`): closing the relay is a real network
cut — connection refused for the page, the Worker and the service worker — and gives every test its
own origin, i.e. its own OPFS, service worker and localStorage.

## What is implemented

- Registration as a built-in app; workspace list with create, sample, open, import (semantics must be
  chosen), export (share / personal backup) with download, fork, delete.
- Outline of all entities, flow page of cells and groups in `(order_key, entity_id)` order; create
  (group, rich text, table, record, image, extra table view), rename, delete with explicit subtree list,
  reorder and move without `expect` (auto-merge), order keys from the WASM `order_key_between`.
- TableView: virtualised rows, keyset paging (`best_effort`), inline editors for text / number / decimal /
  date / datetime / boolean / select / multi_select, every write with the rev seen when the edit began and
  `field_type_revs`; add / delete record; add / rename / delete field; add / rename / delete option;
  migration with `doc.prepare` report before the commit; session filter / sort vs. "save view" (separate
  keys); diagnostics and `VIEW_BROKEN`; derived and manual-override marks.
- RichText: ProseMirror + Loro (`loro-prosemirror` sync plugin), confirmed + working documents,
  debounced updates (500 ms), remote updates into both documents (deferred during IME composition),
  block_id plugin, object_embed node view (read-only cell), object_link, refused update → draft.
- Record, Asset (upload, authenticated fetch → blob URL, availability), Annotation (table cell / block,
  anchor state).
- UndoCoordinator: one stack of editor steps and commits, Ctrl/Cmd+Z pops exactly one, redo.
- Save states and conflict UI: my input is kept until I choose; unknown results can be resent.
- Offline replica (previous section): explicit preparation, cold start without the network, the four
  save states, pending submissions as undo entries (undo = removal from the queue, no network),
  conflicts / refusals / revoked access kept as "需要处理" with the backend's result (dismiss, resend,
  or re-edit), stop on `EPOCH_MISMATCH` / `BASE_TOO_OLD` / lost access with everything kept, export of
  unsaved inputs and of the pending queue as a JSON file, storage failures (quota, transaction error,
  dead Worker) shown as "未保存" with an export, "browser offline" told apart from "BuckyOS
  unreachable", `lock_required` objects read-only without the backend, Mock runs / uploads / URL query
  tables saying that they need the backend.
- Write locks: acquire, renew every 20 s, release, holder display, `LOCK_LOST`, policy toggle and break.
- Mock run: candidate with `prepare` result and warnings, apply / cancel, everything labelled 模拟结果.

## What is not implemented (the UI says so or does not offer it)

- Offline, not done:
  - no deferred asset upload: adding or replacing an asset needs the backend (`blocked_asset` is never used);
  - a window that never prepared a replica keeps the earlier direct-mode behaviour when the backend
    goes away (edits stay "未保存" with a retry); only windows that could not become the holder are
    forced read-only;
  - every pause in rich text typing is one pending row; rows are not merged, and one refused row
    takes the later rows of that editor with it (their content is kept as one draft);
  - the export is a plain JSON file, not an attachment of the personal-backup package, and there is
    no import of such a file;
  - the service worker caches the application, not the Desktop shell's own backend calls: a cold
    offline start has only been exercised with the shell on its mock runtime; in a real zone the
    shell's login / session bootstrap would have to tolerate being offline first;
  - re-authentication is whatever the transport does per request; `BASE_TOO_OLD` is handled like
    `EPOCH_MISMATCH` but the backend never produced it in the tests;
  - lock holders of `lock_required` objects are not known offline.
- Only the `immediate` commit strategy; no explicit drafts (`richtext_diff` is not used).
- Wake-ups use the `doc.wait_changes` long poll only (no kevent). Lock holders are refreshed by re-reading
  the outline every 5 s while some entity requires a lock.
- Reordering is by buttons, not drag and drop. Only `flow` layout is rendered; `placement` is ignored.
  Only the first page of a workspace is shown.
- TableView: no grouping, no manual order, no column resizing; the filter editor builds one condition
  (saving ANDs it with the saved filter); `object_ref` values are displayed but not editable; URL query
  tables are not treated specially (writes are refused by the backend and shown as such).
- Record schemas cannot be edited. Annotations cannot be edited after creation, are always shared and of
  kind `note`.
- No UI for grants (`ws.grant` / `ws.revoke`), checkpoints, `diag.*`; no partial undo (`mode: "partial"`):
  an undo that conflicts is reported and does nothing.
- Rich text: no UI to create a `link` mark or set an ordered list's start; no remote cursors; the caret is
  not restored after an undo. Drafts of refused updates live in `localStorage`, can be read and copied as
  JSON, but cannot be re-applied to the editor with one click.
- Lock: no "request hand-over"; the lock is released 30 s after focus leaves the cell (and on close), not
  on the blur itself.
- The app's own UI text is Chinese only (the app name and summary are in the Desktop dictionaries).
- The zone transport follows the SDK's normal service path but has only been type-checked here; the e2e
  suite runs through the dev override.
