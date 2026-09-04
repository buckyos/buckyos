/* ── .aicanvas.json import / export (PRD FR-DOC-003) ── */

import { CANVAS_SCHEMA_VERSION, type CanvasDocument } from '../domain/types'
import { newId, nowIso } from '../domain/ids'
import { recomputeGeneratedStatuses } from '../domain/selectors'

export function exportDocument(doc: CanvasDocument): string {
  return JSON.stringify(recomputeGeneratedStatuses(doc), null, 2)
}

export function downloadText(filename: string, text: string, mime = 'application/json') {
  const blob = new Blob([text], { type: mime })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = filename
  document.body.appendChild(a)
  a.click()
  a.remove()
  setTimeout(() => URL.revokeObjectURL(url), 1000)
}

export function exportFilename(doc: CanvasDocument): string {
  const safe = doc.title.replace(/[\\/:*?"<>|]/g, '_').slice(0, 40) || 'canvas'
  return `${safe}.aicanvas.json`
}

export interface ImportResult {
  doc: CanvasDocument
  warnings: string[]
}

function compareVersion(a: string, b: string): number {
  const pa = a.split('.').map(Number)
  const pb = b.split('.').map(Number)
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const d = (pa[i] ?? 0) - (pb[i] ?? 0)
    if (d) return d
  }
  return 0
}

/** Tolerant import: unknown fields are kept, missing collections are filled, higher schema versions are rejected. */
export function importDocument(text: string, options: { newId?: boolean } = {}): ImportResult {
  let raw: unknown
  try {
    raw = JSON.parse(text)
  } catch {
    throw new Error('文件不是有效的 JSON')
  }
  if (!raw || typeof raw !== 'object') throw new Error('文件内容不是画布文档')
  const obj = raw as Partial<CanvasDocument> & Record<string, unknown>
  const warnings: string[] = []
  const version = typeof obj.schemaVersion === 'string' ? obj.schemaVersion : ''
  if (!version) throw new Error('缺少 schemaVersion，不是 .aicanvas.json 文件')
  if (compareVersion(version, CANVAS_SCHEMA_VERSION) > 0) {
    throw new Error(`文档版本 ${version} 高于当前支持的 ${CANVAS_SCHEMA_VERSION}，请升级后再打开`)
  }
  if (!Array.isArray(obj.sheets) || obj.sheets.length === 0) throw new Error('文档没有任何 Sheet')
  const ts = nowIso()
  const doc: CanvasDocument = {
    ...obj,
    schemaVersion: CANVAS_SCHEMA_VERSION,
    id: options.newId || typeof obj.id !== 'string' ? newId('canvas') : obj.id,
    title: typeof obj.title === 'string' ? obj.title : '导入的画布',
    revision: typeof obj.revision === 'number' ? obj.revision : 0,
    activeSheetId: typeof obj.activeSheetId === 'string' ? obj.activeSheetId : obj.sheets[0].id,
    sheets: obj.sheets.map((s, i) => ({
      id: s.id ?? newId('sheet'),
      name: s.name ?? `Sheet ${i + 1}`,
      order: s.order ?? i,
      blockIds: Array.isArray(s.blockIds) ? s.blockIds : [],
      camera: s.camera ?? { x: 80, y: 80, zoom: 1 },
    })),
    blocks: obj.blocks && typeof obj.blocks === 'object' ? obj.blocks : {},
    bindings: Array.isArray(obj.bindings) ? obj.bindings : [],
    presentationPaths: Array.isArray(obj.presentationPaths) ? obj.presentationPaths : [],
    comments: Array.isArray(obj.comments) ? obj.comments : [],
    createdAt: typeof obj.createdAt === 'string' ? obj.createdAt : ts,
    updatedAt: ts,
    metadata: { ...(obj.metadata ?? {}), importedFrom: 'aicanvas.json' },
  }
  if (!doc.sheets.some((s) => s.id === doc.activeSheetId)) doc.activeSheetId = doc.sheets[0].id
  // drop dangling block ids, keep unknown block types (renderer shows a placeholder)
  const known = new Set(['text', 'table', 'wish', 'metric', 'chart', 'frame', 'group', 'interactive', 'image', 'video'])
  for (const [id, b] of Object.entries(doc.blocks)) {
    if (!b || typeof b !== 'object' || !b.rect) {
      delete doc.blocks[id]
      warnings.push(`已忽略损坏的块 ${id}`)
    } else if (!known.has(b.type)) {
      warnings.push(`块 ${id} 类型 ${String(b.type)} 未知，已保留但无法编辑`)
    }
  }
  doc.sheets = doc.sheets.map((s) => ({ ...s, blockIds: s.blockIds.filter((id) => doc.blocks[id]) }))
  return { doc: recomputeGeneratedStatuses(doc), warnings }
}
