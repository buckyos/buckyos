/* What one open workspace knows of its ink (手写笔 TODO D3): the decoded segments of the Blocks drawn so far (hit
 * tests and the eraser read them), and the strokes written here that wait for their Blocks — saved or still being
 * sent — which the live layer keeps drawing until their Blocks have drawn them (no gap, no double line) or the user
 * discards the unsaved input. Strokes normally submit at once; exhausted sibling order queues them until a guarded,
 * non-undoable reorder is accepted. Drawing continues while those submissions wait. */

import { randomId } from '../api/ids'
import type { CommitOutcome, Json, Operation } from '../api/types'
import type { SubmissionEvent } from '../api/session'
import { aiwsT } from '../i18n'
import type { WorkspaceStore } from '../state/store'
import { groupAt, relativeTo, type Laid } from '../ui/canvas/layout'
import type { Rect } from '../ui/canvas/render/camera'
import { encodeStroke, INK_VIEW, settleChannels, simplify, SIMPLIFY_PX, widePoints, worldPoints, type InkPoint, type InkSegment, type InkStyle, type WidePoint } from './model'

/** A saved stroke whose Blocks are not in the outline this long after is no longer drawn. */
const GONE_MS = 5000

/** A stroke written here that its Blocks do not draw yet. `key`: the idempotency key of a submission whose result is
 * unknown — resent as it is (the same key, the same operations), so a retry never writes the stroke twice. */
export interface WaitingInk { stroke: string; editId: string; surfaceId: string; cells: string[]; points: WidePoint[]; color: string; outcome: 'queued' | 'sending' | 'saved' | 'unknown' | 'failed'; operations: Operation[]; key?: string }

interface InkSubmission { waiting: WaitingInk; parentId: string; creates: Operation[]; state: 'queued' | 'sending' | 'unknown' | 'local' | 'failed'; compacting: boolean; attempts: number; idempotencyKey: string; notifiedUnknown?: boolean; terminal?: SubmissionEvent; preconditions?: unknown[] }

export class InkBook {
  private readonly segments = new Map<string, { rev: number; ink: InkSegment }>()
  private readonly waiting = new Map<string, WaitingInk>()
  private readonly drawn = new Set<string>()
  private readonly listeners = new Set<() => void>()
  private readonly waitListeners = new Set<() => void>()
  private version = 0
  private emitTimer = 0
  private readonly store: WorkspaceStore
  private readonly submissions = new Map<string, InkSubmission>()
  private readonly barriers = new Map<string, InkSubmission>()

  constructor(store: WorkspaceStore) {
    this.store = store
    store.edits.subscribe(() => {
      for (const submission of [...this.submissions.values()]) {
        const { waiting, state, parentId } = submission
        const entry = store.edits.get(waiting.editId)
        if (!entry && (state === 'queued' || state === 'failed')) {
          this.submissions.delete(waiting.stroke)
          this.store.noteUnsaved(waiting.editId, null)
          this.release(submission)
          this.drop(waiting.stroke)
          queueMicrotask(() => this.flush(parentId))
        }
      }
    })
    store.session.subscribeStatus(() => { if (store.session.status().kind === 'live') this.retryUnknown() })
    store.session.offline?.subscribeSubmissions((event) => {
      const submission = [...this.submissions.values()].find((s) => s.idempotencyKey === event.key)
      if (!submission || !['accepted', 'discarded', 'conflict', 'rejected'].includes(event.state)) return
      if (submission.state !== 'local') submission.terminal = event
      else this.localTerminal(submission, event)
    })
  }

  private localTerminal(submission: InkSubmission, event: SubmissionEvent) {
    if (event.state === 'accepted') {
      submission.state = 'sending'
      void this.complete(submission)
    } else if (event.state === 'discarded') {
      this.submissions.delete(submission.waiting.stroke)
      this.release(submission)
      this.drop(submission.waiting.stroke)
      this.store.noteUnsaved(submission.waiting.editId, null)
      this.flush(submission.parentId)
    } else if (event.state === 'conflict' || event.state === 'rejected') {
      this.settle(submission, Promise.resolve(event.result as unknown as CommitOutcome))
    }
  }

  retryUnknown() {
    for (const submission of this.submissions.values()) {
      if (submission.state === 'unknown') this.settle(submission, this.store.submit(this.options(submission)))
    }
  }

  /** Decoded segments: changes are announced once per tick (many Blocks mount together). */
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener) } }
  snapshot = () => this.version

  get(cellId: string): InkSegment | undefined { return this.segments.get(cellId)?.ink }

  put(cellId: string, rev: number, ink: InkSegment) {
    const known = this.segments.get(cellId)
    if (known && known.rev === rev && known.ink === ink) return
    this.segments.set(cellId, { rev, ink })
    if (!this.emitTimer) this.emitTimer = window.setTimeout(() => { this.emitTimer = 0; this.version += 1; for (const l of [...this.listeners]) l() }, 0)
  }

  private readonly worldCache = new Map<string, { key: string; pts: WidePoint[] }>()

  /** A segment's drawn points in world coordinates (kept per box and rotation), or null before its Block drew it. */
  world(cellId: string, rect: Rect, rotation: number): WidePoint[] | null {
    const known = this.segments.get(cellId)
    if (!known) return null
    const key = `${known.rev}:${rect.x},${rect.y},${rect.w},${rect.h},${rotation}`
    const cached = this.worldCache.get(cellId)
    if (cached?.key === key) return cached.pts
    const pts = worldPoints(known.ink, rect, rotation)
    this.worldCache.set(cellId, { key, pts })
    return pts
  }

  // ---- strokes waiting for their Blocks

  subscribeWaiting = (listener: () => void) => { this.waitListeners.add(listener); return () => { this.waitListeners.delete(listener) } }

  waitingOn(surfaceId: string): WaitingInk[] {
    return [...this.waiting.values()].filter((w) => w.surfaceId === surfaceId)
  }

  private changed() { for (const l of [...this.waitListeners]) l() }

  /** A Block drew its segment: a waiting stroke whose segments are all drawn stops being drawn by the live layer. */
  rendered(cellId: string) {
    for (const w of this.waiting.values()) {
      if (!w.cells.includes(cellId)) continue
      this.drawn.add(cellId)
      if (w.cells.every((c) => this.drawn.has(c))) this.drop(w.stroke)
      return
    }
  }

  drop(stroke: string) {
    const w = this.waiting.get(stroke)
    if (!w) return
    this.waiting.delete(stroke)
    for (const c of w.cells) this.drawn.delete(c)
    this.changed()
  }

  private release(submission: InkSubmission) {
    if (this.barriers.get(submission.parentId) === submission) this.barriers.delete(submission.parentId)
  }

  private allocate(creates: Operation[], after?: string): Operation[] {
    return creates.map((op) => {
      after = this.store.core.order_key_between(after, undefined)
      return { ...op, order_key: after }
    })
  }

  private options(submission: InkSubmission) {
    const { waiting, preconditions } = submission
    return { editId: waiting.editId, label: aiwsT('aiworkspace.canvas.ink.strokeLabel'), operations: waiting.operations, preconditions, idempotencyKey: submission.idempotencyKey, undoable: !submission.compacting }
  }

  private flush(parentId: string) {
    if (this.barriers.has(parentId)) return
    for (const submission of this.submissions.values()) {
      if (submission.parentId !== parentId || submission.state !== 'queued') continue
      const active = [...this.submissions.values()].filter((s) => s.parentId === parentId && (s.state === 'sending' || s.state === 'unknown' || (s.state === 'local' && s.waiting.cells.some((id) => !this.store.outline.get(id)))))
      let after = this.store.outline.childrenOf(parentId).at(-1)?.order_key ?? undefined
      for (const pending of active) {
        for (const op of pending.waiting.operations) {
          if (op.op === 'entity.create' && typeof op.order_key === 'string' && (!after || op.order_key > after)) after = op.order_key
        }
      }
      try {
        submission.waiting.operations = this.allocate(submission.creates, after)
      } catch (error) {
        if (String(error).includes('ORDER_KEY_EXHAUSTED')) {
          if (active.length) return
          submission.compacting = true
          this.barriers.set(parentId, submission)
          void this.rebalance(submission).catch((e) => this.fail(submission, String(e)))
          return
        }
        this.fail(submission, String(error))
        continue
      }
      this.settle(submission, this.store.submit(this.options(submission)))
    }
  }

  private async rebalance(submission: InkSubmission) {
    const siblings = this.store.outline.childrenOf(submission.parentId)
    let after: string | undefined
    const places: Operation[] = siblings.map((sibling) => {
      after = this.store.core.order_key_between(after, undefined)
      return { op: 'tree.place', entity_id: sibling.entity_id, order_key: after, expect: { rev: sibling.struct_rev ?? 0 } }
    })
    const operations = places
    const ids = siblings.map((sibling) => sibling.entity_id).sort().join('\n')
    const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(ids))
    if (!this.submissions.has(submission.waiting.stroke)) return
    const hash = [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, '0')).join('').slice(0, 32)
    submission.preconditions = [{ target: { entity_id: submission.parentId, selector: { kind: 'tree_children' } }, expect: { hash } }]
    submission.waiting.operations = operations
    this.settle(submission, this.store.submit(this.options(submission)))
  }

  /** Save a stroke written on `surfaceId` (world points as sampled at `zoom`): simplified, cut into segments within the
   * config budget, created in one commit under the group it was written in. */
  write(points: InkPoint[], style: InkStyle, zoom: number, surfaceId: string, laid: Map<string, Laid>): string | null {
    if (points.length === 0) return null
    const settled = settleChannels(points)
    const used: InkStyle = { ...style, pressure: style.pressure && settled.some((q) => q.p !== undefined) }
    const kept = simplify(settled, SIMPLIFY_PX / zoom, used)
    const stroke = randomId('k')
    const segments = encodeStroke(kept, used, stroke)
    let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity
    for (const s of segments) { minX = Math.min(minX, s.rect.x); minY = Math.min(minY, s.rect.y); maxX = Math.max(maxX, s.rect.x + s.rect.w); maxY = Math.max(maxY, s.rect.y + s.rect.h) }
    const parentId = groupAt(laid, { x: (minX + maxX) / 2, y: (minY + maxY) / 2 }, surfaceId)
    const cells: string[] = []
    const operations: Operation[] = segments.map((s) => {
      const id = randomId('c')
      cells.push(id)
      return { op: 'entity.create', entity_id: id, type_id: 'buckyos.cell', parent_id: parentId, placement: relativeTo(laid, parentId, s.rect), payload: { view: { type: INK_VIEW, version: 1 }, config: s.config } }
    })
    const editId = `ink:${stroke}`
    const waiting: WaitingInk = { stroke, editId, surfaceId, cells, points: widePoints(kept, used), color: used.color, outcome: 'queued', operations: [] }
    this.store.edits.set({ id: editId, label: aiwsT('aiworkspace.canvas.ink.strokeLabel'), state: 'unsaved' })
    this.waiting.set(stroke, waiting)
    this.submissions.set(stroke, { waiting, parentId, creates: operations, state: 'queued', compacting: false, attempts: 0, idempotencyKey: randomId('ink') })
    this.remember(this.submissions.get(stroke)!)
    this.changed()
    this.flush(parentId)
    return stroke
  }

  private remember(submission: InkSubmission, reason = aiwsT('aiworkspace.canvas.ink.strokeUnknown')) {
    this.store.noteUnsaved(submission.waiting.editId, { label: aiwsT('aiworkspace.canvas.ink.strokeLabel'), operations: submission.creates, mine: submission.creates as unknown as Json, reason, at: new Date().toISOString() })
  }

  private async complete(submission: InkSubmission, key = submission.idempotencyKey) {
    if (submission.idempotencyKey !== key || !this.submissions.has(submission.waiting.stroke)) return
    if (submission.compacting) {
      this.remember(submission)
      this.store.edits.set({ id: submission.waiting.editId, label: aiwsT('aiworkspace.canvas.ink.strokeLabel'), state: 'unsaved' })
    }
    try {
      if (submission.compacting || submission.waiting.cells.some((id) => !this.store.outline.get(id))) await this.store.outline.reload()
    } catch (error) {
      this.store.notify('error', String(error), { label: aiwsT('aiworkspace.common.retry'), run: () => { void this.complete(submission, key) } })
      return
    }
    if (submission.idempotencyKey !== key || !this.submissions.has(submission.waiting.stroke)) return
    if (submission.compacting) {
      submission.compacting = false
      submission.preconditions = undefined
      submission.idempotencyKey = randomId('ink')
      submission.terminal = undefined
      submission.state = 'queued'
      submission.waiting.outcome = 'queued'
      submission.waiting.operations = []
      this.remember(submission)
    } else {
      submission.waiting.outcome = 'saved'
      this.submissions.delete(submission.waiting.stroke)
      this.store.noteUnsaved(submission.waiting.editId, null)
      const { cells, stroke } = submission.waiting
      window.setTimeout(() => { if (cells.some((c) => !this.store.outline.get(c) || this.store.outline.get(c)?.deleted)) this.drop(stroke) }, GONE_MS)
    }
    this.release(submission)
    this.flush(submission.parentId)
  }

  private fail(submission: InkSubmission, detail: string, code = 'INVALID_OPERATION') {
    const { waiting, parentId } = submission
    if (!this.submissions.has(waiting.stroke)) return
    submission.state = 'failed'
    waiting.outcome = 'failed'
    this.waiting.set(waiting.stroke, waiting)
    this.release(submission)
    this.remember(submission, detail)
    this.store.edits.set({ id: waiting.editId, label: aiwsT('aiworkspace.canvas.ink.strokeLabel'), state: 'needs_attention', code, detail, pendingKey: this.store.edits.get(waiting.editId)?.pendingKey })
    this.store.notify('error', aiwsT('aiworkspace.canvas.ink.strokeRefused', { code }), {
      label: aiwsT('aiworkspace.common.retry'),
      run: () => {
        if (!this.submissions.has(waiting.stroke) || submission.state !== 'failed') return
        submission.state = 'queued'
        submission.compacting = false
        submission.attempts = 0
        submission.preconditions = undefined
        submission.idempotencyKey = randomId('ink')
        submission.terminal = undefined
        waiting.key = undefined
        waiting.outcome = 'queued'
        this.flush(parentId)
      },
    })
    this.changed()
    queueMicrotask(() => this.flush(parentId))
  }

  private settle(submission: InkSubmission, submitted: Promise<CommitOutcome>) {
    const { waiting, parentId } = submission
    const key = submission.idempotencyKey
    submission.state = 'sending'
    waiting.outcome = 'sending'
    void submitted.catch((error): CommitOutcome => ({ status: 'unknown', idempotency_key: key, detail: String(error) })).then(async (outcome) => {
      if (submission.idempotencyKey !== key || !this.submissions.has(waiting.stroke)) return
      if (outcome.status === 'accepted') {
        await this.complete(submission)
        return
      }
      if (outcome.status === 'saved_locally') {
        submission.state = 'local'
        waiting.key = outcome.idempotency_key
        if (submission.terminal) {
          const event = submission.terminal
          submission.terminal = undefined
          this.localTerminal(submission, event)
          return
        }
        if (submission.compacting) {
          waiting.outcome = 'queued'
          this.remember(submission)
        } else {
          waiting.outcome = 'saved'
          this.store.noteUnsaved(waiting.editId, null)
          this.flush(parentId)
        }
        return
      }
      if (outcome.status === 'unknown' || outcome.status === 'storage_failed') {
        const first = !submission.notifiedUnknown
        submission.notifiedUnknown = true
        submission.state = 'unknown'
        waiting.outcome = 'unknown'
        waiting.key = submission.idempotencyKey
        this.remember(submission, outcome.detail)
        if (first) this.store.notify('error', aiwsT('aiworkspace.canvas.ink.strokeUnknown'), { label: aiwsT('aiworkspace.common.retry'), run: () => this.retryUnknown() })
        this.changed()
        return
      }
      if (submission.compacting && outcome.status === 'conflict' && submission.attempts++ < 2) {
        submission.state = 'queued'
        waiting.outcome = 'queued'
        try {
          await this.store.outline.reload()
          if (submission.idempotencyKey !== key || !this.submissions.has(waiting.stroke)) return
          submission.idempotencyKey = randomId('ink')
          submission.terminal = undefined
          await this.rebalance(submission)
        } catch (error) { this.fail(submission, String(error)) }
        return
      }
      this.fail(submission, String((outcome.status === 'rejected' ? outcome.errors?.[0]?.detail : outcome.conflicts?.[0]?.detail) ?? outcome.detail ?? outcome.code), outcome.code)
    })
  }

  /** Delete whole logical strokes (every segment of each) in one commit: one undo step. */
  async erase(cellIds: string[], strokes: number): Promise<boolean> {
    const operations: Operation[] = []
    for (const id of cellIds) {
      const entity = this.store.outline.get(id)
      if (entity && !entity.deleted) operations.push({ op: 'entity.delete', entity_id: id, expect: { rev: entity.life_rev } })
    }
    if (operations.length === 0) return false
    const outcome = await this.store.submit({ editId: `erase:${randomId().slice(0, 8)}`, label: aiwsT('aiworkspace.canvas.ink.eraseLabel', { count: strokes }), operations })
    if (outcome.status === 'accepted' || outcome.status === 'saved_locally') return true
    if (outcome.status === 'rejected' || outcome.status === 'conflict') this.store.notify('error', aiwsT('aiworkspace.canvas.ink.eraseRefused', { code: outcome.code }))
    return false
  }
}

const books = new WeakMap<WorkspaceStore, InkBook>()

export function inkBook(store: WorkspaceStore): InkBook {
  let book = books.get(store)
  if (!book) { book = new InkBook(store); books.set(store, book) }
  return book
}
