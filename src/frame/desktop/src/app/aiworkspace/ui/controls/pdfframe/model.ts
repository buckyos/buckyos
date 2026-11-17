/* The PDF Frame on the Desktop (打印 TODO §3.3, T3): reading a frame from the outline (its page shape, box, bands and
 * content area; the shared rules are `@controls/pdfframe`), finding the frame a Block of a header or footer belongs to,
 * and the writes that change a frame's shape (its config and, keeping its scale, its size) in one commit. */

import { bandOf, frameAreas, frameBox, frameRatio, pdfFrameConfig, PDF_FRAME, rescaledWidth, type FrameBand, type PdfFrameConfig } from '@controls/pdfframe'
import { num } from '@controls/placement'
import type { FrameGeometry } from '@print/page.ts'
import type { EntityEnvelope, Json, Operation } from '../../../api/types'
import type { OutlineModel } from '../../../state/outline'
import type { WorkspaceStore } from '../../../state/store'

export { PDF_FRAME }

export function isFrameEntity(e: Pick<EntityEnvelope, 'type_id' | 'kind'> | undefined | null): boolean {
  return !!e && e.type_id === 'buckyos.container' && e.kind === PDF_FRAME
}

/** A frame as the outline has it: its config, box (canvas units) and areas relative to its top-left. */
export function frameLayout(e: Pick<EntityEnvelope, 'config' | 'placement'>) {
  const config = pdfFrameConfig(e.config)
  const box = frameBox(num(e.placement?.w), config)
  return { config, box, areas: frameAreas(box, config) }
}

/** What printing reads of a frame (`@print/page.ts`): its box and its content area. */
export function frameGeometry(e: Pick<EntityEnvelope, 'config' | 'placement'>): FrameGeometry {
  const { box, areas } = frameLayout(e)
  return { w: box.w, h: box.h, content: areas.content }
}

/** Every live PDF Frame among `entities` (an outline), with its geometry and the Surface it is drawn on. */
export function framesIn(entities: readonly EntityEnvelope[]): { id: string; title: string | null; geometry: FrameGeometry; surfaceId: string | null }[] {
  const byId = new Map(entities.map((e) => [e.entity_id, e]))
  const surface = (e: EntityEnvelope): string | null => {
    let cur = e.parent_id
    for (let i = 0; cur && i < 4096; i++) {
      const parent = byId.get(cur)
      if (!parent || parent.deleted) return null
      if (parent.type_id === 'buckyos.container' && parent.kind === 'surface') return parent.entity_id
      cur = parent.parent_id
    }
    return null
  }
  return entities.filter((e) => !e.deleted && isFrameEntity(e)).map((e) => ({ id: e.entity_id, title: e.title || null, geometry: frameGeometry(e), surfaceId: surface(e) }))
}

/** The frame's bands, by slot (the first of each, as the layout takes them). */
export function bandsOf(outline: Pick<OutlineModel, 'childrenOf'>, frameId: string): Partial<Record<FrameBand, EntityEnvelope>> {
  const out: Partial<Record<FrameBand, EntityEnvelope>> = {}
  for (const child of outline.childrenOf(frameId)) {
    const band = bandOf(child)
    if (band && !out[band]) out[band] = child
  }
  return out
}

/** The frame and band a Block of a header or footer sits in (through the groups inside the band), or null. */
export function frameOfBlock(outline: Pick<OutlineModel, 'get'>, id: string): { frame: EntityEnvelope; band: FrameBand } | null {
  let cur = outline.get(id)?.parent_id
  let child: EntityEnvelope | undefined
  for (let i = 0; cur && i < 64; i++) {
    const e = outline.get(cur)
    if (!e) return null
    if (isFrameEntity(e)) {
      const band = child ? bandOf(child) : null
      return band ? { frame: e, band } : null
    }
    if (e.type_id !== 'buckyos.container' || e.kind !== 'group') return null
    child = e
    cur = e.parent_id
  }
  return null
}

/** Is the entity a PDF Frame or anything inside one (its bands and their Blocks)? Printing leaves them out of the content. */
export function partOfPdfFrame(outline: Pick<OutlineModel, 'get'>, id: string): boolean {
  let cur: string | undefined = id
  for (let i = 0; cur && i < 4096; i++) {
    const e = outline.get(cur)
    if (!e) return false
    if (isFrameEntity(e)) return true
    cur = e.parent_id
  }
  return false
}

/** The Surface a Block is drawn on (through its groups and controls), or null. */
export function surfaceOf(outline: Pick<OutlineModel, 'get'>, id: string): EntityEnvelope | null {
  let cur = outline.get(id)?.parent_id
  for (let i = 0; cur && i < 4096; i++) {
    const e = outline.get(cur)
    if (!e) return null
    if (e.type_id === 'buckyos.container' && e.kind === 'surface') return e
    cur = e.parent_id
  }
  return null
}

const SHAPE_KEYS = ['page', 'orientation', 'custom_w', 'custom_h'] as const

/** One commit's operations for `patch` of the frame's config. A new page shape keeps the frame's scale (its width over
 * its natural one) and gives it the box of that shape. */
export async function frameConfigOps(store: WorkspaceStore, frameId: string, patch: Partial<Record<keyof PdfFrameConfig, Json | null>>): Promise<Operation[]> {
  const read = await store.session.read<{ payload: { config?: Record<string, Json> }; key_revs: Record<string, number> }>(frameId)
  const stored: Record<string, Json> = { ...(read.content.payload.config ?? {}) }
  const before = pdfFrameConfig(stored)
  for (const [k, v] of Object.entries(patch)) { if (v === null || v === undefined) delete stored[k]; else stored[k] = v }
  const after = pdfFrameConfig(stored)
  const ops: Operation[] = [{ op: 'entity.set_keys', entity_id: frameId, keys: [{ key: 'config', value: { ...after }, expect: { rev: read.content.key_revs.config ?? 0 } }] }]
  const frame = store.outline.get(frameId)
  const p = frame?.placement
  const reshaped = SHAPE_KEYS.some((k) => before[k] !== after[k])
  if (frame && p && reshaped && frameRatio(before) !== frameRatio(after)) {
    const was = frameBox(num(p.w), before).w
    const w = Math.max(1, rescaledWidth(was, before, after))
    ops.push({ op: 'tree.place', entity_id: frameId, placement: { x: num(p.x) ?? 0, y: num(p.y) ?? 0, w, h: Math.max(1, Math.round(w / frameRatio(after))) } })
    // the header's and footer's Blocks keep their side: by the left edge, the middle or the right edge, whichever third
    // of the old width their centre was in
    for (const band of Object.values(bandsOf(store.outline, frameId))) {
      for (const child of store.outline.childrenOf(band.entity_id)) {
        const cp = child.placement
        const x = num(cp?.x), cw = num(cp?.w)
        if (!cp || x === undefined || cw === undefined) continue
        const centre = x + cw / 2
        const moved = Math.round(centre < was / 3 ? x : centre > (was * 2) / 3 ? x + w - was : x + (w - was) / 2)
        if (moved !== x) ops.push({ op: 'tree.place', entity_id: child.entity_id, placement: { ...cp, x: moved } })
      }
    }
  }
  return ops
}
