---
name: aiworkspace_template_dev
description: Build and maintain AI Workspaces from the command line — create a workspace, add tables, documents and Blocks by committing operations, put mind maps (Block-controls) on a canvas from a Markdown outline, export and import packages, reference data across workspaces, run wish blocks, publish data by path and insert published links.
requires: the buckyos command (agent container: /usr/local/bin/buckyos; host: $BUCKYOS_ROOT/bin/buckyos) signed in; Deno for scripts that use the websdk directly; the workspace must grant you the capability each step needs (structure/update to write, manage to share or publish, run to start a wish)
baseline: standard
---
#### When to use
- You are asked to set up a workspace for someone: tables with fields and rows, a document, a canvas with Blocks bound to them, a wish block that computes results.
- You need to move content between workspaces or Zones: export a share package, import it elsewhere, or link one workspace's published table into another so it follows updates.
- You already hold a run of a wish block to execute: that is `aiworkspace_run`, not this manual.

#### Before you start
- `buckyos aiws --help`: the read verbs (`info`, `read`, `query`), the Block-control verbs (`controls list|describe`, `import`, `export`) and the run verbs. Everything else is one kRPC method of the service (`/kapi/aiworkspace`), called through the websdk: `getAiWorkspaceClient()` in `buckyos-websdk/src/aiworkspace_client.ts` (`info`, `outline`, `read`, `query`, `resolve`, `prepare`, `commit`, `run*`, and `call(method, params)` for the rest).
- The protocol is in `src/frame/aiworkspace/README.md` (Protocol: the method table with its stability level, the operation catalogue `schemas/operations.json`, selectors, error codes). Methods marked `stable` are the ones to build on.
- A write is a Commit: `doc.commit { protocol_version: "1.1", workspace_id, epoch, idempotency_key, session_id, operations: [...] }` (`"1.0"` is still accepted, without Block-controls). `epoch` comes from `ws.get_info`; keep your `idempotency_key` — resending the same key never writes twice (`doc.get_submission` tells you what happened).

#### Shortest path (a workspace with a table and a canvas)
```bash
# a Deno script using the websdk; run it with: deno run -A build.ts
import { buckyos } from "buckyos"            # or the local dist/node.mjs
const aiws = buckyos.getAiWorkspaceClient()
const ws = (await aiws.call("ws.create", { title: "季度经营" })).workspace_id
const { epoch } = await aiws.info(ws)
await aiws.commit(ws, { protocol_version: "1.1", workspace_id: ws, epoch, idempotency_key: "build/1", session_id: "s1", operations: [
  { op: "entity.create", entity_id: "sales", type_id: "buckyos.table-source", parent_id: "data", order_key: "a", name: "销售",
    payload: { fields: [{ field_id: "month", name: "月份", type: "date" }, { field_id: "amount", name: "金额", type: "decimal", scale: 2 }] } },
  { op: "table.insert_records", source_id: "sales", records: [{ record_id: "r1", values: { month: "2026-07-01", amount: "10.50" } }] },
  { op: "entity.create", entity_id: "sf-content", type_id: "buckyos.container", parent_id: "canvas-content", order_key: "a", payload: { kind: "folder", system: "surface_content", surface_id: "sf", title: "看板" } },
  { op: "entity.create", entity_id: "sf", type_id: "buckyos.container", parent_id: "surfaces", order_key: "a", name: "看板", payload: { kind: "surface", layout: { mode: "free" }, title: "看板", content_folder_id: "sf-content" } },
  { op: "entity.create", entity_id: "blk-sales", type_id: "buckyos.cell", parent_id: "sf", order_key: "a", placement: { x: 40, y: 40, w: 520, h: 300 }, payload: { view: { type: "table" }, source_ref: { entity_id: "sales" }, title: "销售" } },
] })
```
Check with `buckyos aiws query --workspace <ws> --source sales` and `buckyos aiws info --workspace <ws>`. Data created under `canvas-content` belongs to its canvas (D05: it goes when its last Block goes); data under `data` is shared.

#### Inputs and outputs
- **Entity types**: `buckyos.table-source` (fields + records), `buckyos.richtext` (`content: { type: "doc", content: [blocks] }`, Markdown converts with the core's `markdown.rs`), `buckyos.record`, `buckyos.asset-ref` (upload first: `buckyos bucket put obj://bucket/<bucket>/<key> <file>` into a bucket your session may write, then `asset.import_object { workspace_id, obj_id }` with the printed `obj_id` — an account may instead pass `bucket_id` + `key` of its own `user/aiworkspace/<user>/assets`; the answer's `object_id` goes into the payload; optional `poster` — the `object_id` of another uploaded PNG/JPEG/WebP/GIF picturing a video, sound or PDF — plus `image { width, height }`, `duration_ms`, `page_count`), `buckyos.dir`, `buckyos.wish`, `buckyos.cell` (a Block: `view.type` from the Renderer catalogue, `source_ref` to the data it shows; a pen stroke is a Cell with `view: { type: "ink", version: 1 }` and its points in `config` — write it exactly as the backend README section "Ink" says, the Desktop's `src/frame/desktop/src/app/aiworkspace/ink/model.ts` `encodeStroke` is the reference encoder).
- **Block-controls (mind maps)**: a container whose `kind` is a control id lays out the Blocks it holds — do not compute their coordinates. `buckyos aiws controls list` names the controls (what each is, when to use it); `buckyos aiws controls describe mindmap` prints the full description to follow (config, child rules, `doc.commit` examples, formats, don'ts). The shortest way to a mind map is an outline file: `buckyos aiws import plan.md --workspace <ws> --parent <surface id> --at 100,80` (a nested Markdown list, or `.opml`); `buckyos aiws export <control id> --workspace <ws> [--format opml] [--out plan.md]` reads one back. With `doc.commit`, nodes are `buckyos.cell` with `view: { type: "mindnode" }`, the text in `title`, `parent_id` = the parent node, placement `{ "x": "auto", "y": "auto", "w": "auto", "h": "auto" }` (a non-text node gives numeric `w` / `h`).
- **Reading**: `doc.outline` (the trees), `doc.read { entity_id, selector? }`, `doc.query { source_id | view_id, filter?, sorts?, limit, cursor }`; `doc.resolve { reference | path }` turns a path such as `/data/销售` into the reference object `{ workspace_id, entity_id, mode: live | fixed, version? }` — keep references, not names (D10).
- **Export / import**: `doc.export { mode: "share" | "personal_backup", self_contained? }` puts the package into the workspace owner's `exports` bucket (kept 7 days) and answers `download: { bucket_id, key, token }`; fetch `GET https://<zonehost>/bucket/v1/<bucket_id>/<key>?token=<token>` (or `buckyos bucket get obj://bucket/<bucket_id>/<key> <file>` where your session may read that bucket). `share` is content only (no collaboration history); `personal_backup` carries it. Import: upload the package (`buckyos bucket put`), then `ws.import { obj_id, semantics: "new" | "restore", replace? }` — an account may pass `bucket_id` + `key` of its own `user/aiworkspace/<user>/exports` instead, and that upload is consumed (or `ws.fork` for a copy inside this Zone).
- **Across workspaces** (D26): the publisher runs `ws.publish { entity_id }` (manage) and gets a link `https://aiws-<id>.<zonehost>/<workspace>/<name>` (`<id>` is the workspace id without `ws_`; or `/kapi/aiworkspace/pub/<ws>/<name>`); the consumer creates an entity with `payload.source = { kind: "workspace", workspace_id, entity_id, mode: "live" }` (another Zone: add `zone` and `url`) and refreshes it: `proc.start { program: "source.refresh@1", params: { entity_id }, idempotency_key }` → `proc.get` until `succeeded`. Any CSV / JSON / Markdown URL works the same with `source: { kind: "url", url }` (`refresh: { interval_s }` schedules it). A sourced entity is read-only (`SOURCE_READ_ONLY`): refresh, do not edit.
- **Wish blocks**: create a `buckyos.wish` (`prompt`, `executor: "xllm"`, `output: { container_id, surface_id, name }`), then `proc.start { program: "wish.xllm@1", params: { wish_id, stage: "analyze" | "execute", execution: { mode: "local" | "hosted" } } }`; `proc.get` reports the candidate and a preview; `proc.apply { run_id, plan_digest, session_id }` writes it. In `hosted` mode the owner's Agent executes (`aiworkspace_run`).
- **Sharing with people**: `ws.grant { subject, capabilities: ["read", ...], scope_entity_id? }` (manage); `ws.list_grants`; entries for visitors are `ws.create_entry` / `ws.list_entries`.

#### When it fails
- `rejected` with `errors[]`: read `code` / `sub_code` (`ID_CONFLICT`, `NAME_CONFLICT`, `SOURCE_READ_ONLY`, `LOCK_REQUIRED`, `QUOTA_EXCEEDED`…), never the text. A conflict answers the current revision in `conflicts[]`: read again, resend with the new `expect`.
- `PERMISSION_DENIED`: `ws.get_info.capabilities` lists what you may do; the owner grants more with `ws.grant`.
- `epoch` mismatch after an import or restore: call `ws.get_info` again.
- An unknown result (the request timed out): `doc.get_submission { epoch, idempotency_key }` — `accepted` means it is in; otherwise resend the same key.

#### Check the result
`buckyos aiws info --workspace <ws>` (head sequence, capabilities, usage), `buckyos aiws read <entity> --workspace <ws>`, `buckyos aiws query --workspace <ws> --source <table>`. The workspace's page for people is on its own origin: `https://aiws-<id>.<zonehost>/<workspace>` (`<id>` is the workspace id without `ws_`; `ws.get_info.public_base` gives the origin), a Surface is `…/<workspace>/<surface>?id=<surface id>` — give users these links, never `/workspace/<ws>` on the Zone host (that is only the signed-in user's list).

#### Source
- Service and protocol: `src/frame/aiworkspace/README.md`; operations: `src/frame/aiworkspace/schemas/operations.json`; architecture decisions: `doc/workspace/BuckyOS AiWorkspace 内核架构改进.md`.
- Client: `buckyos-websdk/src/aiworkspace_client.ts`; CLI verbs: `buckyos-websdk/cli/modules/aiws.ts`.
