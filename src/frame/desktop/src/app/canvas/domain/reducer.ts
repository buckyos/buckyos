/* ── Pure reducer: (doc, command) → doc. No UI, no side effects. ── */

import type { CanvasCommand } from './commands'
import { newId, nowIso } from './ids'
import { createSheet } from './factories'
import { expandMoveSet, recomputeGeneratedStatuses, refBlockId, refKey, unionRect } from './selectors'
import type {
  CanvasBlock,
  CanvasDocument,
  CanvasSheet,
  ContextRef,
  GroupBlock,
  TableBlock,
  WishBlock,
} from './types'
import { MIN_BLOCK_HEIGHT, MIN_BLOCK_WIDTH } from './types'
import { QUIET_COMMANDS } from './commands'

export function applyCommand(doc: CanvasDocument, cmd: CanvasCommand): CanvasDocument {
  const next = reduce(doc, cmd)
  if (next === doc) return doc
  if (QUIET_COMMANDS.has(cmd.type)) return next
  return { ...next, revision: doc.revision + 1, updatedAt: nowIso() }
}

function touch<T extends CanvasBlock>(block: T, bumpData = false): T {
  return {
    ...block,
    contentRevision: block.contentRevision + 1,
    dataRevision: bumpData ? block.dataRevision + 1 : block.dataRevision,
    updatedAt: nowIso(),
  }
}

function withBlocks(doc: CanvasDocument, blocks: Record<string, CanvasBlock>): CanvasDocument {
  return { ...doc, blocks }
}

function sheetById(doc: CanvasDocument, id: string): CanvasSheet | undefined {
  return doc.sheets.find((s) => s.id === id)
}

function updateSheet(doc: CanvasDocument, id: string, fn: (s: CanvasSheet) => CanvasSheet): CanvasDocument {
  return { ...doc, sheets: doc.sheets.map((s) => (s.id === id ? fn(s) : s)) }
}

/** Refit group rects around their children. */
function refitGroups(doc: CanvasDocument, changedIds: Set<string>): CanvasDocument {
  let blocks = doc.blocks
  for (const block of Object.values(doc.blocks)) {
    if (block.type !== 'group') continue
    if (!block.content.childBlockIds.some((c) => changedIds.has(c)) && !changedIds.has(block.id)) continue
    const rect = fitGroupRect(blocks, block)
    if (rect) blocks = { ...blocks, [block.id]: { ...block, rect } }
  }
  return blocks === doc.blocks ? doc : withBlocks(doc, blocks)
}

export const GROUP_PADDING = { top: 44, side: 16, bottom: 16 }

function fitGroupRect(blocks: Record<string, CanvasBlock>, group: GroupBlock) {
  const children = group.content.childBlockIds.map((id) => blocks[id]).filter(Boolean) as CanvasBlock[]
  const u = unionRect(children.map((c) => c.rect))
  if (!u) return null
  return {
    x: u.x - GROUP_PADDING.side,
    y: u.y - GROUP_PADDING.top,
    width: u.width + GROUP_PADDING.side * 2,
    height: u.height + GROUP_PADDING.top + GROUP_PADDING.bottom,
  }
}

function deleteBlocks(doc: CanvasDocument, ids: string[]): CanvasDocument {
  const toDelete = new Set<string>()
  for (const id of ids) {
    const b = doc.blocks[id]
    if (!b) continue
    toDelete.add(id)
    if (b.type === 'group') b.content.childBlockIds.forEach((c) => toDelete.add(c))
  }
  if (toDelete.size === 0) return doc
  const blocks: Record<string, CanvasBlock> = {}
  for (const [id, b] of Object.entries(doc.blocks)) {
    if (toDelete.has(id)) continue
    let nb = b
    if (nb.type === 'group') {
      const kept = nb.content.childBlockIds.filter((c) => !toDelete.has(c))
      if (kept.length !== nb.content.childBlockIds.length) nb = { ...nb, content: { ...nb.content, childBlockIds: kept } }
    }
    if (nb.type === 'wish') {
      const kept = nb.content.generatedGroupIds.filter((g) => !toDelete.has(g))
      const refs = nb.content.contextRefs.filter((r) => !toDelete.has(r.blockId))
      if (kept.length !== nb.content.generatedGroupIds.length || refs.length !== nb.content.contextRefs.length) {
        nb = { ...nb, content: { ...nb.content, generatedGroupIds: kept, contextRefs: refs } }
      }
    }
    blocks[id] = nb
  }
  const sheets = doc.sheets.map((s) => ({ ...s, blockIds: s.blockIds.filter((id) => !toDelete.has(id)) }))
  const bindings = doc.bindings.filter((b) => !toDelete.has(b.targetBlockId))
  const presentationPaths = doc.presentationPaths.map((p) => ({
    ...p,
    steps: p.steps.map((st) => ({ ...st, targetBlockIds: st.targetBlockIds.filter((id) => !toDelete.has(id)) })),
  }))
  return recomputeGeneratedStatuses({ ...doc, blocks, sheets, bindings, presentationPaths })
}

function addBlocks(doc: CanvasDocument, list: CanvasBlock[]): CanvasDocument {
  if (list.length === 0) return doc
  const blocks = { ...doc.blocks }
  const sheets = doc.sheets.map((s) => ({ ...s, blockIds: [...s.blockIds] }))
  for (const b of list) {
    blocks[b.id] = b
    const sheet = sheets.find((s) => s.id === b.sheetId)
    if (sheet && !sheet.blockIds.includes(b.id)) sheet.blockIds.push(b.id)
  }
  return { ...doc, blocks, sheets }
}

function reduce(doc: CanvasDocument, cmd: CanvasCommand): CanvasDocument {
  switch (cmd.type) {
    case 'SET_TITLE':
      return { ...doc, title: cmd.title }

    case 'CREATE_BLOCKS':
      return addBlocks(doc, cmd.blocks)

    case 'DELETE_BLOCKS':
      return deleteBlocks(doc, cmd.ids)

    case 'MOVE_BLOCKS': {
      const ids = expandMoveSet(doc, cmd.ids)
      const blocks = { ...doc.blocks }
      const changed = new Set<string>()
      for (const id of ids) {
        const b = blocks[id]
        if (!b || b.locked) continue
        blocks[id] = { ...b, rect: { ...b.rect, x: b.rect.x + cmd.dx, y: b.rect.y + cmd.dy }, updatedAt: nowIso() }
        changed.add(id)
      }
      if (changed.size === 0) return doc
      return refitGroups(withBlocks(doc, blocks), changed)
    }

    case 'RESIZE_BLOCK': {
      const b = doc.blocks[cmd.id]
      if (!b || b.locked) return doc
      const rect = {
        x: cmd.rect.x,
        y: cmd.rect.y,
        width: Math.max(MIN_BLOCK_WIDTH, cmd.rect.width),
        height: Math.max(MIN_BLOCK_HEIGHT, cmd.rect.height),
      }
      const next = withBlocks(doc, { ...doc.blocks, [cmd.id]: { ...b, rect, updatedAt: nowIso() } })
      return refitGroups(next, new Set([cmd.id]))
    }

    case 'UPDATE_BLOCK': {
      const b = doc.blocks[cmd.id]
      if (!b) return doc
      const { content, ...rest } = cmd.patch
      let nb = { ...b, ...rest } as CanvasBlock
      if (content !== undefined) nb = touch({ ...nb, content } as CanvasBlock, b.type === 'text' || b.type === 'metric' || b.type === 'image' || b.type === 'video')
      if (cmd.userEdit && nb.generated && !nb.generated.detached) {
        nb = { ...nb, generated: { ...nb.generated, userModified: true } }
        // propagate to parent group so re-run prompts the user (FR-RESULT-003)
        const group = Object.values(doc.blocks).find(
          (g): g is GroupBlock => g.type === 'group' && g.content.childBlockIds.includes(nb.id),
        )
        if (group?.generated) {
          const blocks = {
            ...doc.blocks,
            [nb.id]: nb,
            [group.id]: { ...group, generated: { ...group.generated, userModified: true } },
          }
          return recomputeGeneratedStatuses(withBlocks(doc, blocks))
        }
      }
      const next = withBlocks(doc, { ...doc.blocks, [cmd.id]: nb })
      return recomputeGeneratedStatuses(rest.rect ? refitGroups(next, new Set([cmd.id])) : next)
    }

    case 'REORDER_Z': {
      const b = doc.blocks[cmd.id]
      if (!b) return doc
      const zs = Object.values(doc.blocks).filter((x) => x.sheetId === b.sheetId).map((x) => x.zIndex)
      const z = cmd.to === 'front' ? Math.max(0, ...zs) + 1 : Math.min(0, ...zs) - 1
      return withBlocks(doc, { ...doc.blocks, [cmd.id]: { ...b, zIndex: z } })
    }

    case 'UPDATE_TABLE_CELLS': {
      const b = doc.blocks[cmd.id]
      if (!b || b.type !== 'table' || b.locked) return doc
      const rowIndex = new Map(b.content.rows.map((r, i) => [r.id, i]))
      const rows = [...b.content.rows]
      let any = false
      for (const e of cmd.edits) {
        const i = rowIndex.get(e.rowId)
        if (i === undefined) continue
        const prev = rows[i].cells[e.columnId]
        if (prev && JSON.stringify(prev) === JSON.stringify(e.cell)) continue
        if (!prev && e.cell.kind === 'value' && e.cell.value === null) continue
        rows[i] = { ...rows[i], cells: { ...rows[i].cells, [e.columnId]: e.cell } }
        any = true
      }
      if (!any) return doc
      const nb = touch({ ...b, content: { ...b.content, rows } }, true)
      let next = withBlocks(doc, { ...doc.blocks, [cmd.id]: nb })
      if (b.generated && !b.generated.detached) {
        next = reduce(next, { type: 'UPDATE_BLOCK', id: b.id, patch: {}, userEdit: true })
      }
      return recomputeGeneratedStatuses(next)
    }

    case 'TABLE_STRUCTURE': {
      const b = doc.blocks[cmd.id]
      if (!b || b.type !== 'table' || b.locked) return doc
      const c = b.content
      const a = cmd.action
      let content = c
      if (a.kind === 'addRow') {
        const row = a.row ?? { id: newId('row'), cells: {} }
        const idx = a.afterRowId ? c.rows.findIndex((r) => r.id === a.afterRowId) + 1 : c.rows.length
        content = { ...c, rows: [...c.rows.slice(0, idx), row, ...c.rows.slice(idx)] }
      } else if (a.kind === 'addColumn') {
        const column = a.column ?? { id: newId('col'), name: `列${c.columns.length + 1}`, width: 120 }
        const idx = a.afterColumnId ? c.columns.findIndex((x) => x.id === a.afterColumnId) + 1 : c.columns.length
        content = { ...c, columns: [...c.columns.slice(0, idx), column, ...c.columns.slice(idx)] }
      } else if (a.kind === 'deleteRows') {
        const del = new Set(a.rowIds)
        content = { ...c, rows: c.rows.filter((r) => !del.has(r.id)) }
      } else if (a.kind === 'deleteColumns') {
        const del = new Set(a.columnIds)
        content = { ...c, columns: c.columns.filter((x) => !del.has(x.id)) }
      } else if (a.kind === 'renameColumn') {
        content = { ...c, columns: c.columns.map((x) => (x.id === a.columnId ? { ...x, name: a.name } : x)) }
      } else if (a.kind === 'setColumnWidth') {
        content = { ...c, columns: c.columns.map((x) => (x.id === a.columnId ? { ...x, width: a.width } : x)) }
        return withBlocks(doc, { ...doc.blocks, [cmd.id]: { ...b, content } })
      } else if (a.kind === 'setCellWish') {
        const cellWishes = { ...(c.cellWishes ?? {}) }
        if (a.wish) cellWishes[a.key] = a.wish
        else delete cellWishes[a.key]
        content = { ...c, cellWishes }
        return withBlocks(doc, { ...doc.blocks, [cmd.id]: { ...b, content } })
      }
      const bumpData = a.kind !== 'renameColumn'
      const nb = touch({ ...b, content }, bumpData)
      return recomputeGeneratedStatuses(withBlocks(doc, { ...doc.blocks, [cmd.id]: nb }))
    }

    case 'ADD_SHEET': {
      const sheet = createSheet(cmd.name ?? `Sheet ${doc.sheets.length + 1}`, doc.sheets.length)
      return { ...doc, sheets: [...doc.sheets, sheet], activeSheetId: sheet.id }
    }

    case 'RENAME_SHEET':
      return updateSheet(doc, cmd.id, (s) => ({ ...s, name: cmd.name }))

    case 'DELETE_SHEET': {
      if (doc.sheets.length <= 1) return doc
      const sheet = sheetById(doc, cmd.id)
      if (!sheet) return doc
      const without = deleteBlocks(doc, sheet.blockIds)
      const sheets = without.sheets.filter((s) => s.id !== cmd.id).map((s, i) => ({ ...s, order: i }))
      return {
        ...without,
        sheets,
        activeSheetId: without.activeSheetId === cmd.id ? sheets[0].id : without.activeSheetId,
      }
    }

    case 'MOVE_SHEET': {
      const idx = doc.sheets.findIndex((s) => s.id === cmd.id)
      const to = idx + cmd.direction
      if (idx < 0 || to < 0 || to >= doc.sheets.length) return doc
      const sheets = [...doc.sheets]
      ;[sheets[idx], sheets[to]] = [sheets[to], sheets[idx]]
      return { ...doc, sheets: sheets.map((s, i) => ({ ...s, order: i })) }
    }

    case 'DUPLICATE_SHEET': {
      const sheet = sheetById(doc, cmd.id)
      if (!sheet) return doc
      const idMap = new Map<string, string>()
      const copies: CanvasBlock[] = []
      const newSheet = createSheet(`${sheet.name} 副本`, doc.sheets.length)
      for (const id of sheet.blockIds) idMap.set(id, newId('blk'))
      for (const id of sheet.blockIds) {
        const b = doc.blocks[id]
        if (!b) continue
        const nb = structuredClone(b) as CanvasBlock
        nb.id = idMap.get(id)!
        nb.sheetId = newSheet.id
        if (nb.type === 'group') nb.content.childBlockIds = nb.content.childBlockIds.map((c) => idMap.get(c) ?? c)
        if (nb.type === 'wish') {
          nb.content.generatedGroupIds = nb.content.generatedGroupIds.map((g) => idMap.get(g) ?? g)
          nb.content.contextRefs = nb.content.contextRefs.map((r) => ({ ...r, blockId: idMap.get(r.blockId) ?? r.blockId }))
          nb.content.state = 'idle'
        }
        if (nb.generated) {
          nb.generated = {
            ...nb.generated,
            wishBlockId: idMap.get(nb.generated.wishBlockId) ?? nb.generated.wishBlockId,
            sourceRevisions: nb.generated.sourceRevisions.map((s) => {
              const [bid, ...rest] = s.refKey.split(':')
              return { ...s, refKey: [idMap.get(bid) ?? bid, ...rest].join(':') }
            }),
          }
        }
        copies.push(nb)
      }
      const withSheet = { ...doc, sheets: [...doc.sheets, newSheet], activeSheetId: newSheet.id }
      return addBlocks(withSheet, copies)
    }

    case 'SET_ACTIVE_SHEET':
      return sheetById(doc, cmd.id) ? { ...doc, activeSheetId: cmd.id } : doc

    case 'SET_CAMERA':
      return updateSheet(doc, cmd.sheetId, (s) => ({ ...s, camera: cmd.camera }))

    case 'APPLY_AGENT_PATCH':
      return applyPatch(doc, cmd)

    case 'DETACH_GENERATED': {
      const group = doc.blocks[cmd.groupId]
      if (!group?.generated) return doc
      const ids = group.type === 'group' ? [group.id, ...group.content.childBlockIds] : [group.id]
      const blocks = { ...doc.blocks }
      for (const id of ids) {
        const b = blocks[id]
        if (b?.generated) blocks[id] = { ...b, generated: { ...b.generated, detached: true } } as CanvasBlock
      }
      const wishId = group.generated.wishBlockId
      const wish = blocks[wishId]
      if (wish?.type === 'wish') {
        blocks[wishId] = {
          ...wish,
          content: { ...wish.content, generatedGroupIds: wish.content.generatedGroupIds.filter((g) => g !== cmd.groupId) },
        }
      }
      const bindings = doc.bindings.filter((b) => !ids.includes(b.targetBlockId))
      return { ...withBlocks(doc, blocks), bindings }
    }

    case 'WISH_SET_STATE': {
      const w = doc.blocks[cmd.id]
      if (!w || w.type !== 'wish') return doc
      const content = { ...w.content, state: cmd.state, lastError: cmd.error, lastRunId: cmd.runId ?? w.content.lastRunId }
      return withBlocks(doc, { ...doc.blocks, [cmd.id]: { ...w, content } })
    }

    case 'WISH_PUSH_HISTORY': {
      const w = doc.blocks[cmd.id]
      if (!w || w.type !== 'wish') return doc
      const existing = w.content.runHistory.filter((h) => h.runId !== cmd.summary.runId)
      const runHistory = [cmd.summary, ...existing].slice(0, 10)
      return withBlocks(doc, { ...doc.blocks, [cmd.id]: { ...w, content: { ...w.content, runHistory } } })
    }

    case 'PRESENTATION_CREATE_PATH': {
      const ts = nowIso()
      return {
        ...doc,
        presentationPaths: [
          ...doc.presentationPaths,
          { id: cmd.id ?? newId('path'), name: cmd.name, steps: [], createdAt: ts, updatedAt: ts },
        ],
      }
    }
    case 'PRESENTATION_RENAME_PATH':
      return mapPath(doc, cmd.pathId, (p) => ({ ...p, name: cmd.name }))
    case 'PRESENTATION_DELETE_PATH':
      return { ...doc, presentationPaths: doc.presentationPaths.filter((p) => p.id !== cmd.pathId) }
    case 'PRESENTATION_ADD_STEP':
      return mapPath(doc, cmd.pathId, (p) => {
        const idx = cmd.index ?? p.steps.length
        return { ...p, steps: [...p.steps.slice(0, idx), cmd.step, ...p.steps.slice(idx)] }
      })
    case 'PRESENTATION_UPDATE_STEP':
      return mapPath(doc, cmd.pathId, (p) => ({
        ...p,
        steps: p.steps.map((s) => (s.id === cmd.stepId ? { ...s, ...cmd.patch } : s)),
      }))
    case 'PRESENTATION_REMOVE_STEP':
      return mapPath(doc, cmd.pathId, (p) => ({ ...p, steps: p.steps.filter((s) => s.id !== cmd.stepId) }))
    case 'PRESENTATION_MOVE_STEP':
      return mapPath(doc, cmd.pathId, (p) => {
        const idx = p.steps.findIndex((s) => s.id === cmd.stepId)
        const to = idx + cmd.direction
        if (idx < 0 || to < 0 || to >= p.steps.length) return p
        const steps = [...p.steps]
        ;[steps[idx], steps[to]] = [steps[to], steps[idx]]
        return { ...p, steps }
      })

    case 'RESTORE_SNAPSHOT':
      return { ...cmd.doc, id: doc.id }

    default:
      return doc
  }
}

function mapPath(
  doc: CanvasDocument,
  pathId: string,
  fn: (p: CanvasDocument['presentationPaths'][number]) => CanvasDocument['presentationPaths'][number],
): CanvasDocument {
  return {
    ...doc,
    presentationPaths: doc.presentationPaths.map((p) => (p.id === pathId ? { ...fn(p), updatedAt: nowIso() } : p)),
  }
}

/**
 * Atomic patch application (PRD FR-AGENT-004). The patch is assumed validated;
 * any failure throws and the caller keeps the previous doc.
 */
function applyPatch(
  doc: CanvasDocument,
  cmd: Extract<CanvasCommand, { type: 'APPLY_AGENT_PATCH' }>,
): CanvasDocument {
  const { patch, wishId, adapter } = cmd
  const wish = doc.blocks[wishId]
  if (!wish || wish.type !== 'wish') throw new Error('许愿格不存在')

  // Downstream consumers of the groups being replaced (chained wishes, their results, bindings)
  // must follow the replacement to its new id instead of silently losing their source.
  const replacedIds = new Set<string>()
  let replacedRevision = 0
  for (const gid of cmd.replaceGroupIds) {
    const g = doc.blocks[gid]
    if (!g) continue
    replacedIds.add(gid)
    replacedRevision = Math.max(replacedRevision, g.dataRevision)
    if (g.type === 'group') g.content.childBlockIds.forEach((c) => replacedIds.add(c))
  }
  const dependentWishRefs = new Map<string, ContextRef[]>()
  if (replacedIds.size) {
    for (const b of Object.values(doc.blocks)) {
      if (b.type === 'wish' && b.id !== wishId && b.content.contextRefs.some((r) => replacedIds.has(r.blockId))) {
        dependentWishRefs.set(b.id, b.content.contextRefs)
      }
    }
  }

  let next = cmd.replaceGroupIds.length ? deleteBlocks(doc, cmd.replaceGroupIds) : doc
  const ts = nowIso()
  const sourceRevisions = (next.blocks[wishId] as WishBlock).content.contextRefs.map((ref) => ({
    refKey: refKey(ref),
    revision: next.blocks[ref.blockId]?.dataRevision ?? ref.revision,
  }))
  const meta = (extra?: Partial<CanvasBlock['generated']>) => ({
    runId: patch.runId,
    wishBlockId: wishId,
    agentAdapter: adapter,
    generatedAt: ts,
    sourceRevisions,
    status: 'fresh' as const,
    userModified: false,
    detached: false,
    assumptions: patch.assumptions,
    warnings: patch.warnings,
    ...extra,
  })

  const created: CanvasBlock[] = []
  const createdGroups: string[] = []
  const blocks = { ...next.blocks }
  const bindings = [...next.bindings]
  let presentationPaths = next.presentationPaths

  for (const op of patch.operations) {
    switch (op.op) {
      case 'createBlock': {
        const b: CanvasBlock = { ...op.block, generated: meta(), createdAt: ts, updatedAt: ts, locked: false }
        blocks[b.id] = b
        created.push(b)
        if (b.type === 'group') createdGroups.push(b.id)
        break
      }
      case 'updateBlock': {
        const b = blocks[op.blockId]
        if (!b) throw new Error(`引用的块不存在: ${op.blockId}`)
        blocks[op.blockId] = touch({ ...b, ...op.patch } as CanvasBlock)
        break
      }
      case 'createBinding':
        bindings.push(op.binding)
        break
      case 'createGroup': {
        const existing = blocks[op.groupId]
        if (existing?.type === 'group') {
          blocks[op.groupId] = { ...existing, content: { ...existing.content, childBlockIds: op.childBlockIds } }
        }
        break
      }
      case 'resizeToFit': {
        const b = blocks[op.blockId]
        if (b?.type === 'group') {
          const rect = fitGroupRect(blocks, b)
          if (rect) blocks[op.blockId] = { ...b, rect }
        }
        break
      }
      case 'addPresentationStep':
        presentationPaths = presentationPaths.map((p) =>
          p.id === op.pathId ? { ...p, steps: [...p.steps, op.step] } : p,
        )
        break
      case 'updateTableCells': {
        const t = blocks[op.blockId]
        if (!t || t.type !== 'table') throw new Error(`表格不存在: ${op.blockId}`)
        const rows = t.content.rows.map((r) => {
          const edits = op.cells.filter((c) => c.rowId === r.id)
          if (!edits.length) return r
          const cells = { ...r.cells }
          for (const e of edits) cells[e.columnId] = e.cell
          return { ...r, cells }
        })
        blocks[op.blockId] = { ...(t as TableBlock), content: { ...t.content, rows }, updatedAt: ts }
        break
      }
    }
  }

  // group membership: every created non-group block belongs to a created group; if the
  // agent didn't group them, wrap in an implicit group so the result can be handled as a unit.
  const groupedIds = new Set(createdGroups.flatMap((g) => (blocks[g] as GroupBlock).content.childBlockIds))
  const loose = created.filter((b) => b.type !== 'group' && !groupedIds.has(b.id))
  if (loose.length) {
    const gid = newId('grp')
    const group: GroupBlock = {
      id: gid,
      sheetId: loose[0].sheetId,
      type: 'group',
      title: patch.summary.slice(0, 40) || '生成结果',
      rect: loose[0].rect,
      zIndex: 0,
      locked: false,
      contentRevision: 0,
      dataRevision: 0,
      generated: meta(),
      content: { childBlockIds: loose.map((b) => b.id), summary: patch.summary },
      createdAt: ts,
      updatedAt: ts,
    }
    blocks[gid] = group
    created.push(group)
    createdGroups.push(gid)
  }
  for (const gid of createdGroups) {
    const g = blocks[gid] as GroupBlock
    const rect = fitGroupRect(blocks, g)
    if (rect) blocks[gid] = { ...g, rect }
  }

  // bindings for every generated group from every context ref
  for (const gid of createdGroups) {
    for (const ref of (next.blocks[wishId] as WishBlock).content.contextRefs) {
      bindings.push({ id: newId('bind'), source: ref, targetBlockId: gid, createdByRunId: patch.runId })
    }
  }

  // re-point dependents of the replaced groups at the first new group; bump its data revision
  // past the old one so downstream results become "stale" rather than "broken"
  if (replacedIds.size && createdGroups.length) {
    const newGid = createdGroups[0]
    const newGroup = blocks[newGid] as GroupBlock
    blocks[newGid] = { ...newGroup, dataRevision: replacedRevision + 1 }
    const remapKey = (key: string) => {
      const [bid, ...rest] = key.split(':')
      return replacedIds.has(bid) ? newGid : [bid, ...rest].join(':')
    }
    for (const [depId, refs] of dependentWishRefs) {
      const dep = blocks[depId]
      if (!dep || dep.type !== 'wish') continue
      const seen = new Set<string>()
      const contextRefs: ContextRef[] = []
      for (const r of refs) {
        const mapped: ContextRef = replacedIds.has(r.blockId) ? { kind: 'block', blockId: newGid, revision: replacedRevision + 1 } : r
        const key = refKey(mapped)
        if (seen.has(key)) continue
        seen.add(key)
        contextRefs.push(mapped)
      }
      blocks[depId] = { ...dep, content: { ...dep.content, contextRefs } }
    }
    for (const [id, b] of Object.entries(blocks)) {
      if (!b.generated || b.generated.wishBlockId === wishId) continue
      if (!b.generated.sourceRevisions.some((s) => replacedIds.has(refBlockId(s.refKey)))) continue
      blocks[id] = { ...b, generated: { ...b.generated, sourceRevisions: b.generated.sourceRevisions.map((s) => ({ ...s, refKey: remapKey(s.refKey) })) } } as CanvasBlock
    }
    for (let i = 0; i < bindings.length; i++) {
      const bd = bindings[i]
      if (replacedIds.has(bd.source.blockId)) bindings[i] = { ...bd, source: { kind: 'block', blockId: newGid, revision: replacedRevision + 1 } }
    }
  }

  const w = blocks[wishId] as WishBlock
  blocks[wishId] = {
    ...w,
    content: {
      ...w.content,
      generatedGroupIds: [...w.content.generatedGroupIds.filter((g) => blocks[g]), ...createdGroups],
      lastRunId: patch.runId,
    },
  }

  next = { ...next, blocks, bindings, presentationPaths }
  // register created blocks with their sheet
  const sheets = next.sheets.map((s) => {
    const add = created.filter((b) => b.sheetId === s.id && !s.blockIds.includes(b.id)).map((b) => b.id)
    return add.length ? { ...s, blockIds: [...s.blockIds, ...add] } : s
  })
  return recomputeGeneratedStatuses({ ...next, sheets })
}
