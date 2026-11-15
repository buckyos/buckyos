/* WishService (许愿格详细设计 §3, §13; 许愿格交互升级 §3.3): the window's side of wish runs. The service does the
 * work — snapshot, context map, model and program, validation — and writes each stage's product itself when the stage
 * ends: one commit with the product and the stage's record in the wish's history, nothing waits for a confirmation.
 * This class starts stages, follows their progress, puts the commit of a stage this window started into this window's
 * undo stack, cancels, and rolls a stage's current record back.
 *
 * Nothing here runs because of change events: every run is started by the user. */

import { describeError } from '../../api/session'
import type { FreshnessInfo, Json, WishExecutionMode, WishMainStatus, WishPayloadRead, WishRecord, WishRollbackResult, WishRunView, WishStage } from '../../api/types'
import { loadCore } from '../../api/wasm'
import { resolveChatClient } from '../../executor/chat'
import { createInfer } from '../../executor/infer'
import { claims, LocalExecutor } from '../../executor/localExecutor'
import { aiwsT } from '../../i18n'
import type { WorkspaceStore } from '../../state/store'
import { latestRecord } from './records'

const ACTIVE = new Set(['queued', 'snapshotting', 'running', 'validating', 'applying'])
/** The execution mode this browser prefers (内核架构改进 D14); a per-viewer convenience. */
const EXECUTION_KEY = 'aiworkspace.execution'

export function isActive(run: WishRunView | undefined | null): boolean { return Boolean(run && ACTIVE.has(run.state)) }
const CODING_CODES = new Set(['PROGRAM_FAILED', 'MACRO_CODE_CHANGED', 'NEEDS_CODING'])
/** Run codes that send a wish back to coding (the core's `CODING_CODES`): the macro failed, its code changed, or it no
 * longer fits its inputs. */
export function isCodingFailure(error: { code?: string; sub_code?: string } | null | undefined): boolean {
  return Boolean(error && CODING_CODES.has(error.sub_code ?? error.code ?? ''))
}
/** The failed run the next coding repairs (第一期 §2.1): its macro failed, its HTML code changed, or it no longer fits its inputs. */
export function repairOf(run: WishRunView | undefined | null): WishRunView | null {
  return run && run.state === 'failed' && run.stage !== 'analyze' && isCodingFailure(run.error) ? run : null
}

/** Where the run was started from (§5.2): a hint for the context map, never a dependency. */
export interface WishLocation { cell_id?: string; surface_id?: string; selection?: string[]; viewport?: { x: number; y: number; w: number; h: number } }

export type WishBusy = 'analyzing' | 'coding' | 'running' | 'cancelling' | 'rolling_back'

const UNDO_LABEL: Record<WishStage, string> = { analyze: 'aiworkspace.wishcard.undo.analyze', code: 'aiworkspace.wishcard.undo.code', run: 'aiworkspace.wishcard.undo.run' }

export class WishService {
  private readonly store: WorkspaceStore
  /** The run each wish shows (the latest one this window knows). */
  private readonly runs = new Map<string, WishRunView>()
  private readonly polling = new Set<string>()
  private readonly restored = new Set<string>()
  /** Runs this window started or resumed: their commit joins this window's undo stack when they end (§3.3). */
  private readonly own = new Set<string>()
  private readonly settled = new Set<string>()
  /** Local runs this window executes, by run id (T3-4). */
  private readonly locals = new Map<string, LocalExecutor>()
  private readonly listeners = new Set<() => void>()
  private disposed = false
  private version = 0
  readonly subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener) } }
  snapshot = () => this.version
  readonly busy = new Map<string, WishBusy>()
  /** Why the last thing asked of a wish could not start (a run never taken, a rollback refused), shown by its card. */
  readonly errors = new Map<string, string>()

  constructor(store: WorkspaceStore) {
    this.store = store
    // the page goes away: local executors stop without reporting a failure — the lease lapses and the run is
    // interrupted, which this browser can resume (D15)
    window.addEventListener('pagehide', this.onPageHide)
  }

  private readonly onPageHide = () => { for (const local of this.locals.values()) local.cancel() }

  private changed() { this.version += 1; for (const listener of [...this.listeners]) listener() }

  run(wishId: string): WishRunView | undefined { return this.runs.get(wishId) }

  /** The runs under way this window follows, with their latest progress (the status bar's "运行中", §3.8). */
  activeRuns(): WishRunView[] { return [...this.runs.values()].filter(isActive) }

  private setRun(run: WishRunView) {
    this.runs.set(run.wish_id, run)
    this.changed()
    if (!isActive(run)) void this.settle(run)
  }

  /** A stage this window started has ended and written: its commit joins this window's undo stack, as an edit would. */
  private async settle(run: WishRunView) {
    const commit = run.applied?.commit
    if (!this.own.has(run.run_id) || this.settled.has(run.run_id) || run.state !== 'succeeded' || commit?.status !== 'accepted') return
    this.settled.add(run.run_id)
    const wish = this.store.outline.get(run.wish_id)
    this.store.undo.pushCommit(commit.commit_id, aiwsT(UNDO_LABEL[run.stage], { name: wish?.title ?? wish?.name ?? run.wish_id }))
    await this.store.session.whenApplied(commit.seq)
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
      this.setRun(isActive(latest) ? await this.store.session.wishGet(latest.run_id) : latest)
      if (isActive(latest)) void this.poll(latest.run_id, wishId)
    } catch { /* offline or no access: nothing to restore */ }
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
        if (this.disposed) throw new Error(aiwsT('aiworkspace.wish.error.windowClosed'))
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

  // ---- where a run executes (内核架构改进 §9, D14, D22)

  /** The modes this workspace offers: this browser, and the owner's Agent when hosted execution is configured.
   * Empty on a service without the execution model. */
  executionModes(): WishExecutionMode[] {
    const info = this.store.session.info()
    if (!info.wish_execution) return []
    return info.hosted_exec?.enabled ? ['local', 'hosted'] : ['local']
  }

  executionMode(): WishExecutionMode {
    const modes = this.executionModes()
    let saved: string | null = null
    try { saved = window.localStorage.getItem(EXECUTION_KEY) } catch { saved = null }
    if (saved && modes.includes(saved as WishExecutionMode)) return saved as WishExecutionMode
    const preset = this.store.session.info().wish_execution?.default
    if (preset && modes.includes(preset)) return preset
    return 'local'
  }

  setExecutionMode(mode: WishExecutionMode) {
    try { window.localStorage.setItem(EXECUTION_KEY, mode) } catch { /* not remembered */ }
    this.changed()
  }

  /** An interrupted local run resumes from this browser, which keeps its claim token. */
  canResume(run: WishRunView | undefined | null): boolean {
    return Boolean(run && run.state === 'interrupted' && run.execution?.mode === 'local' && claims.get(run.run_id))
  }

  private runLocally(runId: string, claimToken: string, wishId: string) {
    claims.set(runId, claimToken)
    const executor = new LocalExecutor(this.store.session, resolveChatClient(), createInfer(), runId, claimToken)
    this.locals.set(runId, executor)
    executor.run()
      .then((outcome) => { if (outcome === 'finished') claims.forget(runId) })
      .catch((error: unknown) => { this.errors.set(wishId, describeError(error)); this.changed() })
      .finally(() => { this.locals.delete(runId) })
  }

  async resume(run: WishRunView): Promise<WishRunView> {
    return this.guard(run.wish_id, run.stage === 'analyze' ? 'analyzing' : run.stage === 'code' ? 'coding' : 'running', async () => {
      const token = claims.get(run.run_id)
      if (!token) throw new Error(aiwsT('aiworkspace.wish.resumeHint'))
      this.own.add(run.run_id)
      this.runLocally(run.run_id, token, run.wish_id)
      return this.poll(run.run_id, run.wish_id)
    })
  }

  /** Start a stage and follow it to its end. A run that fails is no error here: it wrote its record (§3.3), which says why. */
  private async start(wishId: string, stage: WishStage, extra: Record<string, Json>, cellId?: string, raised?: { event: Json; key: string }): Promise<WishRunView> {
    const params: Record<string, Json> = { wish_id: wishId, stage, location: this.location(wishId, cellId) as unknown as Json, request: { ...this.request(), ...(raised ? { event: raised.event } : {}) }, ...extra }
    if (this.executionModes().length) params.execution = { mode: this.executionMode() }
    // the stage's commit is written in this window's session: the write locks it holds count
    params.session_id = this.store.session.sessionId
    const started = await this.store.session.wishStart('wish.xllm@1', params, raised?.key)
    this.own.add(started.run_id)
    this.setRun(started)
    if (started.claim_token) this.runLocally(started.run_id, started.claim_token, wishId)
    return this.poll(started.run_id, wishId)
  }

  private async payload(wishId: string): Promise<WishPayloadRead> {
    return (await this.store.session.read<WishPayloadRead>(wishId)).content
  }

  // ---- the three stages (AI Native 第一期 §2.1)

  /** Analyze the request: the task contract and the inputs it found are written when the stage ends. */
  async analyze(wishId: string, cellId?: string): Promise<WishRunView> {
    return this.guard(wishId, 'analyzing', () => this.start(wishId, 'analyze', {}, cellId))
  }

  /** Write (or repair) the wish's macro and trial-run it; only the macro is written. `parentRunId`: a failed run to repair. */
  async code(wishId: string, options: { cellId?: string; parentRunId?: string } = {}): Promise<WishRunView> {
    return this.guard(wishId, 'coding', () => this.start(wishId, 'code', options.parentRunId ? { parent_run_id: options.parentRunId } : {}, options.cellId))
  }

  /** Run the stored macro (no model loop); its results are written when it ends. `event` / `idempotencyKey`: the run an
   * event handler starts (扩展控件 P2): `request.event` names it, and it is keyed like the event's writes. */
  async runMacro(wishId: string, options: { cellId?: string; event?: Json; idempotencyKey?: string } = {}): Promise<WishRunView> {
    return this.guard(wishId, 'running', () => this.start(wishId, 'run', {}, options.cellId, options.event && options.idempotencyKey ? { event: options.event, key: options.idempotencyKey } : undefined))
  }

  /** What the page knows of a wish's runs, in the shape `wish_main_status` reads: the run under way, and the last
   * failure — the newest record when it is a failure, or a failed run its record has not arrived from yet. */
  runSummary(wishId: string, payload?: WishPayloadRead['payload']): Json {
    const run = this.runs.get(wishId)
    if (run && isActive(run)) return { active: { stage: run.stage, state: run.state } }
    const latest = latestRecord(payload?.history)
    if (run?.state === 'failed' && (!latest || latest.at < run.created_at)) return { last_failed: { stage: run.stage, error: (run.error ?? null) as unknown as Json } }
    if (latest && !latest.ok && latest.action !== 'rollback') return { last_failed: { stage: latest.stage, error: (latest.error ?? null) as unknown as Json } }
    return {}
  }

  /** The main state and the next action of a wish (PRD §5.8): computed by the core from its freshness and its runs. */
  async mainStatus(wishId: string, freshness: FreshnessInfo | undefined | null, payload?: WishPayloadRead['payload']): Promise<WishMainStatus | null> {
    if (!freshness) return null
    const core = await loadCore()
    return JSON.parse(core.wish_main_status(JSON.stringify(freshness), JSON.stringify(this.runSummary(wishId, payload)))) as WishMainStatus
  }

  /** The wish's next action — analyze, code (repairing a macro that failed) or run — as an event handler asks for it. */
  async next(wishId: string, options: { cellId?: string; event?: Json; idempotencyKey?: string } = {}): Promise<WishRunView | null> {
    const read = await this.payload(wishId)
    const fresh = (await this.store.session.freshness([wishId]))[0]
    const status = await this.mainStatus(wishId, fresh, read.payload)
    switch (status?.next_action) {
      case 'analyze': return this.analyze(wishId, options.cellId)
      case 'code': return this.code(wishId, { cellId: options.cellId, parentRunId: repairOf(this.runs.get(wishId))?.run_id })
      case 'cancel':
      case 'none': return this.runs.get(wishId) ?? null
      default: return this.runMacro(wishId, options)
    }
  }

  async cancel(run: WishRunView): Promise<void> {
    await this.guard(run.wish_id, 'cancelling', async () => {
      this.locals.get(run.run_id)?.cancel()
      await this.store.session.procCancel(run.run_id)
      this.setRun(await this.store.session.wishGet(run.run_id))
    })
  }

  // ---- rollback (§3.4, D3)

  /** Roll `record` (its stage's current one) back to the version before it. `needs_confirm` names what later changes it
   * would lose and writes nothing: asked again with `confirm`, it goes ahead. An accepted rollback is one commit of this
   * window's undo stack. */
  async rollback(wishId: string, record: WishRecord, confirm = false): Promise<WishRollbackResult> {
    return this.guard(wishId, 'rolling_back', async () => {
      const result = await this.store.session.procRollback(record.id, confirm)
      if (result.status === 'accepted' && result.commit.status === 'accepted') {
        const wish = this.store.outline.get(wishId)
        this.store.undo.pushCommit(result.commit.commit_id, aiwsT('aiworkspace.wishcard.undo.rollback', { name: wish?.title ?? wish?.name ?? wishId }))
        await this.store.session.whenApplied(result.commit.seq)
      }
      return result
    })
  }

  // ---- the program (§9.1)

  async programSource(wishId: string): Promise<string | null> {
    const read = await this.payload(wishId)
    const source = read.payload.program?.source
    if (!source) return null
    return (await this.store.session.fetchAsset(source)).text()
  }

  /** Save a hand-edited macro: a new source asset with the manifest it had (a trial run in coding resolves a new one);
   * results then show as generated under another configuration. */
  async saveProgram(wishId: string, source: string): Promise<void> {
    const read = await this.payload(wishId)
    const bytes = new TextEncoder().encode(source)
    const manifest = read.payload.program?.manifest ?? { inputs: {}, results: {} }
    const digest = Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(source + '\n' + JSON.stringify(manifest))))).map((b) => b.toString(16).padStart(2, '0')).join('')
    const uploaded = await this.store.session.uploadAsset(new Blob([bytes], { type: 'text/plain' }), 'main.js')
    const program = { language: 'js', api_version: 3, source: uploaded.object_id, digest, manifest, produces: read.payload.program?.produces ?? [], edited_by: this.store.session.principal ?? 'user' }
    const outcome = await this.store.submit({
      editId: `wish:${wishId}:program`, label: aiwsT('aiworkspace.wish.edit.program'),
      operations: [{ op: 'entity.set_keys', entity_id: wishId, keys: [{ key: 'program', value: program as unknown as Json, expect: { rev: read.key_revs.program ?? 0 } }] }],
    })
    if (outcome.status === 'conflict' || outcome.status === 'rejected') throw new Error(aiwsT('aiworkspace.wish.error.programNotSaved', { code: outcome.code }))
  }

  dispose() {
    this.disposed = true
    window.removeEventListener('pagehide', this.onPageHide)
    for (const local of this.locals.values()) local.cancel()
    this.locals.clear()
    this.runs.clear()
  }
}
