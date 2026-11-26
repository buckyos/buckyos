/* What the Shell's transfer UI is doing (剪贴板规范 §5, §7): a drag from the Shell's handle, choosing a destination
 * by tapping (or from a list), "Paste desktop object…", and the result notice. The handle captures the pointer, so
 * the Shell sees every move and the release itself; windows are hit by the browser's own hit test of the topmost
 * visible element, so a covered window never receives anything. */
import { desktopUIStore } from '../models/DesktopUIDataModel'
import { desktopTransfers, type TransferOffer } from './records'
import {
  beginTransfer, cancelTransfer, deliverTransfer, detachTransfer, liveOfferOf, pendingOfferFor, probeTransfer, retryTransfer,
  type DeliverOutcome, type ShellTransfer,
} from './shell'
import { subscribeWindowTransferTargets, windowTransferTarget, type TransferPoint, type TransferProbeAnswer } from './targets'

/** The phone layout of the desktop (DesktopRoute): one window at a time, the others are not loaded. */
export const isPhoneLayout = (): boolean => typeof matchMedia === 'function' && matchMedia('(max-width:768px)').matches
/** How long a window the Shell switched to may take to load and connect before it can be asked. */
const SWITCH_TIMEOUT = 15000

export interface TransferHit {
  windowId: string
  /** The window's box on screen (for the outline). */
  rect: { left: number; top: number; width: number; height: number }
  at: TransferPoint
}

interface Base { op: ShellTransfer; count: number; sourceApp: string }
export type TransferGesture =
  | (Base & { mode: 'drag'; x: number; y: number; hit?: TransferHit; answer?: TransferProbeAnswer | null; probing: boolean })
  | (Base & { mode: 'choose'; hover?: TransferHit; refused?: string; returnFocus: HTMLElement | null; /** One window at a time: choose from the list, then switch. */ phone: boolean })
  | (Base & { mode: 'paste'; windowId: string; answer?: TransferProbeAnswer | null; probing: boolean; returnFocus: HTMLElement | null })
  | (Base & { mode: 'delivering'; windowId: string })
  | (Base & { mode: 'switching'; windowId: string })

export interface TransferNotice {
  id: number
  key: string
  vars: Record<string, string | number>
  /** The window the result is about (its App is named in the text). */
  windowId?: string
  alert: boolean
  retry?: ShellTransfer
}

let gesture: TransferGesture | null = null
let notice: TransferNotice | null = null
let noticeId = 0
const listeners = new Set<() => void>()
let snapshot: { gesture: TransferGesture | null; notice: TransferNotice | null } = { gesture, notice }
const emit = () => { snapshot = { gesture, notice }; listeners.forEach((l) => l()) }
export const transferUi = {
  subscribe: (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener) } },
  getSnapshot: () => snapshot,
}
const set = (next: TransferGesture | null) => { gesture = next; emit() }

export function dismissNotice(): void { notice = null; emit() }
function show(n: Omit<TransferNotice, 'id'>): void { notice = { ...n, id: ++noticeId }; emit() }

/** The window under a point: the topmost element there that is not the Shell's transfer layer, if it belongs to a
 * window that is not minimized. Its point is in the window's content box, or its focus over the title bar. */
export function hitWindow(x: number, y: number): TransferHit | undefined {
  const top = document.elementsFromPoint(x, y).find((el) => !el.closest('[data-transfer-layer]'))
  const node = top?.closest<HTMLElement>('[data-window-id]')
  const windowId = node?.dataset.windowId
  if (!node || !windowId) return undefined
  const record = desktopUIStore.getSnapshot().runtime.windows.find((w) => w.id === windowId)
  if (!record || record.state === 'minimized') return undefined
  const box = node.getBoundingClientRect()
  const content = node.querySelector<HTMLElement>('[data-window-content]')?.getBoundingClientRect()
  const inside = content && x >= content.left && x < content.right && y >= content.top && y < content.bottom
  return { windowId, rect: { left: box.left, top: box.top, width: box.width, height: box.height }, at: inside ? { x: x - content.left, y: y - content.top } : 'focus' }
}

const countOf = (op: ShellTransfer) => op.items.length

function report(outcome: DeliverOutcome, op: ShellTransfer): void {
  const vars = { count: outcome.items.filter((i) => i.status === 'done' || i.status === 'accepted').length }
  const windowId = outcome.windowId
  if (outcome.status === 'pending') return show({ key: 'transfer.result.pending', vars, windowId, alert: false, retry: op })
  if (outcome.status === 'rejected') {
    const key = outcome.reason === 'stale' ? 'transfer.result.stale' : outcome.reason === 'busy' ? 'transfer.result.busy' : 'transfer.result.rejected'
    return show({ key, vars, windowId, alert: true })
  }
  const done = outcome.items.filter((i) => i.status === 'done' || i.status === 'accepted').length
  const failed = outcome.items.filter((i) => i.status === 'failed').length
  if (outcome.status === 'failed' || !done) return show({ key: outcome.status === 'cancelled' ? 'transfer.result.cancelled' : 'transfer.result.failed', vars, windowId, alert: outcome.status !== 'cancelled' })
  if (failed) return show({ key: 'transfer.result.partial', vars: { done, failed }, windowId, alert: true })
  show({ key: outcome.status === 'accepted' ? 'transfer.result.accepted' : 'transfer.result.done', vars, windowId, alert: false })
}

async function deliver(op: ShellTransfer, windowId: string, at: TransferPoint, base: Base): Promise<void> {
  set({ ...base, mode: 'delivering', windowId })
  const outcome = await deliverTransfer(op, windowId, at)
  if (gesture?.op === op) set(null)
  report(outcome, op)
}

export async function retryNotice(op: ShellTransfer): Promise<void> {
  dismissNotice()
  report(await retryTransfer(op), op)
}

/** Ends whatever the transfer UI is doing without delivering. */
export function cancelGesture(options: { silent?: boolean } = {}): void {
  const current = gesture
  if (!current || current.mode === 'delivering' || current.mode === 'switching') return
  cancelTransfer(current.op)
  set(null)
  if ('returnFocus' in current) current.returnFocus?.focus()
  if (!options.silent) show({ key: 'transfer.result.nothing', vars: {}, alert: false })
}

// The source withdrew its selection (navigation, reload, a new selection) while the user was choosing: nothing goes.
desktopTransfers.subscribe(() => {
  if (gesture && gesture.mode !== 'delivering' && gesture.mode !== 'switching' && !gesture.op.detached && !desktopTransfers.isLive(gesture.op.offerId)) {
    cancelGesture({ silent: true })
    show({ key: 'transfer.result.stale', vars: {}, alert: true })
  }
})

function start(offer: TransferOffer | undefined): Base | undefined {
  if (gesture) cancelGesture({ silent: true })
  const op = offer ? beginTransfer(offer.offerId) : undefined
  return op ? { op, count: countOf(op), sourceApp: op.sourceApp } : undefined
}

// ─── Drag from the handle ───

/**
 * Pointer down on a window's handle: the handle captures the pointer. A move beyond a few pixels is a drag (probe the
 * window under the pointer, deliver on release); a release without one is a tap, which starts choosing.
 */
export function startHandleGesture(windowId: string, event: PointerEvent, handle: HTMLElement): void {
  const offer = liveOfferOf(windowId)
  if (!offer || event.button !== 0) return
  try { handle.setPointerCapture(event.pointerId) } catch { return }
  const origin = { x: event.clientX, y: event.clientY }
  let base: Base | undefined
  let released = false
  let frame = 0
  let latest = origin
  let inFlight = false
  let probedKey = ''
  let wanted: TransferHit | undefined
  /** Bumped when a window's registration changes: an answer asked for before that no longer counts. */
  let epoch = 0
  const keyOf = (hit: TransferHit) => `${hit.windowId}:${typeof hit.at === 'string' ? hit.at : `${Math.round(hit.at.x / 8)},${Math.round(hit.at.y / 8)}`}`
  // One probe at a time; when it answers, the window under the pointer by then is probed if it moved on.
  const probe = () => {
    if (!base || gesture?.mode !== 'drag' || gesture.op !== base.op || inFlight) return
    const hit = wanted
    if (!hit || hit.windowId === base.op.sourceWindowId) { probedKey = ''; return }
    const key = keyOf(hit)
    if (key === probedKey) return
    inFlight = true
    set({ ...gesture, probing: true })
    const op = base.op
    const asked = epoch
    void probeTransfer(op, hit.windowId, hit.at).then((outcome) => {
      inFlight = false
      if (gesture?.mode !== 'drag' || gesture.op !== op || outcome.seq !== op.seq) return
      if (asked !== epoch) { probe(); return }
      probedKey = key
      set({ ...gesture, probing: false, ...(gesture.hit?.windowId === outcome.windowId ? { answer: outcome.ok ? outcome.answer : null } : {}) })
      if (wanted && keyOf(wanted) !== key) probe()
    })
  }
  const update = () => {
    frame = 0
    if (!base || gesture?.mode !== 'drag' || gesture.op !== base.op) return
    wanted = hitWindow(latest.x, latest.y)
    if (wanted?.windowId === base.op.sourceWindowId) wanted = undefined
    set({ ...gesture, x: latest.x, y: latest.y, hit: wanted, ...(wanted?.windowId !== gesture.hit?.windowId ? { answer: undefined } : {}) })
    if (!wanted) probedKey = ''
    probe()
  }
  const onMove = (e: PointerEvent) => {
    if (e.pointerId !== event.pointerId) return
    latest = { x: e.clientX, y: e.clientY }
    if (!base) {
      if (Math.hypot(latest.x - origin.x, latest.y - origin.y) < 6) return
      base = start(offer)
      if (!base) return finish()
      set({ ...base, mode: 'drag', x: latest.x, y: latest.y, probing: false })
    }
    if (!frame) frame = requestAnimationFrame(update)
  }
  const onUp = (e: PointerEvent) => {
    if (e.pointerId !== event.pointerId) return
    released = true
    finish()
    if (!base) { startChoosing(windowId, handle); return }
    if (gesture?.mode !== 'drag' || gesture.op !== base.op) return
    const hit = hitWindow(e.clientX, e.clientY)
    if (!hit || hit.windowId === base.op.sourceWindowId) { cancelGesture(); return }
    void deliver(base.op, hit.windowId, hit.at, base)
  }
  const abort = () => { finish(); if (base && gesture?.op === base.op) cancelGesture() }
  const onCancel = (e: PointerEvent) => { if (e.pointerId === event.pointerId) abort() }
  // Capture lost before the release (another gesture took over, the element went away) cancels; after a normal
  // release it only cleans up.
  const onLost = (e: PointerEvent) => { if (e.pointerId === event.pointerId && !released) abort() }
  const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); abort() } }
  const onBlur = () => abort()
  // The window under the pointer reloaded or went away: its earlier answer no longer counts.
  const stopTargets = subscribeWindowTransferTargets(() => {
    epoch += 1
    probedKey = ''
    if (base && gesture?.mode === 'drag' && gesture.op === base.op) set({ ...gesture, answer: undefined })
    probe()
  })
  function finish() {
    stopTargets()
    if (frame) cancelAnimationFrame(frame)
    handle.removeEventListener('pointermove', onMove)
    handle.removeEventListener('pointerup', onUp)
    handle.removeEventListener('pointercancel', onCancel)
    handle.removeEventListener('lostpointercapture', onLost)
    window.removeEventListener('keydown', onKey, true)
    window.removeEventListener('blur', onBlur)
    try { if (handle.hasPointerCapture(event.pointerId)) handle.releasePointerCapture(event.pointerId) } catch { /* gone */ }
  }
  handle.addEventListener('pointermove', onMove)
  handle.addEventListener('pointerup', onUp)
  handle.addEventListener('pointercancel', onCancel)
  handle.addEventListener('lostpointercapture', onLost)
  window.addEventListener('keydown', onKey, true)
  window.addEventListener('blur', onBlur)
  // Esc must reach the Shell even when the keyboard focus was inside an App's frame.
  handle.focus({ preventScroll: true })
  event.preventDefault()
}

// ─── Tap, then tap a destination (or choose it from the list) ───

/** Starts choosing a destination for the window's live selection: a tap on a window, or one of the listed windows. */
export function startChoosing(windowId: string, returnFocus: HTMLElement | null, phone = isPhoneLayout()): void {
  const base = start(liveOfferOf(windowId))
  if (base) set({ ...base, mode: 'choose', returnFocus, phone })
}

/** Hover feedback while choosing (a mouse; a finger has no hover). */
export function hoverChoice(x: number, y: number): void {
  if (gesture?.mode !== 'choose') return
  const hit = hitWindow(x, y)
  if (hit?.windowId !== gesture.hover?.windowId) set({ ...gesture, hover: hit && hit.windowId !== gesture.op.sourceWindowId ? hit : undefined })
}

/** The user picked a destination: at a point (a tap) or a window's focus (the list, the keyboard). */
export async function chooseDestination(windowId: string, at: TransferPoint): Promise<void> {
  const current = gesture
  if (current?.mode !== 'choose' || windowId === current.op.sourceWindowId) return
  if (current.phone) return sendToWindow(current, windowId)
  const outcome = await probeTransfer(current.op, windowId, at)
  if (gesture !== current) return
  if (!outcome.ok) { set({ ...current, refused: windowId }); return }
  desktopUIStore.focusWindow(windowId)
  await deliver(current.op, windowId, at, current)
}

export function chooseAt(x: number, y: number): void {
  const hit = hitWindow(x, y)
  if (!hit) { cancelGesture(); return }
  void chooseDestination(hit.windowId, hit.at)
}

/**
 * A phone shows one window at a time: the Shell reads the selection while its window is still loaded, switches to the
 * chosen window, waits until it can receive, and asks the user to confirm there.
 */
async function sendToWindow(current: Extract<TransferGesture, { mode: 'choose' }>, windowId: string): Promise<void> {
  const { op } = current
  set({ ...current, mode: 'switching', windowId })
  if (!await detachTransfer(op)) {
    cancelTransfer(op)
    if (gesture?.op === op) set(null)
    return show({ key: 'transfer.result.stale', vars: {}, alert: true })
  }
  const record = desktopUIStore.getSnapshot().runtime.windows.find((w) => w.id === windowId)
  if (record) desktopUIStore.openAppWindow(record.appId, { windowId })
  const ready = await new Promise<boolean>((resolve) => {
    if (windowTransferTarget(windowId)) return resolve(true)
    const timer = setTimeout(() => { stop(); resolve(false) }, SWITCH_TIMEOUT)
    const stop = subscribeWindowTransferTargets(() => { if (windowTransferTarget(windowId)) { clearTimeout(timer); stop(); resolve(true) } })
  })
  if (gesture?.op !== op) return
  if (!ready) {
    cancelTransfer(op)
    set(null)
    return show({ key: 'transfer.result.rejected', vars: {}, windowId, alert: true })
  }
  set({ op, count: current.count, sourceApp: current.sourceApp, mode: 'paste', windowId, probing: true, returnFocus: null })
  const outcome = await probeTransfer(op, windowId, 'focus')
  if (gesture?.mode === 'paste' && gesture.op === op && outcome.seq === op.seq) set({ ...gesture, answer: outcome.ok ? outcome.answer : null, probing: false })
}

// ─── Paste desktop object… ───

/** Opens "Paste desktop object…" for a window: the pending object of another window, probed at the window's focus. */
export function startPaste(windowId: string, returnFocus: HTMLElement | null): void {
  const base = start(pendingOfferFor(windowId))
  if (!base) return
  set({ ...base, mode: 'paste', windowId, probing: true, returnFocus })
  void probeTransfer(base.op, windowId, 'focus').then((outcome) => {
    if (gesture?.mode === 'paste' && gesture.op === base.op && outcome.seq === base.op.seq) set({ ...gesture, answer: outcome.ok ? outcome.answer : null, probing: false })
  })
}

export function confirmPaste(): void {
  const current = gesture
  if (current?.mode !== 'paste') return
  void deliver(current.op, current.windowId, 'focus', current).then(() => current.returnFocus?.focus())
}

/** Whether a window has something for "Paste desktop object…". */
export const hasPendingOffer = (windowId: string): boolean => !!pendingOfferFor(windowId)
