/* WishService (许愿格详细设计 §3, §13, §14): the window's side of wish runs. The service does the
 * work — snapshot, context map, model and program, validation, the plan — and keeps runs and
 * candidates across reloads; this class starts stages, follows their progress, asks for previews
 * with the user's choices, applies the previewed plan (into this window's undo stack) and cancels.
 *
 * The Mock executor stays what it was (phase two D8): an HTML extension that runs in the browser
 * on a snapshot of the inputs. Its results go to the service as a candidate and are planned and
 * applied exactly like a real run's, so there is one set of result-writing rules.
 *
 * Nothing here runs because of change events: every run is started by the user. */

import { describeError } from '../../api/session'
import type {
  AssetContent, CellPayload, DerivedInput, EntityEnvelope, Json, KeyedContent, QueryPage, RecordContent, RichTextContent, TableSourceContent,
  WishChoices, WishPayload, WishPayloadRead, WishRunView, WishStage,
} from '../../api/types'
import type { WorkspaceStore } from '../../state/store'
import { makeBridge } from '../extensions/bridge'
import { HtmlRuntime } from '../extensions/htmlRuntime'
import { MOCK_WISH_RENDERER } from './mockWishDef'

// ---- the Mock executor contract (phase two §7.6)

export interface AnalyzeRequest {
  prompt: string
  executor_config: Record<string, Json>
  declared: { entity_id: string; type_id: string; name: string | null; title: string | null; selector?: Json; label?: string }[]
  visible: { entity_id: string; type_id: string; name: string | null; title: string | null; parent_id: string | undefined; same_folder: boolean }[]
}
export interface AnalyzeResult { context_prompt: string; inputs: { entity_id: string; label?: string; selector?: Json; version?: { mode: 'follow' | 'fixed' } }[]; warnings: string[] }
export interface InputSnapshot { entity_id: string; type_id: string; name: string | null; title: string | null; content: Json }
export interface ExecuteRequest { prompt: string; context_prompt: string; run_id: string; inputs: InputSnapshot[]; executor_config: Record<string, Json> }
export interface MockResult { name: string; type: 'richtext' | 'record' | 'table' | 'image' | 'video'; renderer?: string; title?: string; content: Json; config?: Record<string, Json>; size?: { w: number; h: number } }
export interface ExecuteResult { results: MockResult[]; warnings: string[]; assumptions: string[]; summary: string }

const ACTIVE = new Set(['queued', 'snapshotting', 'running', 'validating', 'applying'])
const WAITING = new Set(['waiting_confirmation', 'conflict', 'rejected'])

export function isActive(run: WishRunView | undefined | null): boolean { return Boolean(run && ACTIVE.has(run.state)) }
export function isWaiting(run: WishRunView | undefined | null): boolean { return Boolean(run && WAITING.has(run.state) && run.candidate) }

/** What an accepted application did, in one sentence. */
export function applyMessage(run: WishRunView): string {
  const results = run.preview?.summary.results ?? []
  const count = (a: string) => results.filter((r) => r.action === a).length
  const missing = run.preview?.summary.missing?.length ?? 0
  const parts = [`新建 ${count('create') + count('new_copy')} 项`, `更新 ${count('update')} 项`]
  if (count('keep_manual')) parts.push(`保留 ${count('keep_manual')} 项人工修改`)
  return `已应用：${parts.join('，')}${missing ? `；上次存在、本次未生成 ${missing} 项（未删除）` : ''}。可整体撤销。`
}

/** Where the run was started from (§5.2): a hint for the context map, never a dependency. */
export interface WishLocation { cell_id?: string; surface_id?: string; selection?: string[]; viewport?: { x: number; y: number; w: number; h: number } }

function astToText(node: { text?: string; content?: unknown[]; type?: string } | undefined): string {
  if (!node) return ''
  if (node.text) return node.text
  const parts = ((node.content ?? []) as { text?: string; content?: unknown[]; type?: string }[]).map(astToText)
  const block = node.type && ['paragraph', 'heading', 'list_item', 'bullet_list', 'ordered_list', 'code_block', 'blockquote', 'table_row'].includes(node.type)
  return block ? `${parts.join('')}\n` : parts.join('')
}

class HtmlExecutor {
  private runtime: HtmlRuntime | null = null
  private mountPromise: Promise<HtmlRuntime> | null = null
  private disposed = false
  private readonly store: WorkspaceStore
  private readonly defId: string
  private readonly wish: EntityEnvelope

  constructor(store: WorkspaceStore, defId: string, wish: EntityEnvelope) {
    this.store = store
    this.defId = defId
    this.wish = wish
  }

  private async mount(): Promise<HtmlRuntime> {
    if (this.disposed) throw new Error('执行器已卸载')
    this.mountPromise ??= (async () => {
      const def = await this.store.session.read<KeyedContent<{ html?: { html: string; css?: string; js?: string; api_version?: number } }>>(this.defId)
      if (this.disposed) throw new Error('执行器已卸载')
      const html = def.content.payload.html
      if (!html) throw new Error(`定义 ${this.defId} 不是 HTML 定义`)
      const cell: EntityEnvelope = { ...this.wish, entity_id: `exec:${this.wish.entity_id}`, type_id: 'buckyos.cell' }
      const payload: CellPayload = { view: { type: 'html' }, source_ref: { entity_id: this.wish.entity_id }, title: this.wish.title ?? undefined }
      const runtime = new HtmlRuntime(html, makeBridge(this.store, { cell, payload, keyRevs: {}, source: this.wish, mode: 'edit', extra: { role: 'executor', wish_id: this.wish.entity_id } }))
      this.runtime = runtime
      runtime.onCrash = (message) => {
        this.runtime = null
        this.mountPromise = null
        this.store.notify('error', `执行器出错：${message}`)
      }
      await runtime.mount(null)
      return runtime
    })()
    try { return await this.mountPromise } catch (error) { this.mountPromise = null; throw error }
  }

  async analyze(request: AnalyzeRequest): Promise<AnalyzeResult> {
    const result = await (await this.mount()).request('analyze', request as unknown as Json) as unknown as AnalyzeResult
    if (!result || typeof result.context_prompt !== 'string' || !Array.isArray(result.inputs)) throw new Error('执行器的分析结果格式不正确')
    return { ...result, warnings: Array.isArray(result.warnings) ? result.warnings : [] }
  }

  async execute(request: ExecuteRequest): Promise<ExecuteResult> {
    const result = await (await this.mount()).request('execute', request as unknown as Json, 60_000) as unknown as ExecuteResult
    if (!result || !Array.isArray(result.results)) throw new Error('执行器的执行结果格式不正确')
    return { ...result, warnings: result.warnings ?? [], assumptions: result.assumptions ?? [], summary: result.summary ?? '' }
  }

  dispose() { this.disposed = true; this.runtime?.dispose(); this.runtime = null; this.mountPromise = null }
}

export type WishBusy = 'analyzing' | 'executing' | 'rerunning' | 'repairing' | 'previewing' | 'applying' | 'cancelling'

export class WishService {
  private readonly store: WorkspaceStore
  /** The run each wish shows (the latest one this window knows). */
  private readonly runs = new Map<string, WishRunView>()
  private readonly polling = new Set<string>()
  /** Analysis runs this window started: written back as soon as they are ready (§7.3). */
  private readonly autoApply = new Set<string>()
  private readonly restored = new Set<string>()
  private readonly executors = new Map<string, HtmlExecutor>()
  private readonly listeners = new Set<() => void>()
  private disposed = false
  private version = 0
  readonly subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener) } }
  snapshot = () => this.version
  readonly busy = new Map<string, WishBusy>()
  /** The last error of a wish's flow, shown by its panel. */
  readonly errors = new Map<string, string>()

  constructor(store: WorkspaceStore) {
    this.store = store
  }

  private changed() { this.version += 1; for (const listener of [...this.listeners]) listener() }

  run(wishId: string): WishRunView | undefined { return this.runs.get(wishId) }

  private setRun(run: WishRunView) {
    this.runs.set(run.wish_id, run)
    this.changed()
  }

  private async guard<T>(wishId: string, busy: WishBusy, work: () => Promise<T>): Promise<T> {
    this.busy.set(wishId, busy)
    this.errors.delete(wishId)
    this.changed()
    try {
      return await work()
    } catch (error) {
      this.errors.set(wishId, describeError(error))
      throw error
    } finally {
      this.busy.delete(wishId)
      this.changed()
    }
  }

  /** Fixed parameters of a run: today in the user's calendar and time zone (§7.2). */
  private request(): Record<string, Json> {
    const now = new Date()
    const pad = (n: number) => String(n).padStart(2, '0')
    return { today: `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`, timezone: Intl.DateTimeFormat().resolvedOptions().timeZone ?? 'UTC' }
  }

  /** The task location: the wish's Block, the canvas selection made before it, the viewport. */
  location(wishId: string, cellId?: string): WishLocation {
    const focus = this.store.canvasFocus
    const cell = cellId ?? this.store.outline.all().find((e) => e.type_id === 'buckyos.cell' && e.source_id === wishId && !e.deleted && (!focus.surfaceId || this.store.outline.ancestors(e.entity_id).includes(focus.surfaceId)))?.entity_id
    const loc: WishLocation = {}
    if (cell) loc.cell_id = cell
    if (focus.surfaceId) loc.surface_id = focus.surfaceId
    const selection = focus.selection.filter((id) => id !== cell)
    if (selection.length) loc.selection = selection
    if (focus.viewport) loc.viewport = focus.viewport
    return loc
  }

  /** Pick up the latest run of a wish after a reload (§13.2: closing the page cancels nothing). */
  async restore(wishId: string): Promise<void> {
    if (this.restored.has(wishId)) return
    this.restored.add(wishId)
    try {
      const runs = await this.store.session.wishList(wishId, 5)
      const latest = runs[0]
      if (!latest || this.runs.has(wishId)) return
      this.setRun(isActive(latest) || isWaiting(latest) ? await this.store.session.wishGet(latest.run_id) : latest)
      if (isActive(latest)) void this.poll(latest.run_id, wishId)
    } catch { /* offline or no access: nothing to restore */ }
  }

  async history(wishId: string): Promise<WishRunView[]> {
    return this.store.session.wishList(wishId, 10)
  }

  private async poll(runId: string, wishId: string): Promise<WishRunView> {
    if (this.polling.has(runId)) {
      while (this.polling.has(runId) && !this.disposed) await new Promise((resolve) => window.setTimeout(resolve, 200))
      return this.runs.get(wishId) ?? await this.store.session.wishGet(runId)
    }
    this.polling.add(runId)
    try {
      let delay = 250
      for (;;) {
        if (this.disposed) throw new Error('窗口已关闭')
        const run = await this.store.session.wishGet(runId)
        if (this.runs.get(wishId)?.run_id === runId || !this.runs.has(wishId) || run.created_at >= (this.runs.get(wishId)?.created_at ?? '')) this.setRun(run)
        if (!ACTIVE.has(run.state)) return run
        await new Promise((resolve) => window.setTimeout(resolve, delay))
        delay = Math.min(1000, delay + 150)
      }
    } finally {
      this.polling.delete(runId)
    }
  }

  private async start(wishId: string, program: 'wish.xllm@1' | 'wish.mock@1', stage: WishStage, extra: Record<string, Json>, cellId?: string): Promise<WishRunView> {
    const params: Record<string, Json> = { wish_id: wishId, stage, location: this.location(wishId, cellId) as unknown as Json, request: this.request(), ...extra }
    const started = await this.store.session.wishStart(program, params)
    this.setRun(started)
    return this.poll(started.run_id, wishId)
  }

  private async payload(wishId: string): Promise<WishPayloadRead> {
    return (await this.store.session.read<WishPayloadRead>(wishId)).content
  }

  // ---- analysis

  async analyze(wishId: string, cellId?: string): Promise<WishRunView | null> {
    return this.guard(wishId, 'analyzing', async () => {
      const read = await this.payload(wishId)
      if (read.payload.executor === 'mock') { await this.analyzeMock(wishId, read); return null }
      if (read.payload.executor !== 'xllm') throw new Error(`执行器 ${read.payload.executor} 本期未接入`)
      this.autoApply.add(wishId)
      const run = await this.start(wishId, 'wish.xllm@1', 'analyze', {}, cellId)
      if (run.state === 'waiting_confirmation' && run.preview) {
        // clicking 分析 is the write-back (§7.3): no second confirmation
        if (!run.preview.ready) throw new Error(`分析没有写回：${run.preview.summary.problems.join('；')}`)
        await this.applyRun(run, run.preview.plan_digest, `分析许愿格`)
      } else if (run.state === 'failed') {
        throw new Error(run.error?.detail ?? '分析失败')
      }
      return this.runs.get(wishId) ?? run
    })
  }

  private executorFor(wish: EntityEnvelope): HtmlExecutor {
    const existing = this.executors.get(wish.entity_id)
    if (existing) return existing
    const def = this.store.outline.all().find((e) => e.type_id === 'buckyos.block-def' && e.def_id === MOCK_WISH_RENDERER && !e.deleted)
    if (!def) throw new Error('工作区中没有 Mock 许愿格的定义实体（def_id buckyos.mock-wish）')
    const created = new HtmlExecutor(this.store, def.entity_id, wish)
    this.executors.set(wish.entity_id, created)
    return created
  }

  private brief(e: EntityEnvelope) { return { entity_id: e.entity_id, type_id: e.type_id, name: e.name, title: e.title ?? null } }

  private async analyzeMock(wishId: string, read: WishPayloadRead) {
    const wish = this.store.outline.get(wishId)
    if (!wish) throw new Error('许愿格不存在')
    const payload = read.payload
    const declared = (payload.inputs ?? []).flatMap((input) => {
      const e = this.store.outline.get(input.entity_id)
      return e ? [{ ...this.brief(e), selector: (input.selector as unknown as Json) ?? undefined, label: input.label }] : []
    })
    const ancestors = new Set(this.store.outline.ancestors(wishId))
    const visible = this.store.outline.all()
      .filter((e) => !e.deleted && e.entity_id !== wishId && e.type_id !== 'buckyos.cell' && e.type_id !== 'buckyos.block-def' && !ancestors.has(e.entity_id) && !e.system
        && (e.type_id !== 'buckyos.container' || e.kind === 'folder') && this.store.outline.ancestors(e.entity_id).includes('data')
        && e.derived?.wish_id !== wishId && !this.store.outline.childrenOf(e.entity_id).some((c) => c.derived?.wish_id === wishId))
      .map((e) => ({ ...this.brief(e), parent_id: e.parent_id, same_folder: e.parent_id === wish.parent_id }))
    const result = await this.executorFor(wish).analyze({ prompt: payload.prompt, executor_config: payload.executor_config ?? {}, declared, visible })
    const problems: string[] = []
    const inputs = result.inputs.filter((input) => {
      const e = this.store.outline.get(input.entity_id)
      if (!e || e.deleted) { problems.push(`输入 ${input.entity_id} 不存在`); return false }
      if (!e.capabilities.includes('read')) { problems.push(`输入 ${input.entity_id} 不可读`); return false }
      return input.entity_id !== wishId
    }).map((input) => ({ entity_id: input.entity_id, ...(input.label ? { label: input.label } : {}), version: { mode: 'follow' as const } }))
    // the Mock decides its results while executing: a dynamic contract (§8.5)
    const analysis = {
      schema_version: 'wish.analysis.v2', status: 'ready', prompt: payload.prompt, context_prompt: result.context_prompt,
      output_contract: { results: [], dynamic: true }, checks: [], blockers: [], warnings: [...result.warnings, ...problems], executor: 'mock', at: new Date().toISOString(),
    }
    const outcome = await this.store.submit({
      editId: `wish:${wishId}:analysis`, label: `分析许愿格 ${wish.title ?? wish.name ?? wishId}`,
      operations: [{ op: 'entity.set_keys', entity_id: wishId, keys: [
        { key: 'analysis', value: analysis as unknown as Json, expect: { rev: read.key_revs.analysis ?? 0 } },
        { key: 'inputs', value: inputs as unknown as Json, expect: { rev: read.key_revs.inputs ?? 0 } },
      ] }],
    })
    if (outcome.status === 'conflict' || outcome.status === 'rejected') throw new Error(`分析结果没有写回许愿格（${outcome.code}）`)
  }

  // ---- execution

  async execute(wishId: string, options: { cellId?: string; feedback?: string; parentRunId?: string } = {}): Promise<WishRunView> {
    return this.guard(wishId, 'executing', async () => {
      const read = await this.payload(wishId)
      if (read.payload.executor === 'mock') return this.executeMock(wishId, read.payload, options.cellId)
      const extra: Record<string, Json> = {}
      if (options.parentRunId) extra.parent_run_id = options.parentRunId
      if (options.feedback) extra.feedback = options.feedback
      const run = await this.start(wishId, 'wish.xllm@1', 'execute', extra, options.cellId)
      if (run.state === 'failed') throw new Error(run.error?.detail ?? '执行失败')
      return run
    })
  }

  async rerunProgram(wishId: string, cellId?: string): Promise<WishRunView> {
    return this.guard(wishId, 'rerunning', async () => {
      const run = await this.start(wishId, 'wish.xllm@1', 'rerun_program', {}, cellId)
      if (run.state === 'failed') throw new Error(run.error?.detail ?? '只重跑程序失败')
      return run
    })
  }

  async repairProgram(wishId: string, failedRunId: string, cellId?: string): Promise<WishRunView> {
    return this.guard(wishId, 'repairing', async () => {
      const run = await this.start(wishId, 'wish.xllm@1', 'repair_program', { parent_run_id: failedRunId }, cellId)
      if (run.state === 'failed') throw new Error(run.error?.detail ?? '修复程序失败')
      return run
    })
  }

  /** A complete input snapshot for the Mock and the versions it read (the run's read set). */
  private async snapshotInput(entity: EntityEnvelope, readSet: DerivedInput[], depth: number): Promise<InputSnapshot> {
    const store = this.store
    const id = entity.entity_id
    const base = { entity_id: id, type_id: entity.type_id, name: entity.name, title: entity.title ?? null }
    const push = (sel: Json | undefined, current: number | string) => {
      readSet.push({ entity_id: id, ...(sel ? { selector: sel as unknown as DerivedInput['selector'] } : {}), version: { mode: 'follow', ...(typeof current === 'number' ? { rev: current } : { hash: current }) } })
    }
    switch (entity.type_id) {
      case 'buckyos.table-source': {
        const meta = await store.session.read<TableSourceContent>(id)
        const rows: QueryPage['rows'] = []
        let cursor: string | undefined
        for (let page = 0; page < 50; page++) {
          const result: QueryPage = await store.session.query({ source_id: id, limit: 1000, ...(cursor ? { cursor } : {}) })
          rows.push(...result.rows)
          cursor = result.next_cursor ?? undefined
          if (!cursor) break
        }
        push({ kind: 'table_members' }, meta.content.members_rev)
        for (const field of meta.content.fields) push({ kind: 'table_field_values', field_id: field.field_id }, field.values_rev)
        return { ...base, content: { fields: meta.content.fields.map((f) => ({ field_id: f.field_id, name: f.name, type: f.type })), rows: rows.map((r) => r.values) } as unknown as Json }
      }
      case 'buckyos.richtext': {
        const read = await store.session.read<RichTextContent>(id)
        push(undefined, read.content_rev)
        return { ...base, content: { text: astToText(read.content.content).trim() } }
      }
      case 'buckyos.record': {
        const read = await store.session.read<RecordContent>(id)
        push(undefined, read.content_rev)
        return { ...base, content: { schema: read.content.schema as unknown as Json, props: read.content.props } }
      }
      case 'buckyos.asset-ref': {
        const read = await store.session.read<AssetContent>(id)
        push(undefined, read.content_rev)
        let svg: string | undefined
        if (read.content.payload.media_type === 'image/svg+xml' && read.content.availability === 'available') {
          try { svg = await (await store.session.fetchAsset(read.content.payload.object_id)).text() } catch { svg = undefined }
        }
        return { ...base, content: { object_id: read.content.payload.object_id, media_type: read.content.payload.media_type ?? null, file_name: read.content.payload.file_name ?? null, ...(svg ? { svg } : {}) } }
      }
      case 'buckyos.container': {
        const children = store.outline.childrenOf(id).filter((c) => c.type_id !== 'buckyos.cell')
        const snaps: InputSnapshot[] = []
        for (const child of children) if (depth < 2) snaps.push(await this.snapshotInput(child, readSet, depth + 1))
        return { ...base, content: { children: snaps as unknown as Json } }
      }
      default: {
        const read = await store.session.read<Json>(id)
        push(undefined, read.content_rev)
        return { ...base, content: (read as unknown as { content: Json }).content }
      }
    }
  }

  private async executeMock(wishId: string, payload: WishPayload, cellId?: string): Promise<WishRunView> {
    const wish = this.store.outline.get(wishId)
    if (!wish) throw new Error('许愿格不存在')
    const fresh = (await this.store.session.freshness([wishId]))[0]
    if (!payload.analysis || fresh?.needs_analysis) throw new Error('提示词或输入在分析之后改变了：需要先重新分析')
    for (const input of payload.inputs ?? []) {
      const e = this.store.outline.get(input.entity_id)
      if (!e || e.deleted) throw new Error(`输入 ${input.label ?? input.entity_id} 不存在或已删除，不能执行`)
      if (!e.capabilities.includes('read')) throw new Error(`输入 ${input.label ?? input.entity_id} 不可读，不能执行`)
      if (e.type_id === 'buckyos.wish') throw new Error('许愿格不能直接以许愿格为输入（请引用它的结果）')
      if (e.derived?.wish_id === wishId || this.store.outline.childrenOf(e.entity_id).some((c) => c.derived?.wish_id === wishId)) throw new Error(`输入 ${input.label ?? input.entity_id} 是本许愿格的结果：输入成环，拒绝执行`)
    }
    const readSet: DerivedInput[] = []
    const snapshots: InputSnapshot[] = []
    for (const input of payload.inputs ?? []) snapshots.push(await this.snapshotInput(this.store.outline.get(input.entity_id)!, readSet, 0))
    const result = await this.executorFor(wish).execute({ prompt: payload.prompt, context_prompt: payload.analysis.context_prompt, run_id: `mock-${Date.now().toString(36)}`, inputs: snapshots, executor_config: payload.executor_config ?? {} })
    const provided = { results: result.results, read_set: readSet, summary: result.summary, warnings: result.warnings, assumptions: result.assumptions }
    const run = await this.start(wishId, 'wish.mock@1', 'execute', { provided: provided as unknown as Json }, cellId)
    if (run.state === 'failed') throw new Error(run.error?.detail ?? '模拟执行失败')
    return run
  }

  // ---- preview and application (§11)

  /** Ask for the plan of `choices` (kept on the service under its digest). */
  async preview(run: WishRunView, choices: WishChoices): Promise<WishRunView> {
    return this.guard(run.wish_id, 'previewing', async () => {
      const next = await this.store.session.wishGet(run.run_id, choices)
      this.setRun(next)
      return next
    })
  }

  /** Apply exactly the previewed plan; the accepted commit joins this window's undo stack (§11.4). */
  async apply(run: WishRunView, planDigest: string): Promise<WishRunView> {
    const wish = this.store.outline.get(run.wish_id)
    return this.guard(run.wish_id, 'applying', () => this.applyRun(run, planDigest, `应用许愿格结果 ${wish?.title ?? wish?.name ?? run.wish_id}${run.simulated ? '（模拟）' : ''}`))
  }

  private async applyRun(run: WishRunView, planDigest: string, label: string): Promise<WishRunView> {
    const editId = `wish:${run.wish_id}:apply`
    this.store.edits.set({ id: editId, label, state: 'unsaved' })
    let next: WishRunView
    try {
      next = await this.store.session.wishApply(run.run_id, planDigest)
    } catch (error) {
      this.store.edits.remove(editId)
      throw error
    }
    this.setRun(next)
    const commit = next.applied?.commit
    if (next.applied?.status === 'accepted' && commit && commit.status === 'accepted') {
      this.store.undo.pushCommit(commit.commit_id, label)
      this.store.edits.set({ id: editId, label, state: 'committed' })
      await this.store.session.whenApplied(commit.seq)
    } else {
      this.store.edits.remove(editId)
    }
    return next
  }

  async cancel(run: WishRunView): Promise<void> {
    await this.guard(run.wish_id, 'cancelling', async () => {
      await this.store.session.procCancel(run.run_id)
      this.setRun(await this.store.session.wishGet(run.run_id))
    })
  }

  /** Put a candidate aside: nothing is written; the run stays in the service's list. */
  discard(wishId: string) {
    this.runs.delete(wishId)
    this.changed()
  }

  // ---- the program (§9.1)

  async programSource(wishId: string): Promise<string | null> {
    const read = await this.payload(wishId)
    const source = read.payload.program?.source
    if (!source) return null
    return (await this.store.session.fetchAsset(source)).text()
  }

  /** Save an edited program: a new source asset; results then show as generated under another configuration. */
  async saveProgram(wishId: string, source: string): Promise<void> {
    const read = await this.payload(wishId)
    const bytes = new TextEncoder().encode(source)
    const digest = Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', bytes))).map((b) => b.toString(16).padStart(2, '0')).join('')
    const uploaded = await this.store.session.uploadAsset(new Blob([bytes], { type: 'text/plain' }), 'main.js')
    const program = { language: 'js', api_version: 2, source: uploaded.object_id, digest, produces: read.payload.program?.produces ?? [], edited_by: this.store.session.principal ?? 'user' }
    const outcome = await this.store.submit({
      editId: `wish:${wishId}:program`, label: '修改许愿格程序',
      operations: [{ op: 'entity.set_keys', entity_id: wishId, keys: [{ key: 'program', value: program as unknown as Json, expect: { rev: read.key_revs.program ?? 0 } }] }],
    })
    if (outcome.status === 'conflict' || outcome.status === 'rejected') throw new Error(`程序没有保存（${outcome.code}）`)
  }

  dispose() {
    this.disposed = true
    for (const executor of this.executors.values()) executor.dispose()
    this.executors.clear()
    this.runs.clear()
  }
}
