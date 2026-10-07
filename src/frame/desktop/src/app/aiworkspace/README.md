# AI Workspace — Desktop app (phase one + phase two UI framework + UI improvement + standard object interaction + connectors)

Front end of the `aiworkspace` service (`src/frame/aiworkspace`). Design:
`doc/workspace/BuckyOS AI Workspace 第一期内置对象详细设计.md` ("the design" below),
`doc/workspace/BuckyOS AI Workspace 第二期规划.md` (the UI framework: data-source and canvas modes, Blocks,
wishes) and `doc/workspace/BuckyOS AI Workspace UI改进.md` (the floating canvas chrome, see "UI improvement"). App id `aiworkspace`, panel `AIWorkspaceAppPanel.tsx`. The `canvas` prototype next to it is the
source of the migrated demos only; nothing runs through it.

There is **no mock backend**. In the Desktop's mock runtime the app says so and stays empty unless the
dev override below points it at a real backend process.

## Structure

```text
AIWorkspaceAppPanel.tsx   entry: what opens (Desktop window: most recent workspace > list; tab: the address),
                          the leave check, the Desktop close / logout guard, the tab's beforeunload
AIWorkspaceRoute.tsx      the browser tab of its own: Desktop route `/workspace/:workspaceId?`
links.ts                  the only definition of the tab / share address `/workspace/<id>?surface=&block=`
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
anchors/                  annotation anchors (design §3.7), the same model as core/src/anchor.rs
  registry.ts             AnchorRegistry: one adapter per range kind — capture(selection) / locate(range);
                          built-in kinds and applications' `<app>/<name>` kinds register the same way
  quote.ts                `context.quote` matching (best inside the target, unmistakable elsewhere)
  richtext.ts             built-in `richtext_text` (Loro cursors + quote), block fallback, capture, and the
                          ProseMirror plugin placing every annotation at the deepest level that resolves
richtext/
  schema.ts               ProseMirror Schema generated from the schema JSON (`richtext_schema()` of the core)
  collab.ts               confirmed doc + working doc, debounced `richtext.apply_update`, remote imports,
                          refused update → draft + rebuild (design §3.3.7)
  blockId.ts              block_id plugin
  RichTextEditor.tsx      editor view, toolbar, object_embed node view, static (read-only) renderer
  drafts.ts, loro.ts
state/outline.ts          OutlineModel: the two trees kept up to date from the change stream's operations (no full
                          re-read on a move); per-entity subscriptions for Blocks
state/userState.ts        user work state (mode, sub-mode, active Surface, viewports, `ui:*` layout preferences…):
                          IndexedDB + server
state/recent.ts           the most recent workspace per service target and principal (app-level, localStorage)
state/freshness.ts        subscribes to `doc.freshness`; the front end never derives freshness itself
ui/
  WorkspaceList.tsx       list / New (blank or template) / import / export / fork / delete
  WorkspaceView.tsx       store context + shell; registers the shipped Block definitions
  shell/                  WorkspaceShell (top-level views, active Surface, right panel, dialogs, layout preferences),
                          MainToolbar (icon, name, Surface switcher, data source, main menu, collaboration),
                          MainMenu (the function index), PresenterToolbar (annotation, zoom and view navigation,
                          identity, share link), StatusSummary (the bottom-right status area: save / sync /
                          connection, persistent alerts and notices; the 修改状态 panel), SidePanel, dialogs (New, export, import, help, Mock, leave check),
                          popover.tsx (popovers and keyboard menus), shellContext.ts, panels, annotations panel
  sources/                data-source mode: DataTree (canvas content collapsed, filters), DataDetail (editors without a
                          Block), RelationsPanel (doc.relations), PropertiesPanel (versions, restore), PermissionsPanel
                          (presets, canvas permissions, subjects)
  canvas/                 CanvasView (sub-modes edit / view / presentation-edit placeholder, selection, pointer tool,
                          one-shot placement, insert, clipboard, group, cross-Surface move, keyboard), ObjectToolbar,
                          InsertCatalog + catalog.ts (the one insert catalog), clipboard.ts, surfaceManage.tsx
                          (rename / icon / delete pre-check), icons.tsx (preset canvas icons), FlowSurface, layout.ts,
                          tools.tsx (near toolbar, context menu, inspector)
  canvas/render/          RenderHost (world layer + camera transform, three-level culling with hysteresis, LOD
                          placeholders, mount budget, overlay, gestures that commit once; painting and hit testing
                          share one order, BlockTree pre-order as a z-index, frames in a stable DOM order by id),
                          camera.ts, spatialIndex.ts
  blocks/                 registry.ts (BlockDefinition, mode policy), BlockHost (lifecycle, mode dispatch, budget,
                          error boundary, generic fallback), useBlockContext (shared renderer / inspector / action
                          context), builtin.tsx (table / richtext / record / asset / note /
                          frame / shape), editors.tsx (the data editors shared with the data-source view), samples.tsx
                          (extension sample 1: metric, bar chart, frame-sequence video)
  extensions/             declarative.tsx (interpreter of `buckyos.block-def` declarative definitions),
                          HtmlBlockHost.tsx + htmlRuntime.ts + bridge.ts (HTML extension Blocks: same-origin iframe,
                          `window.aiws` v2 API — named bindings, watch, typed writes in one batch —, snapshots,
                          local fallback)
  wish/                   WishService (service runs `wish.xllm@1` / `wish.mock@1`: analyze, execute, feedback,
                          re-run / repair the program, preview with choices, apply by plan digest, history),
                          WishPanel + WishCandidate + WishPreviews, wishBlock, mockWishDef (the Mock executor as an
                          HTML definition entity)
  TableViewCell.tsx       virtualised table view, inline editing, filter / sort, conflicts; also the source mode
  tablePager.ts           keyset paging through doc.query, refresh from the change stream
  FieldManager.tsx        fields, options, migration pre-check
  annotations.tsx         cards next to a rich text, aligned with their anchors
  annotationInfo.ts       anchor states in words, labels, jump to an annotation
  ValueEditor.tsx, values.ts, creators.ts
api/demos.ts              the two phase-two demos as commit sequences (季度经营分析, AI 短片工作流)
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
file into a cache named by a hash of the build, serves the entry — the Desktop root and the tab addresses
`/workspace`, `/workspace/…` — network-first with the cached index.html as fallback, built files
cache-first, and touches nothing else. The build's `base` is `/` (absolute asset URLs), so the entry
works under a two-level path. A new build installs in the background
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
desktop, or `http://localhost:5174/workspace?aiwsDevToken=tok-alice` for the app in a tab of its own. `aiwsDevToken` stores `localStorage['aiworkspace.dev'] = {"token":"tok-alice"}` (optional
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

## Phase two in short

- **Two top-level modes** on one session, one undo stack and one pending queue: 数据源 (three columns) and 画布
  (Surfaces, edit / 查看 / 播放编辑 placeholder). Modes, sub-modes, the active Surface and every Surface's viewport are
  user work state (server + IndexedDB), never document commits.
- **Canvas**: the camera is a CSS transform, not React state; Blocks are culled in three levels with hysteresis;
  small Blocks get placeholders; editors and HTML runtimes have a mount budget; a drag or resize is one commit
  (Esc = none), auto-merged (later arrival wins) with a "位置已被修改 / 重新应用" notice. Insert creates data in the
  Surface's content folder plus its Block in one commit; frame / shape are pure UI Blocks.
- **Blocks**: a front-end registry decides which Renderer shows which data; unknown renderers, unsupported versions,
  unreadable data and thrown renderers fall back inside the Block. Declarative and HTML definitions are document
  entities. HTML Blocks run same-origin (no sandbox, D16) through `window.aiws`.
- **Wishes** (许愿格 v0.2) run on the service: analyze (task description, named inputs, output contract, checks) →
  execute (the model writes a program, the service runs it under Deno; candidate with previews and checks) →
  feedback rounds → apply exactly the previewed plan (one commit: results, Blocks, program, refinements,
  dependency records; manual edits ask keep / replace / new). "只重跑程序" needs no model. The Mock executor stays an
  HTML definition entity; its results are planned and applied by the service (`wish.mock@1`) and labelled "模拟".
- **Freshness** comes from `doc.freshness` (core) and is shown identically on wish Blocks, result Blocks, the data
  tree and the detail panels.

## UI improvement (doc/workspace/BuckyOS AI Workspace UI改进.md)

- **What opens.** The usual way is a browser tab of its own: `https://<zone host>/workspace` (the list) or
  `/workspace/<workspace>` (one workspace, also the share link, with `?surface=<surface>&block=<block>`; the
  two are removed from the address once the workspace is open). In a tab the address is the state: opening,
  switching and closing change it (after the leave check), back / forward / reload follow it, and nothing is
  restored automatically. In a Desktop window a normal start reopens the workspace this identity last opened
  successfully (`state/recent.ts`, keyed `mode|target|principal`) on its remembered Surface (`surface:active`);
  closing a workspace pauses the restore for the rest of the app session; the main menu's "在新标签页中打开"
  opens the workspace and its Surface in a tab. A gone Surface falls back to the first readable one and says so.
  A workspace never reopens into the presentation-edit placeholder.
- **Chrome.** The canvas fills the window; the main toolbar (top left), the presenter toolbar (top right: add
  annotation, zoom ▾ with presets / fit, identity, share) and the vertical object toolbar float in screen space
  (`camera.setInsets` keeps fitting and centring out from under them). There is no bottom-right navigation area:
  the bottom-right corner of the work area is the status area (`StatusDock`: the save / sync summary, alerts and
  notices stacked above it; expandable work logs are meant to live there too).
  The right panel shows one tab at a time (属性 / 引用与依赖 / 批注 / 协作 / 修改状态): layout width when the
  application container is ≥ 1100 px, a drawer below. Size classes: wide ≥ 1100, medium ≥ 760, narrow.
- **One action per function.** Menus, toolbar buttons, the context menu and shortcuts call the same functions in
  `CanvasView` (`CanvasCommands`). Insertion entries come from `BlockDefinition.catalog`; workspace extensions
  from `buckyos.block-def` entities. Nothing prompts for a title: text, notes and wishes open their editor at once;
  entries that need data or a file ask for it first.
- **Clipboard.** Per window, in memory, same workspace only: copy = new Block / group ids, same data references,
  one commit; cut = mark, paste moves in one commit (identity kept). Editors keep the text clipboard.
- **Leaving.** Close, switch, a changed tab address, window close and log out run `store.prepareLeave()`; what would
  be lost is asked about (返回处理 / 导出后离开 / 仍然离开). Closing or reloading a tab with such edits makes the
  browser ask (`beforeunload`). In a Desktop window the identity menu logs out through the Desktop
  (`buckyos:request-logout`); in a tab it signs out itself (`src/auth/signOut.ts`, the same `/sso_logout` flow).
- **Layout preferences** live in the user work state under `ui:` (`ui:object-toolbar`, `ui:presenter-toolbar`,
  `ui:grid`, `ui:side`, `ui:pinned-defs`); "恢复默认布局" clears them, never the document.

## Standard object interaction (doc/workspace/标准对象的交互改进.md, S1–S3)

- **Object = content, host = state.** A canvas frame (`.aiws-frame-block`) draws nothing: each type draws its own
  look (notes a coloured sheet, wishes a card, text straight on the canvas). `BlockDefinition.chrome` only says
  whether content larger than the Block is clipped (`clip`, the default) or drawn outside (`none`: frame, shape,
  note). Hover, selection, editing, the cut marker, lock badge and gesture hints are drawn by the RenderHost
  overlay (SVG for outlines and handles, an HTML layer for affordances, the rotation handle and hints).
- **Hover** shows a solid outline at once and the Block's affordances after 150 ms (`BlockDefinition.hover`; by
  default the data's name): at most one label and two buttons, buttons only when the Block is ≥ 64 px on
  screen. A hovered or selected Block publishes its outline shape, resize rule and affordances to the overlay
  through `BlockMetaContext` (no extra reads). On touch the affordances come with the selection.
- **Selection** (edit mode, layout allowed, not locked, not editing): round corner handles (Shift keeps / frees
  the proportions per `resize.aspect`), edge zones, a rotation handle outside the bottom-left corner (Shift
  snaps 15°, a double press resets), sizes by pointer type (`usePointerType`). Several Blocks: member outlines
  and one group box.
- **Near toolbar** (`tools.tsx`): `ToolbarItem`s — the Editor's tools while editing (from `EditorToolbarContext`:
  editors call `useEditorToolbar(owner, items)`, extension Editors `RenderContext.setEditorToolbar`), the type's
  tools (`BlockDefinition.toolbar`; an extension without one shows its actions as words), the common ones
  (annotate, lock / unlock, AI) and "more" (= the context menu). Items that do not fit move into "more". Where
  there is no near toolbar (flow page, data-source view) the same items are drawn inline (`InlineTools`) and the
  lock stays a bar.
- **Intents** (`editorToolbar.ts`): `requestIntent(cellId, value)` carries "open this panel", "type this", "put the
  caret here", "start" or "preview" from the canvas into an Editor that mounts later; the receiver consumes it.
- **Wish** on a free canvas opens in the right panel (`wish` tab); its Block stays a card (`OpenWishContext`).
  "Run" opens it and starts the next pass (analysis, or execution once the analysis is current).
- **Rotation** is `placement.rotation` (degrees, `[0, 360)`): `Laid.rotation` / `Laid.bounds` (`layout.ts`), the
  frame's CSS `rotate` (drags use `translate`, which composes with it), hit tests in the turned shape
  (`spatialIndex.ts`, `geometry.ts`), resizing in the Block's own coordinates. Groups do not rotate yet.
- **Lock** is the shared `locked` key of a Block or group (inherited by a group's members): no handles, no moves,
  Delete / cut / grouping skip it with a notice; the core never enforces it.
- **Version check**: a service on another `protocol_version` (at open, after an outage, on a refused commit, or
  when a replica cannot replay a commit) stops the session with "please refresh" (`VERSION_MISMATCH`), keeping
  what is pending.

## Connectors (doc/workspace/连接线实现方案讨论.md; 标准对象的交互改进 §6, S4)

- **Data**: a Cell with `view.type = connector`. The placement is the box around the two stored ends (`w` / `h` may
  be 0) and `flip` says which corner is which; `start` / `end` are null (a coordinate) or `{ entity_id, anchor }`
  (`auto`, or `point` at a normalised spot of the target); `route` (straight / elbow / curve), `controls`
  (`{u, v, dx, dy}` in the endpoint frame), `label` (`{t, offset}`; the text is the Cell `title`), `config` (look).
  The outline projects all but `config` as `EntityEnvelope.connector` (`state/outline.ts` mirrors the core).
- **Geometry** (`ui/canvas/connectors/geometry.ts`, pure): stored ends, connection points (rect / ellipse, turned
  targets), the endpoint-frame decomposition, the three routers (automatic elbow adapted from React Flow's
  smoothstep; curves as Hermite pieces through the points), arc length, label point, nearest point, bounds. Part
  of `connector@1`: a change that alters existing drawings needs a new `view.version`.
- **Layout** (`connectors/layout.ts`): the second pass of `layoutSurface` routes every line against the Blocks'
  world rectangles; `Laid.rect` stays the stored box (all placement writes use it) and `Laid.bounds` covers the
  path and label. A target's outline comes from `BlockDefinition.shape` (now a function of the payload);
  `ShapeBook` reads the payload of targets whose shape depends on it. Lines enter the spatial index along their
  path and hit within 6 px (or half their width).
- **Drawing** (`ConnectorFrame.tsx`, `paint.ts`): one frame per line in the paint order, `pointer-events: none`,
  SVG from one markup function that gestures also use to repaint in place (`LineRegistry`); caps scale with the
  width; the label breaks the line (a mask) unless it has a fill; low zoom drops the label and simplifies caps.
- **Gestures** (`RenderHost.tsx`): connection handles on one selected Block (mouse), the connector tool (L),
  a dragged end snapping to side midpoints (`point`) or the Block under it (`auto`), end / bend / segment / label
  handles on a selected line, a double press on a line to edit its label, edge panning while dragging. Moving,
  resizing or turning Blocks re-routes their lines per frame without React. One gesture, one commit
  (`connectors/ops.ts`); payload keys are written with `expect`, placements auto-merge.
- **Lifecycle** (`CanvasView.tsx`, `clipboard.ts`): deleting Blocks freezes their lines in the same commit; copy
  keeps only bindings inside the copied set; flow pages neither show nor accept lines; a line drawn to blank
  canvas offers the "next object" (note, text, shape, wish) bound to its end.

## Block extension contract

Register a `BlockDefinition` in `ui/blocks/registry.ts`. `useBlockContext` loads the Cell payload and key
revisions, resolves its exact renderer version and supplies the same definition, configuration, mode and
read-only state to the renderer, Inspector and near-toolbar actions; `chrome`, `shape`, `resize`, `hover` and
`toolbar` (previous section) are optional, a definition without them still works. Registry changes invalidate resolution
for mounted Blocks. Renderers and custom Inspectors have local error boundaries; failed actions report a
notice. Rich-text embeds pass their depth through this context and stop at `MAX_EMBED_DEPTH` (3).

The `html` and `declarative` registrations declare `definitionKind` and load `def_ref`. Resolution checks
the document definition's kind, `accepts`, `allow_no_source`, `config_schema` and HTML `api_version`
before mounting an implementation. `accepts: []` accepts no bound source; use `allow_no_source: true`
for a pure UI Block. Omitted `accepts`, `allow_no_source` and `default_size` use registered defaults. Validation uses the
existing Zod dependency's `fromJSONSchema` (minimum 4.4.3); its supported JSON Schema subset applies,
and conversion errors become a local `invalid_definition` fallback. Invalid values become `invalid_config`.

`config.snapshot` is reserved host metadata, excluded from the extension's configuration schema. It stores
`{ object_id, media_type, size }` and is indexed as an asset reference by the shared Rust core. Snapshot bytes
follow Cell read permissions and travel through export/import and offline preparation (subject to the
normal asset-size limit). `aiws.snapshot()` reports failed saves and refuses writes in view mode. HTML
startup timeouts, crashes and disposal reject waiting calls and release the iframe; retry creates a new runtime.

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
- Flow Surfaces reorder by buttons, not drag and drop.
- Phase two, not done: play mode and presentation editing (entry and placeholder only); Notion-style layout
  containers and canvas templates; connectors; real executors (xllm, agent-work-session); table version restore;
  drag from the data tree onto the canvas (use "添加已有数据…"); multi-user cursors; coordinate comment pins;
  an extension marketplace or AI-generated extension pipeline (HTML definitions are hand-made); plan §15 R1 is
  accepted as is (an HTML definition added by any editor or carried by an imported package runs in the opening
  user's session). Verified only against the standalone backend with the Desktop shell on its mock runtime, in
  Chromium; no real-Zone run. The render probe (`tests/aiworkspace/probe.spec.ts`) measures the production build
  served by `vite preview` and writes `test-results/aiworkspace-probe-*.json`; the acceptance report quotes it.
- TableView: no grouping, no manual order, no column resizing; the filter editor builds one condition
  (saving ANDs it with the saved filter); `object_ref` values are displayed but not editable; URL query
  tables are not treated specially (writes are refused by the backend and shown as such).
- Record schemas cannot be edited. Annotations cannot be edited after creation (re-anchoring exists in the
  protocol, not in the UI), are always shared and of kind `note`. Annotation cards are drawn next to rich
  text only; table cells show a mark. Embedded (read-only, static) rich text shows no annotations.
- No UI for grants (`ws.grant` / `ws.revoke`), checkpoints, `diag.*`; no partial undo (`mode: "partial"`):
  an undo that conflicts is reported and does nothing.
- Rich text: no UI to create a `link` mark or set an ordered list's start; no remote cursors; the caret is
  not restored after an undo. Drafts of refused updates live in `localStorage`, can be read and copied as
  JSON, but cannot be re-applied to the editor with one click.
- Lock: no "request hand-over"; the lock is released 30 s after focus leaves the cell (and on close), not
  on the blur itself.
- UI improvement, not done (doc §13 P2 and the gaps it names): online members / presence, follow, remote
  pointers; starting a presentation, presentation sessions and interaction buttons (the menu entry is disabled and
  says why); annotation anchors on pure UI Blocks or blank canvas points; cross-workspace paste; image upload as a
  canvas icon. Browser zoom 200% was not automated.
- Connectors, not done: named ports, lines between lines, obstacle avoidance and line jumps, lines across
  Surfaces, several or rich labels, touch editing (touch selects lines only), wish-generated lines, explicit
  business relations (连接线方案 §3.2).
- Standard object interaction, not done: whole-selection resizing, align / distribute, snapping guides, remote selections,
  HTML API v3 (`aiws.hover` / `aiws.toolbar`), touch editing beyond the handle sizes, keyboard rotation, rotating
  groups (S5). Typing on a selected text starts editing only for keys that produce a keydown (an IME composition
  does not; double-click instead). A note's colour is not a hover affordance (the toolbar has it).
- The app's own UI text is Chinese only (the app name and summary are in the Desktop dictionaries).
- The zone transport follows the SDK's normal service path but has only been type-checked here; the e2e
  suite runs through the dev override.
