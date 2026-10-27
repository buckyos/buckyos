/* What one open workspace knows of its ink (手写笔 TODO D3): the decoded segments of the Blocks drawn so far (hit
 * tests and the eraser read them), and the strokes written here that wait for their Blocks — saved or still being
 * sent — which the live layer keeps drawing until their Blocks have drawn them (no gap, no double line) or they are
 * refused. A stroke is submitted at once: the next one never waits for the network. */

import { randomId } from '../api/ids'
import type { CommitOutcome, Operation } from '../api/types'
import { aiwsT } from '../i18n'
import type { WorkspaceStore } from '../state/store'
import { groupAt, relativeTo, type Laid } from '../ui/canvas/layout'
import type { Rect } from '../ui/canvas/render/camera'
import { encodeStroke, INK_VIEW, settleChannels, simplify, SIMPLIFY_PX, widePoints, worldPoints, type InkPoint, type InkSegment, type InkStyle, type WidePoint } from './model'

/** A saved stroke whose Blocks are not in the outline this long after is no longer drawn. */
const GONE_MS = 5000

/** A stroke written here that its Blocks do not draw yet. `key`: the idempotency key of a submission whose result is
 * unknown — resent as it is (the same key, the same operations), so a retry never writes the stroke twice. */
export interface WaitingInk { stroke: string; editId: string; surfaceId: string; cells: string[]; points: WidePoint[]; color: string; outcome: 'sending' | 'saved' | 'unknown'; operations: Operation[]; key?: string }

export class InkBook {
  private readonly segments = new Map<string, { rev: number; ink: InkSegment }>()
  private readonly waiting = new Map<string, WaitingInk>()
  private readonly drawn = new Set<string>()
  private readonly listeners = new Set<() => void>()
  private readonly waitListeners = new Set<() => void>()
  private version = 0
  private emitTimer = 0
  private readonly store: WorkspaceStore
  /** The last order key given per parent: strokes written before the previous ones are back still stack in order. */
  private readonly lastKey = new Map<string, string>()

  constructor(store: WorkspaceStore) {
    this.store = store
    // an unsaved stroke given up in the save-state list is no longer drawn
    store.edits.subscribe(() => {
      for (const w of [...this.waiting.values()]) {
        const entry = store.edits.get(w.editId)
        if (w.outcome === 'unknown' && (!entry || entry.state === 'needs_attention')) this.drop(w.stroke)
      }
    })
    // the connection is back: strokes whose result is unknown are resent
    store.session.subscribeStatus(() => { if (store.session.status().kind === 'live') this.retryUnknown() })
  }

  /** Resend every stroke whose result is unknown (same key, same content). */
  retryUnknown() {
    for (const w of this.waiting.values()) if (w.outcome === 'unknown' && w.key) this.settle(w, this.store.retry({ editId: w.editId, label: aiwsT('aiworkspace.canvas.ink.strokeLabel'), operations: w.operations }, w.key))
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

  /** The order key of a new child of `parentId`, after its last child and after any stroke given one before. */
  nextKey(parentId: string): string {
    const known = this.store.outline.childrenOf(parentId).at(-1)?.order_key ?? undefined
    const mine = this.lastKey.get(parentId)
    const after = mine && (!known || mine > known) ? mine : known
    const key = this.store.core.order_key_between(after, undefined)
    this.lastKey.set(parentId, key)
    return key
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
      return { op: 'entity.create', entity_id: id, type_id: 'buckyos.cell', parent_id: parentId, order_key: this.nextKey(parentId), placement: relativeTo(laid, parentId, s.rect), payload: { view: { type: INK_VIEW, version: 1 }, config: s.config } }
    })
    const editId = `ink:${stroke}`
    const waiting: WaitingInk = { stroke, editId, surfaceId, cells, points: widePoints(kept, used), color: used.color, outcome: 'sending', operations }
    this.waiting.set(stroke, waiting)
    this.changed()
    this.settle(waiting, this.store.submit({ editId, label: aiwsT('aiworkspace.canvas.ink.strokeLabel'), operations }))
    return stroke
  }

  private settle(waiting: WaitingInk, submitted: Promise<CommitOutcome>) {
    waiting.outcome = 'sending'
    void submitted.then((outcome) => {
      const { stroke, cells } = waiting
      if (outcome.status === 'accepted' || outcome.status === 'saved_locally') {
        waiting.outcome = 'saved'
        // saved but gone at once (deleted elsewhere, undone): nothing will draw it
        window.setTimeout(() => { if (cells.some((c) => !this.store.outline.get(c) || this.store.outline.get(c)?.deleted)) this.drop(stroke) }, GONE_MS)
        return
      }
      if (outcome.status === 'unknown' || outcome.status === 'storage_failed') {
        // still drawn, still in the save-state list; resent with its key when the connection is back, or from here
        const first = waiting.key === undefined
        waiting.outcome = 'unknown'
        if (outcome.idempotency_key) waiting.key = outcome.idempotency_key
        if (first && waiting.key) this.store.notify('error', aiwsT('aiworkspace.canvas.ink.strokeUnknown'), { label: aiwsT('aiworkspace.common.retry'), run: () => this.retryUnknown() })
        return
      }
      this.drop(stroke)
      this.store.notify('error', aiwsT('aiworkspace.canvas.ink.strokeRefused', { code: outcome.code }))
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
