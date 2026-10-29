/* Import plan → commit operations (draw.io 导入 TODO §3 提交): objects before lines (a binding needs its target),
 * parents before children, every sibling list keyed in plan order after what the parent already holds. The host
 * uploads the plan's images first and passes what it got; an image it could not get becomes its fallback shape. */

import { boxOf } from './connector.ts'
import type { Point, Rect } from './geometry.ts'
import { keysAfter, randomId } from './keys.ts'
import type { Json, PagePlan, PlanItem } from './convert.ts'

export interface UploadedAsset { object_id: string; file_name: string }

export interface BuildTarget {
  /** The Surface (or group) the page's top-level objects go into, and the world point of the page's (0, 0). */
  parentId: string
  origin: Point
  /** The last order key among the parent's children (the import goes on top), null when empty. */
  afterKey: string | null
  /** Where image data entities go (the Surface's content folder) and its last child's order key. */
  contentFolderId: string
  contentAfterKey: string | null
  /** Uploaded images by plan image key; absent or null → the item's fallback. */
  assets: Map<string, UploadedAsset | null>
  newId?: (prefix: string) => string
}

/** An operation of the commit API (`entity.create`); typed loosely here, checked by the core. */
export type ImportOp = { op: 'entity.create'; entity_id: string; type_id: string; parent_id: string; order_key: string; placement?: Record<string, number>; name?: string; payload: Record<string, Json> }

export interface BuiltPage {
  /** Groups, Blocks and image data, in an order every batch split keeps valid. */
  objects: ImportOp[]
  /** Connectors (after every object). */
  lines: ImportOp[]
  /** The new top-level entity ids (to select afterwards). */
  topLevel: string[]
  /** Plan key → entity id. */
  ids: Map<string, string>
}

/** `config` is limited to 64 KiB by the core: the kept draw.io style goes first when it does not fit. */
const CONFIG_BUDGET = 60 * 1024
const enc = new TextEncoder()
function fitConfig(config: Record<string, Json>): Record<string, Json> {
  if (enc.encode(JSON.stringify(config)).length <= CONFIG_BUDGET) return config
  const drawio = config.drawio as Record<string, Json> | undefined
  if (drawio?.style) {
    const trimmed = { ...config, drawio: { ...drawio, style: null } }
    if (enc.encode(JSON.stringify(trimmed)).length <= CONFIG_BUDGET) return trimmed
  }
  const { drawio: _drawio, ...rest } = config
  void _drawio
  return rest
}

const r2 = (v: number) => Math.round(v * 100) / 100

export function buildOperations(plan: PagePlan, target: BuildTarget): BuiltPage {
  const newId = target.newId ?? randomId
  const ids = new Map<string, string>()
  const byKey = new Map(plan.items.map((i) => [i.key, i]))
  for (const item of plan.items) ids.set(item.key, newId(item.type === 'group' ? 'g' : 'c'))
  // order keys per parent, in plan order
  const siblings = new Map<string | null, PlanItem[]>()
  for (const item of plan.items) { const list = siblings.get(item.parent) ?? []; list.push(item); siblings.set(item.parent, list) }
  const orderKey = new Map<string, string>()
  for (const [parent, list] of siblings) {
    const keys = keysAfter(parent === null ? target.afterKey : null, list.length)
    list.forEach((item, i) => orderKey.set(item.key, keys[i]))
  }
  const images = plan.items.filter((i) => i.type === 'asset' && i.image && target.assets.get(i.image.key))
  const contentKeys = keysAfter(target.contentAfterKey, images.length)
  const absOf = (key: string | null): Point => {
    if (key === null) return { x: 0, y: 0 }
    const p = byKey.get(key)
    return p ? { x: p.rect.x, y: p.rect.y } : { x: 0, y: 0 }
  }
  const placementOf = (item: PlanItem, rect: Rect): Record<string, number> => {
    const o = item.parent === null ? { x: -target.origin.x, y: -target.origin.y } : absOf(item.parent)
    return { x: r2(rect.x - o.x), y: r2(rect.y - o.y), w: Math.max(item.type === 'connector' ? 0 : 1, r2(rect.w)), h: Math.max(item.type === 'connector' ? 0 : 1, r2(rect.h)), ...(item.rotation && item.type !== 'group' ? { rotation: item.rotation } : {}) }
  }
  const objects: ImportOp[] = []
  const lines: ImportOp[] = []
  const parentIdOf = (item: PlanItem) => (item.parent === null ? target.parentId : ids.get(item.parent)!)
  let contentIndex = 0
  for (const item of plan.items) {
    const id = ids.get(item.key)!
    const base = { op: 'entity.create' as const, entity_id: id, parent_id: parentIdOf(item), order_key: orderKey.get(item.key)! }
    const common: Record<string, Json> = { ...(item.payload ?? {}), ...(item.title ? { title: item.title.slice(0, 256) } : {}), ...(item.locked ? { locked: true } : {}) }
    if (item.type === 'group') {
      objects.push({ ...base, type_id: 'buckyos.container', placement: placementOf(item, item.rect), payload: { kind: 'group', layout: { mode: 'free' }, ...common } })
      continue
    }
    if (item.type === 'connector' && item.line) {
      const line = item.line
      const { rect, flip } = boxOf(line.startPoint, line.endPoint)
      const end = (e: typeof line.start): Json => (e && ids.has(e.key) ? { entity_id: ids.get(e.key)!, anchor: { kind: 'named', id: e.anchor } } : null)
      const label = line.label && (line.label.t !== 0.5 || line.label.offset) ? { t: Math.round(line.label.t * 10000) / 10000, ...(line.label.offset ? { offset: Math.round(line.label.offset * 10) / 10 } : {}) } : null
      const payload: Record<string, Json> = {
        view: { type: 'connector', version: 1 }, start: end(line.start), end: end(line.end),
        ...(flip.h || flip.v ? { flip: { ...(flip.h ? { h: true } : {}), ...(flip.v ? { v: true } : {}) } } : {}),
        ...(line.route !== 'straight' ? { route: line.route } : {}),
        ...(line.controls.length ? { controls: line.controls.slice(0, 64) as unknown as Json } : {}),
        ...(label ? { label } : {}),
        ...common, config: fitConfig(item.config),
      }
      // two coordinate ends on one spot are not a line
      if (payload.start === null && payload.end === null && rect.w + rect.h <= 0) continue
      lines.push({ ...base, type_id: 'buckyos.cell', placement: placementOf(item, rect), payload })
      continue
    }
    const asset = item.type === 'asset' && item.image ? target.assets.get(item.image.key) : undefined
    if (item.type === 'asset' && asset) {
      const dataId = newId('d')
      // no entity name: names are unique among siblings, and two pictures may share a file name
      objects.push({ op: 'entity.create', entity_id: dataId, type_id: 'buckyos.asset-ref', parent_id: target.contentFolderId, order_key: contentKeys[contentIndex++], payload: { object_id: asset.object_id, file_name: asset.file_name.slice(0, 255) } })
      objects.push({ ...base, type_id: 'buckyos.cell', placement: placementOf(item, item.rect), payload: { view: { type: 'asset' }, source_ref: { entity_id: dataId }, ...common, config: fitConfig(item.config) } })
      continue
    }
    const shape = item.type === 'asset' ? { config: item.fallback?.config ?? item.config, title: item.fallback?.title ?? item.title } : { config: item.config, title: item.title }
    const view = item.type === 'frame' ? 'frame' : 'shape'
    objects.push({ ...base, type_id: 'buckyos.cell', placement: placementOf(item, item.rect), payload: { view: { type: view }, ...common, ...(shape.title ? { title: shape.title.slice(0, 256) } : {}), config: fitConfig(shape.config) } })
  }
  const topLevel = plan.items.filter((i) => i.parent === null).map((i) => ids.get(i.key)!).filter((id) => objects.some((o) => o.entity_id === id) || lines.some((o) => o.entity_id === id))
  return { objects, lines, topLevel, ids }
}

/** Split operations into commits of at most `size` (draw.io 导入 TODO §3: 2 000 per batch, under the core's 10 000). */
export function batches<T>(ops: T[], size = 2000): T[][] {
  const out: T[][] = []
  for (let i = 0; i < ops.length; i += size) out.push(ops.slice(i, i + size))
  return out
}
