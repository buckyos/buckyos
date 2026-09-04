/* ── Block / document factories ── */

import { newId, nowIso } from './ids'
import type {
  CanvasBlock,
  CanvasBlockOf,
  CanvasDocument,
  CanvasSheet,
  CellValueType,
  ChartBlockContent,
  ImageBlockContent,
  MetricBlockContent,
  Rect,
  TableBlockContent,
  TableCell,
  TableColumn,
  TableRow,
  TextBlockContent,
  VideoBlockContent,
  WishBlockContent,
} from './types'
import { CANVAS_SCHEMA_VERSION } from './types'

export function createSheet(name: string, order: number): CanvasSheet {
  return { id: newId('sheet'), name, order, blockIds: [], camera: { x: 80, y: 80, zoom: 1 } }
}

export function createEmptyDocument(title = '未命名画布'): CanvasDocument {
  const sheet = createSheet('Sheet 1', 0)
  const ts = nowIso()
  return {
    schemaVersion: CANVAS_SCHEMA_VERSION,
    id: newId('canvas'),
    title,
    revision: 0,
    activeSheetId: sheet.id,
    sheets: [sheet],
    blocks: {},
    bindings: [],
    presentationPaths: [],
    comments: [],
    createdAt: ts,
    updatedAt: ts,
    metadata: {},
  }
}

interface BaseInit {
  sheetId: string
  rect: Rect
  title?: string
  id?: string
  zIndex?: number
}

function base(init: BaseInit) {
  const ts = nowIso()
  return {
    id: init.id ?? newId('blk'),
    sheetId: init.sheetId,
    title: init.title,
    rect: init.rect,
    zIndex: init.zIndex ?? 1,
    locked: false,
    contentRevision: 0,
    dataRevision: 0,
    createdAt: ts,
    updatedAt: ts,
  }
}

export function createTextBlock(init: BaseInit & { text?: string }): CanvasBlockOf<'text'> {
  const content: TextBlockContent = { text: init.text ?? '', format: 'markdown' }
  return { ...base(init), type: 'text', content }
}

export function createWishBlock(
  init: BaseInit & { prompt?: string; contextRefs?: WishBlockContent['contextRefs'] },
): CanvasBlockOf<'wish'> {
  const content: WishBlockContent = {
    prompt: init.prompt ?? '',
    contextRefs: init.contextRefs ?? [],
    outputPreference: 'auto',
    refreshPolicy: { mode: 'notify_on_change' },
    state: 'idle',
    generatedGroupIds: [],
    runHistory: [],
  }
  return { ...base({ title: '许愿格', ...init }), type: 'wish', content }
}

export function createMetricBlock(init: BaseInit & { content: MetricBlockContent }): CanvasBlockOf<'metric'> {
  return { ...base(init), type: 'metric', content: init.content }
}

export function createChartBlock(init: BaseInit & { content: ChartBlockContent }): CanvasBlockOf<'chart'> {
  return { ...base(init), type: 'chart', content: init.content }
}

export function createImageBlock(init: BaseInit & { content?: Partial<ImageBlockContent> }): CanvasBlockOf<'image'> {
  const content: ImageBlockContent = { src: '', fit: 'contain', ...init.content }
  return { ...base({ title: '图片', ...init }), type: 'image', content }
}

export function createVideoBlock(init: BaseInit & { content?: VideoBlockContent }): CanvasBlockOf<'video'> {
  return { ...base({ title: '视频', ...init }), type: 'video', content: init.content ?? {} }
}

/** Block height that shows an image of the given aspect ratio at `width` without letterboxing (header included). */
export function imageBlockHeight(width: number, naturalWidth?: number, naturalHeight?: number, header = 28): number {
  if (!naturalWidth || !naturalHeight) return Math.round(width * 0.66) + header
  return Math.max(60, Math.round((width * naturalHeight) / naturalWidth)) + header
}

export function createFrameBlock(init: BaseInit & { color?: string }): CanvasBlockOf<'frame'> {
  return {
    ...base({ title: '框架', zIndex: 0, ...init }),
    type: 'frame',
    content: { color: init.color, moveChildren: true },
  }
}

export function createGroupBlock(
  init: BaseInit & { childBlockIds: string[]; summary?: string },
): CanvasBlockOf<'group'> {
  return {
    ...base({ zIndex: 0, ...init }),
    type: 'group',
    content: { childBlockIds: init.childBlockIds, summary: init.summary },
  }
}

export function createTableBlock(
  init: BaseInit & { content?: TableBlockContent; rows?: number; cols?: number },
): CanvasBlockOf<'table'> {
  const content = init.content ?? emptyTableContent(init.rows ?? 10, init.cols ?? 5)
  return { ...base({ title: '表格', ...init }), type: 'table', content }
}

export function emptyTableContent(rows: number, cols: number): TableBlockContent {
  const columns: TableColumn[] = Array.from({ length: cols }, (_, i) => ({
    id: newId('col'),
    name: `列${i + 1}`,
    width: 120,
  }))
  const rowList: TableRow[] = Array.from({ length: rows }, () => ({
    id: newId('row'),
    cells: {},
  }))
  return { columns, rows: rowList, source: { kind: 'manual' } }
}

export function valueCell(raw: string | number | boolean | null): TableCell {
  return parseCellValue(raw)
}

const NUM_RE = /^-?\s*[¥$€]?\s*-?\d{1,3}(,\d{3})*(\.\d+)?%?$|^-?\d+(\.\d+)?%?$|^-?\d*\.\d+%?$/
const DATE_RE = /^(\d{4})[-/.年](\d{1,2})[-/.月](\d{1,2})日?(?:[ T](\d{1,2}):(\d{2})(?::(\d{2}))?)?$/

/** Parse a raw import value into a typed TableCell. Never mutates the source text (PRD FR-TABLE-003). */
export function parseCellValue(raw: string | number | boolean | null | undefined): TableCell {
  if (raw === null || raw === undefined || raw === '') {
    return { kind: 'value', value: null, valueType: 'null' }
  }
  if (typeof raw === 'number') {
    return { kind: 'value', value: raw, valueType: 'number' }
  }
  if (typeof raw === 'boolean') {
    return { kind: 'value', value: raw, valueType: 'boolean' }
  }
  const text = String(raw)
  const trimmed = text.trim()
  const lower = trimmed.toLowerCase()
  if (lower === 'true' || lower === 'false') {
    return { kind: 'value', value: lower === 'true', valueType: 'boolean', displayValue: trimmed }
  }
  if (NUM_RE.test(trimmed)) {
    const isPercent = trimmed.endsWith('%')
    const n = Number(trimmed.replace(/[,¥$€%\s]/g, ''))
    if (Number.isFinite(n)) {
      return {
        kind: 'value',
        value: isPercent ? n / 100 : n,
        valueType: 'number',
        displayValue: trimmed,
      }
    }
  }
  const dm = DATE_RE.exec(trimmed)
  if (dm) {
    const y = Number(dm[1])
    const m = Number(dm[2])
    const d = Number(dm[3])
    const valid = m >= 1 && m <= 12 && d >= 1 && d <= 31
    if (valid) {
      const iso = `${y}-${String(m).padStart(2, '0')}-${String(d).padStart(2, '0')}`
      return { kind: 'value', value: iso, valueType: 'date', displayValue: trimmed }
    }
    return { kind: 'value', value: trimmed, valueType: 'string', warning: '日期解析不确定，已保留原文' }
  }
  return { kind: 'value', value: text, valueType: 'string' }
}

export function inferColumnType(cells: TableCell[]): CellValueType {
  const counts: Record<CellValueType, number> = { string: 0, number: 0, date: 0, boolean: 0, null: 0 }
  for (const c of cells) {
    if (c.kind === 'value') counts[c.valueType] += 1
  }
  const nonNull = cells.length - counts.null
  if (nonNull === 0) return 'string'
  const best = (['number', 'date', 'boolean', 'string'] as CellValueType[]).reduce((a, b) =>
    counts[b] > counts[a] ? b : a,
  )
  return counts[best] / nonNull >= 0.8 ? best : 'string'
}

/** Detect whether the first row looks like a header (mostly non-numeric, unique). */
export function looksLikeHeader(matrix: string[][]): boolean {
  if (matrix.length < 2) return true
  const first = matrix[0]
  const nonEmpty = first.filter((v) => v.trim() !== '')
  if (nonEmpty.length === 0) return false
  const numericInHeader = nonEmpty.filter((v) => NUM_RE.test(v.trim())).length
  const numericInSecond = matrix[1].filter((v) => NUM_RE.test(String(v).trim())).length
  return numericInHeader === 0 && (numericInSecond > 0 || new Set(nonEmpty).size === nonEmpty.length)
}

export function tableContentFromMatrix(
  matrix: Array<Array<string | number | boolean | null>>,
  options: { hasHeader: boolean; source?: TableBlockContent['source'] },
): TableBlockContent {
  const width = Math.max(0, ...matrix.map((r) => r.length))
  const headerRow = options.hasHeader ? matrix[0] ?? [] : []
  const dataRows = options.hasHeader ? matrix.slice(1) : matrix
  const columns: TableColumn[] = Array.from({ length: width }, (_, i) => ({
    id: newId('col'),
    name: options.hasHeader ? String(headerRow[i] ?? '').trim() || `列${i + 1}` : `列${i + 1}`,
    width: 120,
  }))
  const rows: TableRow[] = dataRows.map((r) => {
    const cells: Record<string, TableCell> = {}
    columns.forEach((c, i) => {
      cells[c.id] = parseCellValue(r[i] as string | number | boolean | null | undefined)
    })
    return { id: newId('row'), cells }
  })
  columns.forEach((c) => {
    c.inferredType = inferColumnType(rows.map((r) => r.cells[c.id]))
    const longest = Math.max(c.name.length, ...rows.slice(0, 50).map((r) => cellDisplayLength(r.cells[c.id])))
    c.width = Math.min(260, Math.max(80, longest * 9 + 24))
  })
  return { columns, rows, source: options.source ?? { kind: 'manual' } }
}

function cellDisplayLength(cell: TableCell | undefined): number {
  if (!cell) return 0
  const v = cell.displayValue ?? (cell.kind === 'value' ? cell.value : cell.value)
  return v == null ? 0 : String(v).length
}

export function cloneBlockForPaste(block: CanvasBlock, sheetId: string, offset = 24): CanvasBlock {
  const ts = nowIso()
  const cloned = structuredClone(block) as CanvasBlock
  cloned.id = newId('blk')
  cloned.sheetId = sheetId
  cloned.rect = { ...cloned.rect, x: cloned.rect.x + offset, y: cloned.rect.y + offset }
  cloned.createdAt = ts
  cloned.updatedAt = ts
  cloned.generated = undefined
  if (cloned.type === 'wish') {
    cloned.content = { ...cloned.content, state: 'idle', generatedGroupIds: [], runHistory: [], lastRunId: undefined }
  }
  if (cloned.type === 'group') {
    cloned.content = { ...cloned.content, childBlockIds: [] }
  }
  return cloned
}
