/* The browser host of the table importer (表格数据导入 TODO §3.4, §3.5): the wizard's grids and choices become plans
 * (`@table/plan`, every value checked by the core's own `normalize_value` through WASM), then one Commit: the
 * TableSources (in the canvas's content folder or the chosen data folder) with all their rows and, on a canvas, their
 * table Blocks. Cancelling or a refused Commit leaves nothing behind; undo takes the import back whole. The original
 * file can be kept as an asset next to the table. */

import { ImportReport } from '@drawio/convert'
import { blockSize, planTable, tableOperations, TableLimitError, uniqueName, type Grid, type Json as TableJson, type TableChoices, type TablePlan } from '@table/mod'
import { describeError } from '../../api/session'
import { randomId } from '../../api/ids'
import type { Operation } from '../../api/types'
import { aiwsT } from '../../i18n'
import type { WorkspaceStore } from '../../state/store'
import type { ImportOutcome } from '../canvas/drawioImport'
import { submitImport } from './submitImport'
import type { TableDestination } from './tableImportRequest'

export interface ImportPart { key: string; label: string; grid: Grid; choices: TableChoices }

/** `send`: request `done + 1` of `total` of a large Commit is going up; `commit`: the service is writing. */
export interface TableImportProgress { stage: 'plan' | 'upload' | 'send' | 'commit'; done: number; total: number }

/** The core's own check of a value for a field (WASM `normalize_value`). */
export function wasmValidate(store: WorkspaceStore) {
  return (field: Record<string, TableJson>, value: TableJson) => {
    try { store.core.normalize_value(JSON.stringify(field), JSON.stringify(value)); return true } catch { return false }
  }
}

const GAP = 40

export async function runTableImport(store: WorkspaceStore, parts: ImportPart[], destination: TableDestination, options: {
  name: string
  keep: { name: string; bytes: Uint8Array } | null
  signal: AbortSignal
  onProgress: (progress: TableImportProgress) => void
}): Promise<ImportOutcome> {
  const report = new ImportReport()
  const outcome: ImportOutcome = { name: options.name, report, surfaceId: 'canvas' in destination ? destination.canvas.surfaceId : null, created: [], bounds: null, kind: 'table' }
  // 1 plans (nothing is written yet)
  const validate = wasmValidate(store)
  const plans: { part: ImportPart; plan: TablePlan }[] = []
  for (const part of parts) {
    options.onProgress({ stage: 'plan', done: plans.length, total: parts.length })
    await new Promise((resolve) => setTimeout(resolve, 0))
    try {
      plans.push({ part, plan: planTable(part.grid, part.choices, { validate }) })
    } catch (error) {
      if (error instanceof TableLimitError) return { ...outcome, error: aiwsT('aiworkspace.import.tooLarge', { rows: error.rows, cells: error.cells }) }
      throw error
    }
  }
  if (options.signal.aborted) return { ...outcome, error: aiwsT('aiworkspace.import.cancelled') }
  for (const { part, plan } of plans) {
    for (const note of plan.report.list()) {
      const existing = report.notes.get(note.kind)
      if (existing) { existing.count += note.count; for (const s of note.samples) if (existing.samples.length < 5 && !existing.samples.includes(s)) existing.samples.push(s) }
      else report.notes.set(note.kind, { ...note, samples: parts.length > 1 ? note.samples.map((s) => `${part.label}: ${s}`).slice(0, 5) : [...note.samples] })
    }
  }
  const folderId = 'canvas' in destination ? destination.canvas.contentFolderId : destination.folderId
  // 2 the original file, as an asset next to the tables
  let keepOp: Operation | null = null
  if (options.keep) {
    options.onProgress({ stage: 'upload', done: 0, total: 1 })
    try {
      const uploaded = await store.session.uploadAsset(new Blob([options.keep.bytes as BlobPart]), options.keep.name)
      keepOp = { op: 'entity.create', entity_id: randomId('d'), type_id: 'buckyos.asset-ref', parent_id: folderId, order_key: '', payload: { object_id: uploaded.object_id, file_name: options.keep.name.slice(0, 255) } }
    } catch (error) {
      report.add('table_keep_failed', 'The original file could not be kept', describeError(error))
    }
  }
  // 3 operations: names unique in the folder, Blocks side by side (a flow page: after the selected Block)
  const taken = new Set(store.outline.childrenOf(folderId).map((c) => c.name).filter((n): n is string => Boolean(n)))
  let folderKey = store.outline.childrenOf(folderId).at(-1)?.order_key ?? null
  const canvas = 'canvas' in destination ? destination.canvas : null
  let blockKeyAfter: string | null = canvas ? (canvas.isFree ? store.outline.childrenOf(canvas.surfaceId).at(-1)?.order_key ?? null : null) : null
  let blockKeyBefore: string | null = null
  if (canvas && !canvas.isFree) {
    const siblings = store.outline.childrenOf(canvas.surfaceId)
    const at = canvas.afterId ? siblings.findIndex((s) => s.entity_id === canvas.afterId) : siblings.length - 1
    blockKeyAfter = siblings[at]?.order_key ?? null
    blockKeyBefore = at >= 0 ? siblings[at + 1]?.order_key ?? null : siblings[0]?.order_key ?? null
  }
  const sizes = plans.map(({ plan }) => blockSize(plan.fields.length, plan.records.length))
  const totalW = sizes.reduce((w, s) => w + s.w + GAP, -GAP)
  const maxH = Math.max(0, ...sizes.map((s) => s.h))
  let x = canvas ? (canvas.at ? canvas.at.x : Math.round(canvas.center.x - totalW / 2)) : 0
  const y = canvas ? (canvas.at ? canvas.at.y : Math.round(canvas.center.y - maxH / 2)) : 0
  const built = plans.map(({ plan }, i) => {
    const name = uniqueName(plan.title, taken)
    taken.add(name)
    let block: Parameters<typeof tableOperations>[1]['block']
    if (canvas) {
      const orderKey = store.core.order_key_between(blockKeyAfter ?? undefined, blockKeyBefore ?? undefined)
      blockKeyAfter = orderKey
      block = { parentId: canvas.surfaceId, afterKey: null, orderKey, placement: canvas.isFree ? { x, y, w: sizes[i].w, h: sizes[i].h } : null }
      if (canvas.isFree) outcome.bounds = { x: canvas.at ? canvas.at.x : Math.round(canvas.center.x - totalW / 2), y, w: totalW, h: maxH }
      x += sizes[i].w + GAP
    }
    const ops = tableOperations(plan, { folderId, afterKey: folderKey, name, block })
    folderKey = (ops.operations[0].order_key as string) ?? folderKey
    return { plan, ops }
  })
  if (keepOp) keepOp.order_key = store.core.order_key_between(folderKey ?? undefined, undefined)
  // 4 one Commit: every table with its rows and the kept file — all of it, or nothing
  const total = built.reduce((n, b) => n + b.plan.records.length, 0)
  if (options.signal.aborted) return { ...outcome, error: aiwsT('aiworkspace.import.cancelled') }
  const operations = [...built.flatMap((b) => b.ops.operations), ...(keepOp ? [keepOp] : [])] as unknown as Operation[]
  const label = aiwsT('aiworkspace.import.label', { name: built.length === 1 ? built[0].plan.title : options.name })
  let failed: string | null
  try {
    failed = await submitImport(store, { editId: `table-import:${built[0]?.ops.sourceId ?? ''}`, label, operations }, 'files', (p) => options.onProgress(p.stage === 'commit' ? { stage: 'send', done: p.done, total: p.total } : { stage: 'commit', done: 0, total }))
  } catch (error) {
    failed = describeError(error)
  }
  if (failed) return { ...outcome, bounds: null, error: aiwsT('aiworkspace.import.failed', { reason: failed }) }
  for (const { ops } of built) if (ops.cellId) outcome.created.push(ops.cellId)
  options.onProgress({ stage: 'commit', done: total, total })
  outcome.summary = aiwsT('aiworkspace.import.summary', { tables: aiwsT('aiworkspace.import.count.tables', { count: built.length }), rows: aiwsT('aiworkspace.import.count.rows', { count: total }) })
  return outcome
}
