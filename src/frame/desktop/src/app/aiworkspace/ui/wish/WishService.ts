/* WishService (phase two §7): the host side of a wish. Two passes — analyze (prompt + visible
 * context → context prompt + input references, written back to the wish) and execute (context prompt
 * + a read set fixed at the start → candidate results) — then an explicit application that writes
 * result content, result Blocks and dependency records in one commit guarded by the read set.
 * Executors never write the document; the Mock executor runs in the browser as an HTML extension.
 *
 * Only explicit execution exists: nothing here listens to change events to re-run. */

import { randomId } from '../../api/ids'
import type { ReadOk } from '../../api/session'
import type {
  AssetContent, CellPayload, CommitOutcome, DerivedInput, DerivedRecord, EntityEnvelope, Json, KeyedContent, Operation, Placement, QueryPage,
  RecordContent, RichTextContent, TableSourceContent, WishLastRun, WishPayload, WishPayloadRead,
} from '../../api/types'
import type { WorkspaceStore } from '../../state/store'
import { makeBridge } from '../extensions/bridge'
import { HtmlRuntime } from '../extensions/htmlRuntime'
import { MOCK_WISH_RENDERER } from './mockWishDef'

// ---- executor contract (§7.6)

export interface AnalyzeRequest {
  prompt: string
  executor_config: Record<string, Json>
  declared: { entity_id: string; type_id: string; name: string | null; title: string | null; selector?: Json; label?: string }[]
  visible: { entity_id: string; type_id: string; name: string | null; title: string | null; parent_id: string | undefined; same_folder: boolean }[]
}
export interface AnalyzeResult { context_prompt: string; inputs: { entity_id: string; label?: string; selector?: Json; version?: { mode: 'follow' | 'fixed' } }[]; warnings: string[] }

export interface InputSnapshot { entity_id: string; type_id: string; name: string | null; title: string | null; content: Json }
export interface ExecuteRequest { prompt: string; context_prompt: string; run_id: string; inputs: InputSnapshot[]; executor_config: Record<string, Json> }

export type ResultContent =
  | { markdown: string }
  | { schema: { properties: { key: string; name: string; type: string; scale?: number }[] }; props: Record<string, Json> }
  | { fields: { field_id: string; name: string; type: string }[]; rows: Record<string, Json>[] }
  | { svg: string; width: number; height: number; alt?: string; caption?: string }
  | { frames: { svg: string; durationMs: number; caption?: string }[]; width: number; height: number; caption?: string }
export interface ResultSpec {
  name: string
  type: 'richtext' | 'record' | 'table' | 'image' | 'video'
  renderer?: string
  title?: string
  content: ResultContent
  config?: Record<string, Json>
  size?: { w: number; h: number }
  position?: { x: number; y: number }
}
export interface ExecuteResult { results: ResultSpec[]; warnings: string[]; assumptions: string[]; summary: string; kind?: string; simulated?: boolean }

export interface Executor {
  readonly id: string
  analyze(request: AnalyzeRequest): Promise<AnalyzeResult>
  execute(request: ExecuteRequest): Promise<ExecuteResult>
  dispose(): void
}

/** A candidate produced by `execute`, kept in memory until applied or discarded (§7.2). */
export interface Candidate {
  wishId: string
  runId: string
  executor: string
  prompt: string
  contextPrompt: string
  /** The read set fixed when execution started: preconditions of the application and the dependency record. */
  readSet: DerivedInput[]
  preconditions: { target: { entity_id: string; selector?: Json }; expect: { rev: number } | { hash: string } }[]
  inputs: InputSnapshot[]
  result: ExecuteResult
  executedAt: string
  simulated: boolean
}

export type ManualChoice = 'keep' | 'replace' | 'new'

export interface ApplyPlan {
  operations: Operation[]
  preconditions: Candidate['preconditions']
  /** Results that exist and were edited by hand since generation: the user decides each (§7.5 rule 7). */
  manual: { name: string; entityId: string }[]
  produced: string[]
  missing: string[]
  created: string[]
  updated: string[]
  skipped: string[]
  groupId: string
  uploads: number
}

function fnv(text: string): number {
  let h = 0x811c9dc5
  for (let i = 0; i < text.length; i++) { h ^= text.charCodeAt(i); h = Math.imul(h, 0x01000193) >>> 0 }
  return h >>> 0
}
/** A stable, id-safe slug of a result name (`^[a-z0-9][a-z0-9_-]*$`). */
export function slugOf(name: string): string {
  const ascii = name.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '')
  const tail = fnv(name).toString(36)
  return (ascii ? `${ascii.slice(0, 20)}-${tail}` : `r-${tail}`).replace(/^[^a-z0-9]/, 'r')
}

function markdownToAst(markdown: string, prefix: string): Json {
  const content: Json[] = []
  let n = 0
  const id = () => `${prefix}-${++n}`
  let list: Json[] | null = null
  const flush = () => { if (list) { content.push({ type: 'bullet_list', attrs: { block_id: id() }, content: list }); list = null } }
  for (const raw of markdown.split('\n')) {
    const line = raw.trimEnd()
    if (line.trim() === '') { flush(); continue }
    const heading = /^(#{1,3})\s+(.*)$/.exec(line)
    if (heading) { flush(); content.push({ type: 'heading', attrs: { block_id: id(), level: heading[1].length }, content: [{ type: 'text', text: heading[2] }] }); continue }
    const bullet = /^[-*]\s+(.*)$/.exec(line)
    if (bullet) { list ??= []; list.push({ type: 'list_item', attrs: { block_id: id() }, content: [{ type: 'paragraph', attrs: { block_id: id() }, content: [{ type: 'text', text: bullet[1] }] }] }); continue }
    flush()
    content.push({ type: 'paragraph', attrs: { block_id: id() }, content: [{ type: 'text', text: line.replace(/\*\*/g, '') }] })
  }
  flush()
  if (content.length === 0) content.push({ type: 'paragraph', attrs: { block_id: id() } })
  return { type: 'doc', content }
}

function astToText(node: { text?: string; content?: unknown[]; type?: string } | undefined): string {
  if (!node) return ''
  if (node.text) return node.text
  const parts = ((node.content ?? []) as { text?: string; content?: unknown[]; type?: string }[]).map(astToText)
  const block = node.type && ['paragraph', 'heading', 'list_item', 'bullet_list', 'ordered_list'].includes(node.type)
  return block ? `${parts.join('')}\n` : parts.join('')
}

// ---- the Mock executor: an HTML extension run headless (D8)

class HtmlExecutor implements Executor {
  readonly id: string
  private runtime: HtmlRuntime | null = null
  private readonly store: WorkspaceStore
  private readonly defId: string
  private readonly wish: EntityEnvelope
  private mountPromise: Promise<HtmlRuntime> | null = null

  constructor(id: string, store: WorkspaceStore, defId: string, wish: EntityEnvelope) {
    this.id = id
    this.store = store
    this.defId = defId
    this.wish = wish
  }

  private async mount(): Promise<HtmlRuntime> {
    if (this.runtime) return this.runtime
    this.mountPromise ??= (async () => {
      const def = await this.store.session.read<KeyedContent<{ html?: { html: string; css?: string; js?: string } }>>(this.defId)
      const html = def.content.payload.html
      if (!html) throw new Error(`定义 ${this.defId} 不是 HTML 定义`)
      const cell: EntityEnvelope = { ...this.wish, entity_id: `exec:${this.wish.entity_id}`, type_id: 'buckyos.cell' }
      const payload: CellPayload = { view: { type: 'html' }, source_ref: { entity_id: this.wish.entity_id }, title: this.wish.title ?? undefined }
      const runtime = new HtmlRuntime(html, makeBridge(this.store, { cell, payload, keyRevs: {}, source: this.wish, mode: 'edit', extra: { role: 'executor', wish_id: this.wish.entity_id } }))
      runtime.onCrash = (message) => this.store.notify('error', `执行器出错：${message}`)
      await runtime.mount(null)
      this.runtime = runtime
      return runtime
    })()
    try { return await this.mountPromise } catch (error) { this.mountPromise = null; throw error }
  }

  async analyze(request: AnalyzeRequest): Promise<AnalyzeResult> {
    const runtime = await this.mount()
    const result = await runtime.request('analyze', request as unknown as Json) as unknown as AnalyzeResult
    if (!result || typeof result.context_prompt !== 'string' || !Array.isArray(result.inputs)) throw new Error('执行器的分析结果格式不正确')
    return { ...result, warnings: Array.isArray(result.warnings) ? result.warnings : [] }
  }

  async execute(request: ExecuteRequest): Promise<ExecuteResult> {
    const runtime = await this.mount()
    const result = await runtime.request('execute', request as unknown as Json, 60_000) as unknown as ExecuteResult
    if (!result || !Array.isArray(result.results)) throw new Error('执行器的执行结果格式不正确')
    for (const r of result.results) {
      if (typeof r.name !== 'string' || !r.name || !['richtext', 'record', 'table', 'image', 'video'].includes(r.type) || !r.content) throw new Error(`执行器返回了无效的结果项「${String(r.name)}」`)
    }
    return { ...result, warnings: result.warnings ?? [], assumptions: result.assumptions ?? [], summary: result.summary ?? '' }
  }

  dispose() { this.runtime?.dispose(); this.runtime = null }
}

// ---- the service

export class WishService {
  private readonly store: WorkspaceStore
  private readonly candidates = new Map<string, Candidate>()
  private readonly executors = new Map<string, Executor>()
  private readonly listeners = new Set<() => void>()
  private version = 0
  readonly subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener) } }
  snapshot = () => this.version
  /** Which wishes are busy and with what. */
  readonly busy = new Map<string, 'analyzing' | 'executing' | 'applying'>()

  constructor(store: WorkspaceStore) {
    this.store = store
  }

  private changed() { this.version += 1; for (const listener of [...this.listeners]) listener() }

  candidate(wishId: string): Candidate | undefined { return this.candidates.get(wishId) }
  discard(wishId: string) { this.candidates.delete(wishId); this.changed() }

  private executorFor(wish: EntityEnvelope, executor: string): Executor {
    const key = `${wish.entity_id}:${executor}`
    const existing = this.executors.get(key)
    if (existing) return existing
    if (executor !== 'mock') throw new Error(`执行器 ${executor} 本期未接入（只实现 mock）`)
    const def = this.store.outline.all().find((e) => e.type_id === 'buckyos.block-def' && e.def_id === MOCK_WISH_RENDERER && !e.deleted)
    if (!def) throw new Error('工作区中没有 Mock 许愿格的定义实体（def_id buckyos.mock-wish）')
    const created = new HtmlExecutor('mock', this.store, def.entity_id, wish)
    this.executors.set(key, created)
    return created
  }

  private async readWish(wishId: string): Promise<ReadOk<WishPayloadRead>> {
    return this.store.session.read<WishPayloadRead>(wishId)
  }

  private brief(e: EntityEnvelope) { return { entity_id: e.entity_id, type_id: e.type_id, name: e.name, title: e.title ?? null } }

  // ---- pass 1

  async analyze(wishId: string): Promise<void> {
    const wish = this.store.outline.get(wishId)
    if (!wish) throw new Error('许愿格不存在')
    this.busy.set(wishId, 'analyzing'); this.changed()
    try {
      const read = await this.readWish(wishId)
      const payload = read.content.payload
      const executor = this.executorFor(wish, payload.executor)
      const declared = (payload.inputs ?? []).flatMap((input) => {
        const e = this.store.outline.get(input.entity_id)
        return e ? [{ ...this.brief(e), selector: (input.selector as unknown as Json) ?? undefined, label: input.label }] : []
      })
      // the visible context: data of the data tree, excluding the wish's own ancestors, system folders and
      // anything this wish produced (an input from its own results would be a cycle, §7.5 rule 5)
      const ancestors = new Set(this.store.outline.ancestors(wishId))
      const visible = this.store.outline.all()
        .filter((e) => !e.deleted && e.entity_id !== wishId && e.type_id !== 'buckyos.cell' && e.type_id !== 'buckyos.block-def' && !ancestors.has(e.entity_id) && !e.system
          && (e.type_id !== 'buckyos.container' || e.kind === 'folder') && this.store.outline.ancestors(e.entity_id).includes('data')
          && e.derived?.wish_id !== wishId && !this.store.outline.childrenOf(e.entity_id).some((c) => c.derived?.wish_id === wishId))
        .map((e) => ({ ...this.brief(e), parent_id: e.parent_id, same_folder: e.parent_id === wish.parent_id }))
      const result = await executor.analyze({ prompt: payload.prompt, executor_config: payload.executor_config ?? {}, declared, visible })
      // the host checks the inputs the executor proposes: they must exist and be readable (§7.2)
      const problems: string[] = []
      const inputs = result.inputs.filter((input) => {
        const e = this.store.outline.get(input.entity_id)
        if (!e || e.deleted) { problems.push(`输入 ${input.entity_id} 不存在`); return false }
        if (!e.capabilities.includes('read')) { problems.push(`输入 ${input.entity_id} 不可读`); return false }
        if (input.entity_id === wishId) { problems.push('许愿格不能以自己为输入'); return false }
        return true
      }).map((input) => ({ entity_id: input.entity_id, ...(input.selector ? { selector: input.selector } : {}), version: input.version ?? { mode: 'follow' as const }, ...(input.label ? { label: input.label } : {}) }))
      const analysis = { prompt: payload.prompt, context_prompt: result.context_prompt, at: new Date().toISOString(), warnings: [...result.warnings, ...problems], executor: payload.executor }
      const outcome = await this.store.submit({
        editId: `wish:${wishId}:analysis`, label: `分析许愿格 ${wish.title ?? wish.name ?? wishId}`,
        operations: [{ op: 'entity.set_keys', entity_id: wishId, keys: [
          { key: 'analysis', value: analysis as unknown as Json, expect: { rev: read.content.key_revs.analysis ?? 0 } },
          { key: 'inputs', value: inputs as unknown as Json, expect: { rev: read.content.key_revs.inputs ?? 0 } },
        ] }],
      })
      if (outcome.status === 'conflict' || outcome.status === 'rejected') throw new Error(`分析结果没有写回许愿格（${outcome.code}）`)
    } finally {
      this.busy.delete(wishId); this.changed()
    }
  }

  // ---- pass 2: read set + snapshot, then the executor

  private async snapshotInput(entity: EntityEnvelope, selector: Json | undefined, readSet: DerivedInput[], preconditions: Candidate['preconditions'], depth: number): Promise<InputSnapshot> {
    const store = this.store
    const id = entity.entity_id
    const base = { entity_id: id, type_id: entity.type_id, name: entity.name, title: entity.title ?? null }
    const push = (sel: Json | undefined, current: number | string) => {
      const target = { entity_id: id, ...(sel ? { selector: sel } : {}) }
      readSet.push({ entity_id: id, ...(sel ? { selector: sel as unknown as DerivedInput['selector'] } : {}), version: { mode: 'follow', ...(typeof current === 'number' ? { rev: current } : { hash: current }) } })
      preconditions.push({ target, expect: typeof current === 'number' ? { rev: current } : { hash: current } })
    }
    switch (entity.type_id) {
      case 'buckyos.table-source': {
        const meta = await store.session.read<TableSourceContent>(id)
        const page: QueryPage = await store.session.query({ source_id: id, limit: 1000, consistency: 'best_effort' })
        push({ kind: 'table_members' }, meta.content.members_rev)
        for (const field of meta.content.fields) push({ kind: 'table_field_values', field_id: field.field_id }, field.values_rev)
        return { ...base, content: { fields: meta.content.fields.map((f) => ({ field_id: f.field_id, name: f.name, type: f.type })), rows: page.rows.map((r) => r.values) } as unknown as Json }
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
        return { ...base, content: { object_id: read.content.payload.object_id, media_type: read.content.payload.media_type ?? null, file_name: read.content.payload.file_name ?? null, alt: typeof read.content.payload.image === 'object' ? null : null, ...(svg ? { svg } : {}) } }
      }
      case 'buckyos.container': {
        // a result folder as input (the previous stage's output): its children, one level deep
        const children = store.outline.childrenOf(id).filter((c) => c.type_id !== 'buckyos.cell')
        const snaps: InputSnapshot[] = []
        for (const child of children) if (depth < 2) snaps.push(await this.snapshotInput(child, undefined, readSet, preconditions, depth + 1))
        return { ...base, content: { children: snaps as unknown as Json } }
      }
      default: {
        const read = await store.session.read<Json>(id)
        push(undefined, read.content_rev)
        return { ...base, content: (read as unknown as { content: Json }).content }
      }
    }
    void selector
  }

  async execute(wishId: string): Promise<Candidate> {
    const wish = this.store.outline.get(wishId)
    if (!wish) throw new Error('许愿格不存在')
    this.busy.set(wishId, 'executing'); this.changed()
    try {
      const read = await this.readWish(wishId)
      const payload = read.content.payload
      if (!payload.analysis || payload.analysis.prompt !== payload.prompt) throw new Error('提示词已修改，需要先重新分析')
      const inputs = payload.inputs ?? []
      if (inputs.length === 0) throw new Error('没有输入：先分析，或手动添加输入')
      for (const input of inputs) {
        const e = this.store.outline.get(input.entity_id)
        if (!e || e.deleted) throw new Error(`输入 ${input.label ?? input.entity_id} 不存在或已删除，不能执行`)
        if (!e.capabilities.includes('read')) throw new Error(`输入 ${input.label ?? input.entity_id} 不可读，不能执行`)
        if (e.type_id === 'buckyos.wish') throw new Error('许愿格不能直接以许愿格为输入（请引用它的结果）')
      }
      // an input that was produced by this wish is a cycle (§7.5 rule 5)
      for (const input of inputs) {
        const e = this.store.outline.get(input.entity_id)
        const derived = e?.derived
        if (derived?.wish_id === wishId) throw new Error(`输入 ${input.label ?? input.entity_id} 是本许愿格的结果：输入成环，拒绝执行`)
        for (const child of e ? this.store.outline.childrenOf(e.entity_id) : []) if (child.derived?.wish_id === wishId) throw new Error(`输入文件夹包含本许愿格的结果：输入成环，拒绝执行`)
      }
      const executor = this.executorFor(wish, payload.executor)
      const runId = `run_${randomId()}`
      const readSet: DerivedInput[] = []
      const preconditions: Candidate['preconditions'] = []
      const snapshots: InputSnapshot[] = []
      for (const input of inputs) {
        const e = this.store.outline.get(input.entity_id)!
        snapshots.push(await this.snapshotInput(e, input.selector as unknown as Json, readSet, preconditions, 0))
      }
      // the wish's own last run is part of the read set: of two concurrent applications only one is accepted (§7.2)
      preconditions.push({ target: { entity_id: wishId, selector: { kind: 'doc_key', key: 'last_run' } }, expect: { rev: read.content.key_revs.last_run ?? 0 } })
      const result = await executor.execute({ prompt: payload.prompt, context_prompt: payload.analysis.context_prompt, run_id: runId, inputs: snapshots, executor_config: payload.executor_config ?? {} })
      const candidate: Candidate = { wishId, runId, executor: payload.executor, prompt: payload.prompt, contextPrompt: payload.analysis.context_prompt, readSet, preconditions, inputs: snapshots, result, executedAt: new Date().toISOString(), simulated: payload.executor === 'mock' || Boolean(result.simulated) }
      this.candidates.set(wishId, candidate)
      return candidate
    } finally {
      this.busy.delete(wishId); this.changed()
    }
  }

  // ---- application (§7.3)

  private surfaceOf(wishId: string, payload: WishPayload): { surfaceId: string; anchor: Placement | null } {
    const cells = this.store.outline.all().filter((e) => e.type_id === 'buckyos.cell' && e.source_id === wishId && !e.deleted)
    const wanted = payload.output?.surface_id
    const cell = cells.find((c) => !wanted || this.store.outline.ancestors(c.entity_id).includes(wanted)) ?? cells[0]
    if (cell) {
      const surface = this.store.outline.ancestors(cell.entity_id).map((id) => this.store.outline.get(id)).find((e) => e?.kind === 'surface')
      return { surfaceId: surface?.entity_id ?? wanted ?? '', anchor: cell.placement ?? null }
    }
    return { surfaceId: wanted ?? this.store.outline.childrenOf('surfaces')[0]?.entity_id ?? '', anchor: null }
  }

  /** Build the commit of a candidate. `choices` answers the manual-modification question per result name. */
  async plan(candidate: Candidate, choices: Record<string, ManualChoice> = {}): Promise<ApplyPlan> {
    const store = this.store
    const wishId = candidate.wishId
    const wishRead = await this.readWish(wishId)
    const payload = wishRead.content.payload
    const mode = payload.output_mode ?? 'overwrite'
    const wishEntity = store.outline.get(wishId)!
    const containerId = payload.output?.container_id ?? wishEntity.parent_id ?? 'data'
    const outputName = payload.output?.name ?? '结果'
    const runSeq = ((payload.last_run as (WishLastRun & { seq?: number }) | undefined)?.seq ?? 0) + 1
    const groupId = mode === 'overwrite' ? `${wishId}-out` : `${wishId}-run${runSeq}`
    const folderId = `${groupId}-data`
    const { surfaceId, anchor } = this.surfaceOf(wishId, payload)
    const ops: Operation[] = []
    const core = store.core
    const nextKey = (parentId: string, after?: string) => core.order_key_between(after ?? store.outline.childrenOf(parentId).at(-1)?.order_key ?? undefined, undefined)
    let folderKey: string | undefined
    let blockKey: string | undefined
    const keyIn = (parentId: string, last: { v?: string }) => { last.v = nextKey(parentId, last.v); return last.v }
    const folderLast = { v: undefined as string | undefined }
    const blockLast = { v: undefined as string | undefined }
    // result folder (data tree) and result group (BlockTree)
    const folderExists = Boolean(store.outline.get(folderId)?.deleted === false)
    if (!folderExists) {
      ops.push({ op: 'entity.create', entity_id: folderId, type_id: 'buckyos.container', parent_id: containerId, order_key: nextKey(containerId), name: mode === 'overwrite' ? outputName : `${outputName} #${runSeq}`, payload: { kind: 'folder', title: mode === 'overwrite' ? outputName : `${outputName} #${runSeq}` } })
    }
    void folderKey; void blockKey
    const groupExists = Boolean(store.outline.get(groupId)?.deleted === false)
    const positions = candidate.result.results.map((r, i) => ({ x: r.position?.x ?? (i % 2) * 500, y: r.position?.y ?? Math.floor(i / 2) * 300, w: r.size?.w ?? 400, h: r.size?.h ?? 240 }))
    const extent = positions.reduce((acc, p) => ({ w: Math.max(acc.w, p.x + p.w), h: Math.max(acc.h, p.y + p.h) }), { w: 200, h: 100 })
    const groupPlacement: Placement = { x: (anchor ? anchor.x + anchor.w + 60 : 40), y: (anchor ? anchor.y : 40) + (mode === 'new' ? (runSeq - 1) * (extent.h + 80) : 0), w: extent.w + 40, h: extent.h + 60 }
    if (surfaceId && !groupExists) {
      ops.push({ op: 'entity.create', entity_id: groupId, type_id: 'buckyos.container', parent_id: surfaceId, order_key: nextKey(surfaceId), placement: groupPlacement, payload: { kind: 'group', layout: { mode: 'free' }, title: mode === 'overwrite' ? `${outputName}（${candidate.simulated ? '模拟' : '生成'}）` : `${outputName} #${runSeq}` } })
    }
    const derived = (): DerivedRecord => ({ wish_id: wishId, run_id: candidate.runId, executor: candidate.executor, inputs: candidate.readSet, generated_rev: 0, simulated: candidate.simulated, at: candidate.executedAt, output_mode: mode, group: groupId })
    const produced: string[] = []
    const created: string[] = []
    const updated: string[] = []
    const skipped: string[] = []
    const manual: ApplyPlan['manual'] = []
    let uploads = 0
    const previous = new Set((payload.last_run?.produced ?? []).map((id) => id))
    for (const [index, result] of candidate.result.results.entries()) {
      const slug = slugOf(result.name)
      let entityId = mode === 'overwrite' ? `${wishId}-r-${slug}` : `${groupId}-${slug}`
      const pos = positions[index]
      let existing = store.outline.get(entityId)
      if (existing?.deleted) existing = undefined
      const choice = choices[result.name]
      if (existing && mode === 'overwrite') {
        const freshness = await store.session.freshness([entityId]).then((items) => items[0]).catch(() => undefined)
        if (freshness?.manual_modified && !choice) { manual.push({ name: result.name, entityId }); continue }
        if (freshness?.manual_modified && choice === 'keep') { ops.push({ op: 'entity.set_derived', entity_id: entityId, derived: { ...derived(), kept_manual: true } as unknown as Json }); skipped.push(entityId); produced.push(entityId); continue }
        if (freshness?.manual_modified && choice === 'new') { entityId = `${entityId}-${runSeq}`; existing = undefined }
      }
      produced.push(entityId)
      const renderer = result.renderer ?? (result.type === 'image' ? 'asset' : result.type === 'video' ? 'sample.video' : result.type)
      const title = result.title ?? result.name
      const name = result.name
      // ---- content
      if (result.type === 'richtext') {
        const ast = markdownToAst((result.content as { markdown: string }).markdown, slug)
        if (existing) {
          const current = await store.session.read<RichTextContent>(entityId)
          const diff = JSON.parse(core.richtext_diff(entityId, JSON.stringify(current.content.content), JSON.stringify(ast), JSON.stringify(current.content.blocks))) as Operation[]
          ops.push(...diff)
          updated.push(entityId)
        } else {
          ops.push({ op: 'entity.create', entity_id: entityId, type_id: 'buckyos.richtext', parent_id: folderId, order_key: keyIn(folderId, folderLast), name, payload: { content: ast } })
          created.push(entityId)
        }
      } else if (result.type === 'record' || result.type === 'video') {
        const content = result.type === 'record'
          ? (result.content as { schema: Json; props: Record<string, Json> })
          : (() => {
              const v = result.content as { frames: { svg: string; durationMs: number; caption?: string }[]; width: number; height: number; caption?: string }
              return { schema: { properties: [{ key: 'frames', name: '帧', type: 'text' }, { key: 'duration', name: '时长(毫秒)', type: 'number' }, { key: 'caption', name: '说明', type: 'text' }, { key: 'width', name: '宽', type: 'number' }, { key: 'height', name: '高', type: 'number' }] } as Json, props: { frames: '[]', duration: v.frames.reduce((n, f) => n + f.durationMs, 0), caption: v.caption ?? '', width: v.width, height: v.height } as Record<string, Json> }
            })()
        if (result.type === 'video') {
          // frames become assets; the record keeps their ids and durations (no data URLs in the document, §7.6)
          const v = result.content as { frames: { svg: string; durationMs: number; caption?: string }[] }
          const frames: Json[] = []
          for (const frame of v.frames) {
            const uploaded = await store.session.uploadAsset(new Blob([frame.svg], { type: 'image/svg+xml' }), 'frame.svg')
            uploads += 1
            frames.push({ object_id: uploaded.object_id, media_type: uploaded.media_type, duration_ms: frame.durationMs, caption: frame.caption ?? '' })
          }
          content.props.frames = JSON.stringify(frames)
        }
        if (existing) {
          const current = await store.session.read<RecordContent>(entityId)
          const keys: { key: string; value: Json; expect: { rev: number } }[] = []
          if (JSON.stringify(current.content.schema) !== JSON.stringify(content.schema)) keys.push({ key: 'schema', value: content.schema, expect: { rev: current.content.key_revs.schema ?? 0 } })
          for (const [k, v] of Object.entries(content.props)) if (JSON.stringify(current.content.props[k]) !== JSON.stringify(v)) keys.push({ key: `p:${k}`, value: v, expect: { rev: current.content.key_revs[`p:${k}`] ?? 0 } })
          if (keys.length > 0) ops.push({ op: 'entity.set_keys', entity_id: entityId, keys })
          updated.push(entityId)
        } else {
          ops.push({ op: 'entity.create', entity_id: entityId, type_id: 'buckyos.record', parent_id: folderId, order_key: keyIn(folderId, folderLast), name, payload: { schema: content.schema, props: content.props } })
          created.push(entityId)
        }
      } else if (result.type === 'table') {
        const t = result.content as { fields: { field_id: string; name: string; type: string }[]; rows: Record<string, Json>[] }
        if (existing) {
          const meta = await store.session.read<TableSourceContent>(entityId)
          const known = new Set(meta.content.fields.map((f) => f.field_id))
          for (const field of t.fields) if (!known.has(field.field_id)) ops.push({ op: 'table.add_field', source_id: entityId, field: { field_id: field.field_id, name: field.name, type: field.type }, expect: { rev: meta.content.members_rev } })
          const page = await store.session.query({ source_id: entityId, limit: 1000, consistency: 'best_effort' })
          if (page.rows.length > 0) ops.push({ op: 'table.delete_records', source_id: entityId, records: page.rows.map((r) => ({ record_id: r.record_id, expect: { rev: r.rev } })) })
          ops.push({ op: 'table.insert_records', source_id: entityId, records: t.rows.map((values, i) => ({ record_id: `${slug}-${runSeq}-${i + 1}`, values })) })
          updated.push(entityId)
        } else {
          ops.push({ op: 'entity.create', entity_id: entityId, type_id: 'buckyos.table-source', parent_id: folderId, order_key: keyIn(folderId, folderLast), name, payload: { fields: t.fields as unknown as Json } })
          if (t.rows.length > 0) ops.push({ op: 'table.insert_records', source_id: entityId, records: t.rows.map((values, i) => ({ record_id: `${slug}-${i + 1}`, values })) })
          created.push(entityId)
        }
      } else if (result.type === 'image') {
        const img = result.content as { svg: string; width: number; height: number; alt?: string; caption?: string }
        const uploaded = await store.session.uploadAsset(new Blob([img.svg], { type: 'image/svg+xml' }), `${slug}.svg`)
        uploads += 1
        if (existing) {
          const current = await store.session.read<AssetContent>(entityId)
          if (current.content.payload.object_id !== uploaded.object_id) {
            ops.push({ op: 'entity.set_keys', entity_id: entityId, keys: [
              { key: 'object_id', value: uploaded.object_id, expect: { rev: current.content.key_revs.object_id ?? 0 } },
              { key: 'image', value: { width: img.width, height: img.height }, expect: { rev: current.content.key_revs.image ?? 0 } },
            ] })
          }
          updated.push(entityId)
        } else {
          ops.push({ op: 'entity.create', entity_id: entityId, type_id: 'buckyos.asset-ref', parent_id: folderId, order_key: keyIn(folderId, folderLast), name, payload: { object_id: uploaded.object_id, file_name: `${slug}.svg`, image: { width: img.width, height: img.height } } })
          created.push(entityId)
        }
      }
      ops.push({ op: 'entity.set_derived', entity_id: entityId, derived: derived() as unknown as Json })
      // ---- its Block
      const hasBlock = store.outline.all().some((e) => e.type_id === 'buckyos.cell' && e.source_id === entityId && !e.deleted)
      if (surfaceId && !hasBlock) {
        const cellPayload: Record<string, Json> = { view: { type: renderer }, source_ref: { entity_id: entityId }, title, ...(result.config ? { config: result.config } : {}) }
        if (renderer === 'asset') cellPayload.options = { fit: 'cover' }
        ops.push({ op: 'entity.create', entity_id: `${entityId}-blk`, type_id: 'buckyos.cell', parent_id: groupId, order_key: keyIn(groupId, blockLast), placement: { x: pos.x + 20, y: pos.y + 40, w: pos.w, h: pos.h }, payload: cellPayload })
      }
    }
    const missing = mode === 'overwrite' ? [...previous].filter((id) => !produced.includes(id) && store.outline.get(id) && !store.outline.get(id)?.deleted) : []
    const lastRun: WishLastRun & { seq: number } = { run_id: candidate.runId, state: 'succeeded', at: candidate.executedAt, read_set: candidate.readSet, produced, missing, group: groupId, simulated: candidate.simulated, seq: runSeq }
    ops.push({ op: 'entity.set_keys', entity_id: wishId, keys: [{ key: 'last_run', value: lastRun as unknown as Json, expect: { rev: wishRead.content.key_revs.last_run ?? 0 } }] })
    return { operations: ops, preconditions: candidate.preconditions, manual, produced, missing, created, updated, skipped, groupId, uploads }
  }

  /** Apply: one commit, guarded by the read set. Failure, conflict or cancel keeps the old results. */
  async apply(candidate: Candidate, choices: Record<string, ManualChoice> = {}): Promise<{ outcome: CommitOutcome | null; plan: ApplyPlan }> {
    const wishId = candidate.wishId
    this.busy.set(wishId, 'applying'); this.changed()
    try {
      const plan = await this.plan(candidate, choices)
      if (plan.manual.length > 0) return { outcome: null, plan }
      const wish = this.store.outline.get(wishId)
      const outcome = await this.store.submit({
        editId: `wish:${wishId}:apply`, label: `应用许愿格结果 ${wish?.title ?? wish?.name ?? wishId}${candidate.simulated ? '（模拟）' : ''}`,
        operations: plan.operations, preconditions: plan.preconditions,
      })
      if (outcome.status === 'accepted' || outcome.status === 'saved_locally') this.candidates.delete(wishId)
      this.changed()
      return { outcome, plan }
    } finally {
      this.busy.delete(wishId); this.changed()
    }
  }

  dispose() {
    for (const executor of this.executors.values()) executor.dispose()
    this.executors.clear()
    this.candidates.clear()
  }
}
