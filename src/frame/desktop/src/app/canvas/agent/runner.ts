/* ── Wish run orchestration: state machine, adapter call, validation, atomic apply (PRD §11.8) ── */

import type { CanvasStore } from '../store/canvas-store'
import { newId, nowIso } from '../domain/ids'
import { refKey, unionRect } from '../domain/selectors'
import type { CanvasDocument, TableBlock, WishBlock } from '../domain/types'
import { GROUP_PADDING } from '../domain/reducer'
import { trackEvent } from '../events'
import { buildContext, contextItemFor, contextSize } from './context'
import { AgentRunError, type AgentRunEvent, type AgentRunRequest, type CanvasAgentAdapter, type CanvasPatch } from './contracts'
import { HttpCanvasAgentAdapter } from './http'
import { MockCanvasAgentAdapter } from './mock'
import { validatePatch } from './patch-validator'

export const MAX_CONTEXT_BYTES = 4 * 1024 * 1024

export interface RunOptions {
  mode: 'replace' | 'keep'
  /** override adapter for this run (e.g. "retry with mock") */
  adapterId?: 'mock' | 'http'
}

export interface PreflightResult {
  ok: boolean
  errors: string[]
  needsDecision: boolean
  userModifiedGroups: string[]
}

export class WishRunner {
  private controllers = new Map<string, AbortController>()
  readonly mock: MockCanvasAgentAdapter
  readonly http: HttpCanvasAgentAdapter

  private readonly store: CanvasStore

  constructor(store: CanvasStore) {
    this.store = store
    this.mock = new MockCanvasAgentAdapter(() => ({ debugMode: store.getState().settings.mockDebugMode }))
    this.http = new HttpCanvasAgentAdapter(() => ({ baseUrl: store.getState().settings.httpBaseUrl, timeoutMs: store.getState().settings.timeoutMs }))
  }

  adapter(id?: 'mock' | 'http'): CanvasAgentAdapter {
    const which = id ?? this.store.getState().settings.adapter
    return which === 'http' ? this.http : this.mock
  }

  isRunning(wishId: string) {
    return this.controllers.has(wishId)
  }

  /** PRD FR-WISH-005 – reasons a wish must not run right now. */
  preflight(wishId: string): PreflightResult {
    const doc = this.store.doc
    const wish = doc.blocks[wishId]
    const errors: string[] = []
    if (!wish || wish.type !== 'wish') return { ok: false, errors: ['许愿格不存在'], needsDecision: false, userModifiedGroups: [] }
    if (!wish.content.prompt.trim()) errors.push('请先写下你的目标')
    for (const ref of wish.content.contextRefs) {
      if (!doc.blocks[ref.blockId]) errors.push('有数据来源已被删除，请移除或重新选择')
    }
    if (this.controllers.has(wishId)) errors.push('正在运行中')
    const { items } = buildContext(doc, wish)
    if (contextSize(items) > MAX_CONTEXT_BYTES) errors.push('数据来源超过当前 Agent 允许的大小（4MB），请缩小选区')
    const userModifiedGroups = wish.content.generatedGroupIds.filter((g) => doc.blocks[g]?.generated?.userModified)
    return { ok: errors.length === 0, errors: [...new Set(errors)], needsDecision: userModifiedGroups.length > 0, userModifiedGroups }
  }

  cancel(wishId: string) {
    this.controllers.get(wishId)?.abort()
  }

  async run(wishId: string, options: RunOptions): Promise<boolean> {
    const store = this.store
    const pre = this.preflight(wishId)
    if (!pre.ok) {
      store.toast(pre.errors[0], 'error')
      return false
    }
    const doc0 = store.doc
    const wish = doc0.blocks[wishId] as WishBlock
    const adapter = this.adapter(options.adapterId)
    const runId = newId('run')
    const ctrl = new AbortController()
    this.controllers.set(wishId, ctrl)
    const startedAt = nowIso()
    const replaceGroupIds = options.mode === 'replace' ? wish.content.generatedGroupIds.filter((g) => doc0.blocks[g]) : []

    store.setRun(wishId, { wishId, runId, stage: 'planning', message: '正在理解目标', log: [], warnings: [], startedAt, error: undefined, errorDetails: undefined, errorKind: undefined })
    store.dispatch({ type: 'WISH_SET_STATE', id: wishId, state: 'planning', runId })
    trackEvent('wish_run_started', { adapter: adapter.id })

    const { items, warnings } = buildContext(doc0, wish)
    for (const w of warnings) store.appendRunLog(wishId, w, 'warning')
    const request: AgentRunRequest = {
      protocolVersion: '0.1',
      runId,
      canvas: { id: doc0.id, revision: doc0.revision, locale: 'zh-CN' },
      wish: { blockId: wishId, prompt: wish.content.prompt, outputPreference: wish.content.outputPreference },
      context: items,
      destination: destinationFor(doc0, wish, replaceGroupIds),
      capabilities: ['read_canvas_context', 'create_standard_blocks'],
    }

    const timeout = setTimeout(() => ctrl.abort(new AgentRunError('timeout', '运行超时')), store.getState().settings.timeoutMs)
    const onEvent = (e: AgentRunEvent) => this.handleEvent(wishId, e)
    let patch: CanvasPatch
    try {
      patch = await adapter.run(request, onEvent, ctrl.signal)
    } catch (e) {
      clearTimeout(timeout)
      this.controllers.delete(wishId)
      const reason = ctrl.signal.reason
      const err = reason instanceof AgentRunError ? reason : e instanceof AgentRunError ? e : new AgentRunError('failed', e instanceof Error ? e.message : String(e))
      const cancelled = err.kind === 'cancelled'
      store.dispatch({ type: 'WISH_SET_STATE', id: wishId, state: cancelled ? 'cancelled' : 'failed', error: cancelled ? undefined : err.message, runId })
      store.setRun(wishId, { stage: cancelled ? 'cancelled' : 'failed', message: err.message, error: cancelled ? undefined : err.message, errorDetails: err.details, errorKind: err.kind })
      store.appendRunLog(wishId, err.message, cancelled ? 'info' : 'error')
      store.dispatch({ type: 'WISH_PUSH_HISTORY', id: wishId, summary: summary(runId, startedAt, cancelled ? 'cancelled' : 'failed', wish, doc0, adapter.id, err.message) })
      trackEvent(cancelled ? 'wish_run_cancelled' : 'wish_run_failed', { kind: err.kind })
      return false
    }
    clearTimeout(timeout)
    this.controllers.delete(wishId)

    // validate
    store.setRun(wishId, { stage: 'validating', message: '正在校验结果' })
    store.dispatch({ type: 'WISH_SET_STATE', id: wishId, state: 'validating', runId })
    const docNow = store.doc
    const validation = validatePatch(docNow, patch, wishId)
    if (!validation.ok) {
      const kind = validation.conflict ? 'conflict' : 'invalid_patch'
      const msg = validation.conflict ? '画布在运行期间已变化，结果未写入。请基于最新内容重新运行。' : '结果未通过校验，画布未被修改。'
      store.dispatch({ type: 'WISH_SET_STATE', id: wishId, state: 'failed', error: msg, runId })
      store.setRun(wishId, { stage: 'failed', message: msg, error: msg, errorDetails: validation.errors, errorKind: kind })
      validation.errors.forEach((e) => store.appendRunLog(wishId, e, 'error'))
      store.dispatch({ type: 'WISH_PUSH_HISTORY', id: wishId, summary: summary(runId, startedAt, 'failed', wish, doc0, adapter.id, msg) })
      trackEvent('wish_run_failed', { kind })
      return false
    }

    // apply atomically (one undo entry)
    store.setRun(wishId, { stage: 'applying', message: '正在写入画布', percent: 95 })
    const applied = store.dispatch({ type: 'APPLY_AGENT_PATCH', patch, wishId, adapter: adapter.id, replaceGroupIds })
    if (!applied) {
      store.dispatch({ type: 'WISH_SET_STATE', id: wishId, state: 'failed', error: '写入画布失败', runId })
      store.setRun(wishId, { stage: 'failed', message: '写入画布失败', error: '写入画布失败' })
      return false
    }
    const after = store.doc.blocks[wishId] as WishBlock
    const newGroups = after.content.generatedGroupIds.filter((g) => !wish.content.generatedGroupIds.includes(g) || replaceGroupIds.includes(g))
    store.dispatch({ type: 'WISH_SET_STATE', id: wishId, state: 'succeeded', runId })
    store.setRun(wishId, { stage: 'succeeded', message: `已生成 ${countVisible(store.doc, newGroups)} 个结果`, percent: 100, warnings: patch.warnings })
    patch.warnings.forEach((w) => store.appendRunLog(wishId, w, 'warning'))
    store.appendRunLog(wishId, patch.summary)
    store.dispatch({ type: 'WISH_PUSH_HISTORY', id: wishId, summary: { ...summary(runId, startedAt, 'succeeded', wish, doc0, adapter.id), groupId: newGroups[0] } })
    trackEvent(replaceGroupIds.length ? 'result_refreshed' : 'wish_run_succeeded', { adapter: adapter.id })
    return true
  }

  /** Table AI cell (PRD §11.6). Uses the row as context, writes back one cell. */
  async runCell(tableId: string, rowId: string, columnId: string, prompt: string): Promise<boolean> {
    const store = this.store
    const doc0 = store.doc
    const table = doc0.blocks[tableId]
    if (!table || table.type !== 'table') return false
    const key = `${rowId}:${columnId}`
    const wishId = table.content.cellWishes?.[key]?.id ?? newId('cellwish')
    const rowIndex = table.content.rows.findIndex((r) => r.id === rowId)
    if (rowIndex < 0) return false
    const adapter = this.adapter()
    const runId = newId('run')
    const ctrl = new AbortController()
    const runKey = `cell:${tableId}:${key}`
    this.controllers.set(runKey, ctrl)
    store.dispatch({ type: 'TABLE_STRUCTURE', id: tableId, action: { kind: 'setCellWish', key, wish: { id: wishId, prompt, rowId, columnId, state: 'running' } } })
    store.setRun(runKey, { wishId: runKey, runId, stage: 'running', message: '正在计算', log: [], warnings: [], startedAt: nowIso(), cellKey: key })
    const ref = { kind: 'tableRange' as const, blockId: tableId, range: { rowStart: rowIndex, rowEnd: rowIndex, colStart: 0, colEnd: table.content.columns.length - 1 }, revision: table.dataRevision }
    const item = contextItemFor(doc0, ref)
    const request: AgentRunRequest = {
      protocolVersion: '0.1',
      runId,
      canvas: { id: doc0.id, revision: doc0.revision, locale: 'zh-CN' },
      wish: { blockId: wishId, prompt, outputPreference: 'auto' },
      context: item ? [item] : [],
      destination: { sheetId: table.sheetId, anchor: { x: table.rect.x, y: table.rect.y }, maxWidth: 400 },
      capabilities: ['read_canvas_context'],
      cell: { tableBlockId: tableId, rowId, columnId },
    }
    const finish = (state: 'succeeded' | 'failed', error?: string) => {
      this.controllers.delete(runKey)
      store.dispatch({ type: 'TABLE_STRUCTURE', id: tableId, action: { kind: 'setCellWish', key, wish: { id: wishId, prompt, rowId, columnId, state, lastRunAt: nowIso(), error } } })
      store.setRun(runKey, null)
    }
    try {
      const patch = await adapter.run(request, () => undefined, ctrl.signal)
      const doc = store.doc
      // cell patches are applied directly: only the target ai cell may be written
      const op = patch.operations.find((o) => o.op === 'updateTableCells')
      if (!op || op.op !== 'updateTableCells' || op.cells.some((c) => c.rowId !== rowId || c.columnId !== columnId)) {
        finish('failed', '结果未通过校验')
        return false
      }
      const t = doc.blocks[tableId] as TableBlock | undefined
      if (!t) return false
      store.dispatch({ type: 'UPDATE_TABLE_CELLS', id: tableId, edits: op.cells.map((c) => ({ ...c, cell: { ...c.cell, kind: 'ai' as const, wishId } })) })
      finish('succeeded')
      return true
    } catch (e) {
      finish('failed', e instanceof Error ? e.message : String(e))
      return false
    }
  }

  private handleEvent(wishId: string, e: AgentRunEvent) {
    const store = this.store
    switch (e.type) {
      case 'status':
        store.setRun(wishId, { stage: e.stage, message: e.message, percent: undefined })
        store.dispatch({ type: 'WISH_SET_STATE', id: wishId, state: e.stage })
        store.appendRunLog(wishId, `${stageLabel(e.stage)}：${e.message}`)
        break
      case 'progress':
        store.setRun(wishId, { stage: e.stage, message: e.message, percent: e.percent })
        if (store.doc.blocks[wishId]?.type === 'wish' && (store.doc.blocks[wishId] as WishBlock).content.state !== e.stage) {
          store.dispatch({ type: 'WISH_SET_STATE', id: wishId, state: e.stage })
        }
        store.appendRunLog(wishId, `${e.message}（${e.percent}%）`)
        break
      case 'warning':
        store.setRun(wishId, { warnings: [...(store.getState().runs[wishId]?.warnings ?? []), e.message] })
        store.appendRunLog(wishId, e.message, 'warning')
        break
      case 'log':
        store.appendRunLog(wishId, e.message)
        break
      case 'completed':
        store.appendRunLog(wishId, `Agent 任务完成 (${e.jobId})`)
        break
    }
  }
}

function stageLabel(stage: string): string {
  return { planning: '理解目标', running: '生成中', validating: '校验', applying: '写入' }[stage] ?? stage
}

function countVisible(doc: CanvasDocument, groupIds: string[]): number {
  return groupIds.reduce((n, g) => {
    const b = doc.blocks[g]
    return n + (b?.type === 'group' ? b.content.childBlockIds.length : 1)
  }, 0)
}

function summary(runId: string, startedAt: string, status: WishBlock['content']['state'], wish: WishBlock, doc: CanvasDocument, adapter: string, error?: string) {
  return {
    runId,
    startedAt,
    finishedAt: nowIso(),
    status,
    promptExcerpt: wish.content.prompt.slice(0, 80),
    sourceRevisions: wish.content.contextRefs.map((r) => ({ refKey: refKey(r), revision: doc.blocks[r.blockId]?.dataRevision ?? r.revision })),
    adapter,
    error,
  }
}

/** Where new results go: replacing → old group's spot; otherwise below the wish / below existing results. */
function destinationFor(doc: CanvasDocument, wish: WishBlock, replaceGroupIds: string[]) {
  const replaced = replaceGroupIds.map((g) => doc.blocks[g]).filter(Boolean)
  if (replaced.length) {
    const u = unionRect(replaced.map((b) => b.rect))!
    return { sheetId: wish.sheetId, anchor: { x: u.x + GROUP_PADDING.side, y: u.y + GROUP_PADDING.top }, maxWidth: 1000 }
  }
  const existing = wish.content.generatedGroupIds.map((g) => doc.blocks[g]).filter(Boolean)
  const bottom = Math.max(wish.rect.y + wish.rect.height, ...existing.map((b) => b.rect.y + b.rect.height))
  return { sheetId: wish.sheetId, anchor: { x: wish.rect.x + GROUP_PADDING.side, y: bottom + 40 + GROUP_PADDING.top }, maxWidth: 1000 }
}
