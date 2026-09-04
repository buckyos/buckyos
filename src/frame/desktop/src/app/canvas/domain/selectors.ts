import type {
  CanvasBlock,
  CanvasDocument,
  Camera,
  ContextRef,
  GeneratedStatus,
  Rect,
  TableBlock,
  TableBlockContent,
  WishBlock,
} from './types'

export function refKey(ref: ContextRef): string {
  if (ref.kind === 'block') return ref.blockId
  const r = ref.range
  return `${ref.blockId}:r${r.rowStart}-${r.rowEnd}:c${r.colStart}-${r.colEnd}`
}

export function refBlockId(key: string): string {
  return key.split(':')[0]
}

export function sheetBlocks(doc: CanvasDocument, sheetId: string): CanvasBlock[] {
  const sheet = doc.sheets.find((s) => s.id === sheetId)
  if (!sheet) return []
  return sheet.blockIds
    .map((id) => doc.blocks[id])
    .filter((b): b is CanvasBlock => Boolean(b))
}

export function activeSheet(doc: CanvasDocument) {
  return doc.sheets.find((s) => s.id === doc.activeSheetId) ?? doc.sheets[0]
}

export function blocksOfType<T extends CanvasBlock['type']>(
  doc: CanvasDocument,
  type: T,
  sheetId?: string,
): Array<Extract<CanvasBlock, { type: T }>> {
  return Object.values(doc.blocks).filter(
    (b): b is Extract<CanvasBlock, { type: T }> =>
      b.type === type && (!sheetId || b.sheetId === sheetId),
  )
}

export function tableBlocks(doc: CanvasDocument, sheetId?: string): TableBlock[] {
  return blocksOfType(doc, 'table', sheetId)
}

export function wishBlocks(doc: CanvasDocument, sheetId?: string): WishBlock[] {
  return blocksOfType(doc, 'wish', sheetId)
}

/** Derived freshness of an AI-generated block (PRD FR-BIND-001). */
export function generatedStatus(doc: CanvasDocument, block: CanvasBlock): GeneratedStatus | 'never_run' | null {
  const meta = block.generated
  if (!meta || meta.detached) return null
  let status: GeneratedStatus = 'fresh'
  for (const src of meta.sourceRevisions) {
    const srcBlock = doc.blocks[refBlockId(src.refKey)]
    if (!srcBlock) return 'broken'
    if (srcBlock.dataRevision !== src.revision) status = 'stale'
  }
  return status
}

export function recomputeGeneratedStatuses(doc: CanvasDocument): CanvasDocument {
  let changed = false
  const blocks = { ...doc.blocks }
  for (const block of Object.values(blocks)) {
    if (!block.generated || block.generated.detached) continue
    const next = generatedStatus(doc, block)
    if (next && next !== 'never_run' && next !== block.generated.status) {
      blocks[block.id] = { ...block, generated: { ...block.generated, status: next } } as CanvasBlock
      changed = true
    }
  }
  return changed ? { ...doc, blocks } : doc
}

export function wishStatus(doc: CanvasDocument, wish: WishBlock): GeneratedStatus | 'never_run' {
  if (wish.content.generatedGroupIds.length === 0) return 'never_run'
  let worst: GeneratedStatus = 'fresh'
  for (const gid of wish.content.generatedGroupIds) {
    const group = doc.blocks[gid]
    if (!group) continue
    const s = generatedStatus(doc, group)
    if (s === 'broken') return 'broken'
    if (s === 'stale') worst = 'stale'
  }
  return worst
}

/** Wish that generated the given block (direct or via group), if any. */
export function generatingWishId(doc: CanvasDocument, blockId: string): string | null {
  const block = doc.blocks[blockId]
  if (!block?.generated || block.generated.detached) return null
  return block.generated.wishBlockId
}

/**
 * Cycle detection (PRD FR-BIND-003): adding `refBlockId` as a source of `wishId`
 * creates a cycle if `refBlockId` (transitively) depends on `wishId`'s output.
 */
export function wouldCreateCycle(doc: CanvasDocument, wishId: string, refBlockIdValue: string): boolean {
  const visited = new Set<string>()
  const stack = [refBlockIdValue]
  while (stack.length) {
    const id = stack.pop()!
    if (visited.has(id)) continue
    visited.add(id)
    const producer = generatingWishId(doc, id)
    if (!producer) continue
    if (producer === wishId) return true
    const wish = doc.blocks[producer]
    if (wish?.type === 'wish') {
      for (const ref of wish.content.contextRefs) stack.push(ref.blockId)
    }
  }
  return false
}

export function unionRect(rects: Rect[]): Rect | null {
  if (rects.length === 0) return null
  let x1 = Infinity
  let y1 = Infinity
  let x2 = -Infinity
  let y2 = -Infinity
  for (const r of rects) {
    x1 = Math.min(x1, r.x)
    y1 = Math.min(y1, r.y)
    x2 = Math.max(x2, r.x + r.width)
    y2 = Math.max(y2, r.y + r.height)
  }
  return { x: x1, y: y1, width: x2 - x1, height: y2 - y1 }
}

export function rectContains(outer: Rect, inner: Rect): boolean {
  return (
    inner.x >= outer.x &&
    inner.y >= outer.y &&
    inner.x + inner.width <= outer.x + outer.width &&
    inner.y + inner.height <= outer.y + outer.height
  )
}

export function rectsIntersect(a: Rect, b: Rect): boolean {
  return !(
    a.x + a.width < b.x ||
    b.x + b.width < a.x ||
    a.y + a.height < b.y ||
    b.y + b.height < a.y
  )
}

export function cameraToFit(rect: Rect, viewport: { width: number; height: number }, padding = 60): Camera {
  const zoomX = (viewport.width - padding * 2) / Math.max(rect.width, 1)
  const zoomY = (viewport.height - padding * 2) / Math.max(rect.height, 1)
  const zoom = Math.min(Math.max(Math.min(zoomX, zoomY), 0.1), 2)
  return {
    zoom,
    x: viewport.width / 2 - (rect.x + rect.width / 2) * zoom,
    y: viewport.height / 2 - (rect.y + rect.height / 2) * zoom,
  }
}

/** Children of a frame = blocks fully contained inside its rect on the same sheet. */
export function frameChildren(doc: CanvasDocument, frame: CanvasBlock): string[] {
  return sheetBlocks(doc, frame.sheetId)
    .filter((b) => b.id !== frame.id && b.type !== 'frame' && rectContains(frame.rect, b.rect))
    .map((b) => b.id)
}

/** Expand a set of ids to include children that move together (groups, frames). */
export function expandMoveSet(doc: CanvasDocument, ids: string[], includeFrameChildren = true): string[] {
  const out = new Set<string>()
  for (const id of ids) {
    const block = doc.blocks[id]
    if (!block) continue
    out.add(id)
    if (block.type === 'group') {
      for (const child of block.content.childBlockIds) out.add(child)
    }
    if (block.type === 'frame' && includeFrameChildren && block.content.moveChildren) {
      for (const child of frameChildren(doc, block)) out.add(child)
    }
  }
  return [...out]
}

export function tableRangeLabel(block: TableBlock, ref: ContextRef): string {
  const name = block.title ?? '表格'
  if (ref.kind === 'block') {
    return `${name}!全部 (${block.content.rows.length}行×${block.content.columns.length}列)`
  }
  const r = ref.range
  return `${name}!${colLetter(r.colStart)}${r.rowStart + 1}:${colLetter(r.colEnd)}${r.rowEnd + 1}`
}

export function colLetter(index: number): string {
  let s = ''
  let n = index
  do {
    s = String.fromCharCode(65 + (n % 26)) + s
    n = Math.floor(n / 26) - 1
  } while (n >= 0)
  return s
}

export function cellDisplay(cell: import('./types').TableCell | undefined): string {
  if (!cell) return ''
  if (cell.displayValue !== undefined) return cell.displayValue
  if (cell.kind === 'ai') return cell.value == null ? '' : String(cell.value)
  if (cell.value === null || cell.value === undefined) return ''
  if (typeof cell.value === 'number') return formatNumber(cell.value)
  if (typeof cell.value === 'boolean') return cell.value ? 'TRUE' : 'FALSE'
  return String(cell.value)
}

export function cellNumber(cell: import('./types').TableCell | undefined): number | null {
  if (!cell) return null
  const v = cell.kind === 'ai' ? cell.value : cell.value
  if (typeof v === 'number' && Number.isFinite(v)) return v
  if (typeof v === 'string') {
    const n = Number(v.replace(/[,¥$%\s]/g, ''))
    return Number.isFinite(n) && v.trim() !== '' ? n : null
  }
  return null
}

export function formatNumber(n: number, format: 'plain' | 'percent' | 'currency' = 'plain'): string {
  if (format === 'percent') return `${(n * 100).toFixed(1)}%`
  if (format === 'currency') return `¥${n.toLocaleString('zh-CN', { maximumFractionDigits: 0 })}`
  if (Number.isInteger(n)) return n.toLocaleString('zh-CN')
  return n.toLocaleString('zh-CN', { maximumFractionDigits: 2 })
}

export function tableToMatrix(content: TableBlockContent): string[][] {
  const header = content.columns.map((c) => c.name)
  const rows = content.rows.map((r) => content.columns.map((c) => cellDisplay(r.cells[c.id])))
  return [header, ...rows]
}

export function tableToRecords(content: TableBlockContent): Record<string, import('./types').CellPrimitive>[] {
  return content.rows.map((r) => {
    const rec: Record<string, import('./types').CellPrimitive> = {}
    for (const c of content.columns) {
      const cell = r.cells[c.id]
      rec[c.name] = cell ? (cell.kind === 'ai' ? (cell.value ?? cell.displayValue ?? null) : cell.value) : null
    }
    return rec
  })
}
