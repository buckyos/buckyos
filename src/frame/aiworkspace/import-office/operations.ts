/* A docx plan → commit operations (Office 导入 TODO §3.2): every segment is a Block on the flow page in document
 * order — a rich text entity in the page's content folder with its Cell, or a picture's asset entity with its asset
 * Cell — and the original file last as a file Block. A slide deck goes through the draw.io importer's
 * `buildOperations` page by page instead. Batches keep each data entity before the Cell showing it and stay under
 * the core's 10 000 operations and the service's 8 MiB request body. */

import { keysAfter, randomId } from '../import-drawio/keys.ts'
import type { ImportOp, UploadedAsset } from '../import-drawio/operations.ts'
import type { Json } from '../import-drawio/convert.ts'
import type { DocPlan } from './plan.ts'

export interface DocTarget {
  /** The flow page and its content folder, and the last order keys among their children (null: empty). */
  surfaceId: string
  contentFolderId: string
  afterKey: string | null
  contentAfterKey: string | null
  /** Uploaded pictures by plan image key; absent or null → the picture is left out. */
  assets: Map<string, UploadedAsset | null>
  /** The original file, a file Block after the text (kept for "open as is" and download). */
  original?: { object_id: string; file_name: string } | null
  newId?: (prefix: string) => string
}

export interface BuiltDoc {
  ops: ImportOp[]
  /** The page's new Cells, in order. */
  blocks: string[]
  /** Pictures left out because they could not be stored. */
  missing: number
}

export function buildDocOperations(plan: DocPlan, target: DocTarget): BuiltDoc {
  const newId = target.newId ?? randomId
  const shown = plan.segments.filter((s) => s.kind === 'richtext' || target.assets.get(s.image))
  const extra = target.original ? 1 : 0
  const cellKeys = keysAfter(target.afterKey, shown.length + extra)
  const dataKeys = keysAfter(target.contentAfterKey, shown.length + extra)
  const ops: ImportOp[] = []
  const blocks: string[] = []
  const names = new Map(plan.images.map((i) => [i.key, i.name]))
  shown.forEach((segment, k) => {
    const dataId = newId('d')
    const cellId = newId('c')
    if (segment.kind === 'richtext') {
      ops.push({ op: 'entity.create', entity_id: dataId, type_id: 'buckyos.richtext', parent_id: target.contentFolderId, order_key: dataKeys[k], payload: { content: segment.content as unknown as Json } })
      ops.push({ op: 'entity.create', entity_id: cellId, type_id: 'buckyos.cell', parent_id: target.surfaceId, order_key: cellKeys[k], payload: { view: { type: 'richtext' }, source_ref: { entity_id: dataId } } })
    } else {
      const asset = target.assets.get(segment.image)!
      const payload: Record<string, Json> = { object_id: asset.object_id, file_name: (asset.file_name || names.get(segment.image) || 'image').slice(0, 255), ...(segment.pixels ? { image: segment.pixels } : {}) }
      ops.push({ op: 'entity.create', entity_id: dataId, type_id: 'buckyos.asset-ref', parent_id: target.contentFolderId, order_key: dataKeys[k], payload })
      ops.push({ op: 'entity.create', entity_id: cellId, type_id: 'buckyos.cell', parent_id: target.surfaceId, order_key: cellKeys[k], payload: { view: { type: 'asset' }, source_ref: { entity_id: dataId }, ...(segment.title ? { title: segment.title } : {}) } })
    }
    blocks.push(cellId)
  })
  if (target.original) {
    const dataId = newId('d')
    const cellId = newId('c')
    ops.push({ op: 'entity.create', entity_id: dataId, type_id: 'buckyos.asset-ref', parent_id: target.contentFolderId, order_key: dataKeys[shown.length], payload: { object_id: target.original.object_id, file_name: target.original.file_name.slice(0, 255) } })
    ops.push({ op: 'entity.create', entity_id: cellId, type_id: 'buckyos.cell', parent_id: target.surfaceId, order_key: cellKeys[shown.length], payload: { view: { type: 'asset' }, source_ref: { entity_id: dataId } } })
    blocks.push(cellId)
  }
  return { ops, blocks, missing: plan.segments.length - shown.length }
}
