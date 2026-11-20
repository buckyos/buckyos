#### Create a workspace, table and canvas
```ts
// a Deno script using the websdk; run it with: deno run -A build.ts
import { buckyos } from "buckyos"            // or the local dist/node.mjs
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

Check `buckyos aiws cat /data/销售 --workspace <ws>`, `ls /surfaces/看板` and `info`. A canvas owns data under `canvas-content` (deleted with its last Block); data under `data` is shared. Nested folders under `surfaces` hold canvases and folders. Move the canvas with `mv` and keep its content folder under `canvas-content`. Grants on folders cover children; only empty folders can be deleted.

#### Data types and imports
- Tables: `text`, `number`, `decimal` (scale), `currency` (ISO 4217 code, default scale 2), `date`, `datetime`, `boolean`, `select`, `multi_select`, `object_ref` and `json` (object ≤16 KiB, depth ≤8; emptiness filters). Write `{"amount":"12.50","currency":"CNY"}` or a bare default-currency amount. A `submitter:true` text field is service-filled, one record per principal (`SUBMITTER_EXISTS` gives `data.record_id`); append-only users edit/delete their own. `table field-add … --default <display value>` or `table.update_field` sets typed defaults on new records, except `submitter`, `unique`, `object_ref`. Use `null` to remove a default, `backfill` for existing rows. SQL `json_each(v."<picks>#raw")` expands a multi-select vote.
- CSV/TSV/XLSX/SQLite: `import_table <file> --workspace <ws> --dry-run`, review inferred types, sample rows and conversions, then repeat without `--dry-run`. Use `--sheet`/`--all-sheets` or `--table`, header/date/timezone/locale options as needed.
- DOCX/PPTX: `import_office <file> --workspace <ws> --dry-run`, check simplifications, then import. DOCX makes a flow page, PPTX a canvas and presentation path. For unsupported charts, SmartArt, EMF/WMF, formulas, or older/ODF files use Desktop’s Office import (LibreOffice).
- `buckyos.richtext` stores a doc tree; use core `markdown.rs` for conversion. For `buckyos.asset-ref` upload via `buckyos bucket put obj://bucket/<bucket>/<key> <file>`, then `asset.import_object {workspace_id,obj_id}` and use its `object_id`. Optional `poster` is an imported PNG/JPEG/WebP/GIF; dimensions, duration and page count are metadata. `buckyos.dir` members map relative paths to `{kind:"file",object_id}` or `{kind:"folder"}`. Update with `dir.set_member|remove_member|replace`; removing a folder removes its subtree.

#### Built-in controls and records
Use `buckyos aiws controls list` / `describe <kind>` for the renderer contract. Create mind maps with `buckyos aiws import plan.md --workspace <ws> --parent <surface> --at 100,80` (nested Markdown or OPML). Read back with `buckyos aiws export <control> --workspace <ws> [--format opml] [--out plan.md]`. For operations use `buckyos.cell` with `view.type: mindnode`, title text, parent node and automatic layout (numeric width/height for non-text nodes). Ink Cells use `view:{type:"ink",version:1}` and the Desktop's `ink/model.ts` encoder.

To-dos and calendar items are records. Use `buckyos aiws todo list|add|done|undone|update|move|rm` and `buckyos aiws cal list|add|update|rm|skip|postpone` for the user's `schedule` workspace (`ws.ensure`); return the printed links. Else `buckyos aiws place todo|calendar --workspace <ws> --surface <canvas> [--bind "name=id[,id];…"]`. The service creates a missing table, binds an unambiguous match or returns `NEEDS_UI`. For manual writes use `table.*` and `schemas/standard-tables.md` (`buckyos.todo@1`, `buckyos.calendar@1`).

#### Composite instances and wish Blocks
A composite's `def_ref` is immutable. Child Blocks carry their `slot`; layout is automatic except in fixed layouts.
```json
[{"op":"entity.create","entity_id":"board-1","type_id":"buckyos.container","parent_id":"<surface>","order_key":"m","placement":{"x":40,"y":40,"w":"auto","h":"auto"},"payload":{"kind":"composite","def_ref":{"entity_id":"<def>"},"config":{"title":"Today"}}},
 {"op":"entity.create","entity_id":"card-1","type_id":"buckyos.cell","parent_id":"board-1","order_key":"a","placement":{"x":"auto","y":"auto","w":240,"h":120},"payload":{"view":{"type":"html"},"def_ref":{"entity_id":"<card def>"},"slot":"cards","bindings":{"task":{"entity_id":"<record>"}}}}]
```
Move a Block in/out with `tree.move` and `slot` (name or null). Clone a subtree with `tree.clone {source_id,parent_id,placement?,slot?,id_seed}`. It preserves shared data/definitions, duplicates owned data and remaps inner connectors. Fresh `id_seed` ≤64 bytes. `cloned[].new_ids` maps old to new; IDs are `c` + the first 26 hex digits of SHA-256 of `<id_seed>\n<old id>`.

Create a `buckyos.wish` with `prompt`, `executor:"xllm"` and `output:{container_id,surface_id,name}`. Only the service writes `history`. `proc.start` with `program:"wish.xllm@1"` takes `wish_id`, `stage` (`analyze|code|run`) and `execution.mode`. `local` returns a claim token; `hosted` dispatches to the owner's Agent. See `aiworkspace_run` for the stage contract and rollback. Templates keep `program` and omit its history, while clones and exports keep the history.
