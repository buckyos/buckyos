---
name: aiworkspace_template_dev
description: Build and maintain AI Workspaces from the command line — create a workspace, add tables, documents and Blocks by committing operations, export and import packages, reference data across workspaces, run wish blocks, publish data by path and insert published links.
requires: the buckyos command (agent container: /usr/local/bin/buckyos; host: $BUCKYOS_ROOT/bin/buckyos) signed in; Deno for scripts that use the websdk directly; the workspace must grant you the capability each step needs (structure/update to write, manage to share or publish, run to start a wish)
---
#### When to use
- You are asked to set up a workspace for someone: tables with fields and rows, a document, a canvas with Blocks bound to them, a wish block that computes results.
- You need to move content between workspaces or Zones: export a share package, import it elsewhere, or link one workspace's published table into another so it follows updates.
- You already hold a run of a wish block to execute: that is `aiworkspace_run`, not this manual.

#### Before you start
- `buckyos aiws --help`: the read verbs (`info`, `read`, `query`) and the run verbs. Everything else is one kRPC method of the service (`/kapi/aiworkspace`), called through the websdk: `getAiWorkspaceClient()` in `buckyos-websdk/src/aiworkspace_client.ts` (`info`, `outline`, `read`, `query`, `resolve`, `prepare`, `commit`, `run*`, and `call(method, params)` for the rest).
- The protocol is in `src/frame/aiworkspace/README.md` (Protocol: the method table with its stability level, the operation catalogue `schemas/operations.json`, selectors, error codes). Methods marked `stable` are the ones to build on.
- A write is a Commit: `doc.commit { protocol_version: "1.0", workspace_id, epoch, idempotency_key, session_id, operations: [...] }`. `epoch` comes from `ws.get_info`; keep your `idempotency_key` — resending the same key never writes twice (`doc.get_submission` tells you what happened).

#### Shortest path (a workspace with a table and a canvas)
```bash
# a Deno script using the websdk; run it with: deno run -A build.ts
import { buckyos } from "buckyos"            # or the local dist/node.mjs
const aiws = buckyos.getAiWorkspaceClient()
const ws = (await aiws.call("ws.create", { title: "季度经营" })).workspace_id
const { epoch } = await aiws.info(ws)
await aiws.commit(ws, { protocol_version: "1.0", workspace_id: ws, epoch, idempotency_key: "build/1", session_id: "s1", operations: [
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
- **Entity types**: `buckyos.table-source` (fields + records), `buckyos.richtext` (`content: { type: "doc", content: [blocks] }`, Markdown converts with the core's `markdown.rs`), `buckyos.record`, `buckyos.asset-ref` (upload first: `asset.begin_upload` → `PUT /kapi/aiworkspace/upload/<id>` → `asset.finish_upload`), `buckyos.dir`, `buckyos.wish`, `buckyos.cell` (a Block: `view.type` from the Renderer catalogue, `source_ref` to the data it shows).
- **Reading**: `doc.outline` (the trees), `doc.read { entity_id, selector? }`, `doc.query { source_id | view_id, filter?, sorts?, limit, cursor }`; `doc.resolve { reference | path }` turns a path such as `/data/销售` into the reference object `{ workspace_id, entity_id, mode: live | fixed, version? }` — keep references, not names (D10).
- **Export / import**: `doc.export { mode: "share" | "personal_backup", self_contained? }` returns an export id; download `GET /kapi/aiworkspace/export/<ws>/<id>` (bearer: your session). `share` is content only (no collaboration history); `personal_backup` carries it. Import: `asset.begin_upload` the package, then `ws.import { upload_id, title }` (or `ws.fork` for a copy inside this Zone).
- **Across workspaces** (D26): the publisher runs `ws.publish { entity_id }` (manage) and gets a link `https://aiws-<id>.<zonehost>/<workspace>/<name>` (or `/kapi/aiworkspace/pub/<ws>/<name>`); the consumer creates an entity with `payload.source = { kind: "workspace", workspace_id, entity_id, mode: "live" }` (another Zone: add `zone` and `url`) and refreshes it: `proc.start { program: "source.refresh@1", params: { entity_id }, idempotency_key }` → `proc.get` until `succeeded`. Any CSV / JSON / Markdown URL works the same with `source: { kind: "url", url }` (`refresh: { interval_s }` schedules it). A sourced entity is read-only (`SOURCE_READ_ONLY`): refresh, do not edit.
- **Wish blocks**: create a `buckyos.wish` (`prompt`, `executor: "xllm"`, `output: { container_id, surface_id, name }`), then `proc.start { program: "wish.xllm@1", params: { wish_id, stage: "analyze" | "execute", execution: { mode: "local" | "hosted" } } }`; `proc.get` reports the candidate and a preview; `proc.apply { run_id, plan_digest, session_id }` writes it. In `hosted` mode the owner's Agent executes (`aiworkspace_run`).
- **Sharing with people**: `ws.grant { subject, capabilities: ["read", ...], scope_entity_id? }` (manage); `ws.list_grants`; entries for visitors are `ws.create_entry` / `ws.list_entries`.

#### When it fails
- `rejected` with `errors[]`: read `code` / `sub_code` (`ID_CONFLICT`, `NAME_CONFLICT`, `SOURCE_READ_ONLY`, `LOCK_REQUIRED`, `QUOTA_EXCEEDED`…), never the text. A conflict answers the current revision in `conflicts[]`: read again, resend with the new `expect`.
- `PERMISSION_DENIED`: `ws.get_info.capabilities` lists what you may do; the owner grants more with `ws.grant`.
- `epoch` mismatch after an import or restore: call `ws.get_info` again.
- An unknown result (the request timed out): `doc.get_submission { epoch, idempotency_key }` — `accepted` means it is in; otherwise resend the same key.

#### Check the result
`buckyos aiws info --workspace <ws>` (head sequence, capabilities, usage), `buckyos aiws read <entity> --workspace <ws>`, `buckyos aiws query --workspace <ws> --source <table>`; the Desktop opens it at `https://<zonehost>/workspace/<ws>`.

#### Source
- Service and protocol: `src/frame/aiworkspace/README.md`; operations: `src/frame/aiworkspace/schemas/operations.json`; architecture decisions: `doc/workspace/BuckyOS AiWorkspace 内核架构改进.md`.
- Client: `buckyos-websdk/src/aiworkspace_client.ts`; CLI verbs: `buckyos-websdk/cli/modules/aiws.ts`.
