/* The Shell's transfer router (剪贴板规范 §5): one operation per drag or paste through the Shell. The router shows
 * targets only kinds, formats and operations, probes again at the final point, reads from the source only once the
 * user let go on or chose a destination, and delivers only to that destination. A rejected, timed-out or stale answer
 * never delivers anywhere; a delivery without a receipt in time is "pending check" and a retry reuses its id.
 *
 * Sources are records of `desktopTransfers` with items (an App frame's `transfer.offer`, or a built-in selection
 * registered with `offerWindowSelection`); targets are `windowTransferTarget(windowId)`. */
import { newOfferId } from 'buckyos/transfer'
import { desktopTransfers, type TransferOffer, type TransferOfferItem, type TransferOperation } from './records'
import {
  windowTransferTarget,
  type TransferDelivery, type TransferDeliveryItem, type TransferDeliveryResult, type TransferItemStatus,
  type TransferPoint, type TransferProbeAnswer, type TransferProbeItem,
} from './targets'

const PROBE_TIMEOUT = 1500
const READ_TIMEOUT = 10000
const DELIVER_TIMEOUT = 15000

/** Why nothing was delivered, or what came back. */
export type TransferFailure = 'unsupported' | 'stale' | 'denied' | 'timeout' | 'no-target' | 'busy'

export interface ShellTransfer {
  readonly operationId: string
  readonly offerId: string
  readonly sourceWindowId?: string
  readonly sourceApp: string
  /** The items as targets see them, fixed when the operation began. */
  readonly items: TransferProbeItem[]
  state: 'open' | 'delivering' | 'pending' | 'finished' | 'cancelled'
  seq: number
  /** Every representation was read into the Shell (a phone switching windows unloads the source): the operation no
   * longer needs the source. */
  detached?: boolean
  /** Fixed once the data was read: a retry sends exactly this again. */
  delivery?: { windowId: string; value: TransferDelivery; skipped: string[] }
  result?: TransferDeliveryResult
}

export type ProbeOutcome =
  | { ok: true; windowId: string; seq: number; answer: TransferProbeAnswer }
  | { ok: false; windowId: string; seq: number; reason: TransferFailure | 'none' }

export type DeliverOutcome =
  | { status: TransferDeliveryResult['status']; windowId: string; items: TransferDeliveryResult['items'] }
  | { status: 'pending'; windowId: string; items: TransferDeliveryResult['items'] }
  | { status: 'rejected'; windowId: string; reason: TransferFailure | 'none'; items: TransferDeliveryResult['items'] }

class TimeoutError extends Error { constructor() { super('timeout') } }
const within = <T>(promise: Promise<T> | T, ms: number): Promise<T> => new Promise<T>((resolve, reject) => {
  const timer = setTimeout(() => reject(new TimeoutError()), ms)
  Promise.resolve(promise).then((value) => { clearTimeout(timer); resolve(value) }, (error) => { clearTimeout(timer); reject(error) })
})
/** The code a failed probe, read or delivery stands for (`TransferError` of the SDK, a timeout, anything else). */
function failureOf(error: unknown): TransferFailure {
  if (error instanceof TimeoutError) return 'timeout'
  const code = (error as { code?: unknown } | null)?.code ?? (error instanceof Error ? error.message : undefined)
  return code === 'stale' || code === 'denied' || code === 'unsupported' || code === 'timeout' || code === 'busy' ? code : 'unsupported'
}

/** The answer kept only where it fits the probe: items it showed, formats and operations they allow, once each. */
function checkAnswer(answer: TransferProbeAnswer | null | undefined, items: TransferProbeItem[]): TransferProbeAnswer | null {
  if (!answer || typeof answer.targetId !== 'string' || !answer.targetId || !Array.isArray(answer.accept)) return null
  const shown = new Map(items.map((item) => [item.itemKey, item]))
  const seen = new Set<string>()
  const accept = answer.accept.filter((a) => {
    const item = shown.get(a?.itemKey)
    if (!item || seen.has(a.itemKey) || !item.formats.includes(a.format) || !item.operations.includes(a.operation)) return false
    seen.add(a.itemKey)
    return true
  })
  return accept.length ? { targetId: answer.targetId, accept, action: typeof answer.action === 'string' ? answer.action.slice(0, 200) : '' } : null
}

const operations = new Map<string, ShellTransfer>()

/** Starts an operation on a routable record; undefined when the record is gone or has no items. */
export function beginTransfer(offerId: string): ShellTransfer | undefined {
  const offer = desktopTransfers.get(offerId)
  const items = desktopTransfers.probeItems(offerId)
  if (!offer || !items?.length || !desktopTransfers.isLive(offerId)) return undefined
  const operationId = newOfferId()
  desktopTransfers.hold(offerId, operationId)
  const op: ShellTransfer = { operationId, offerId, sourceWindowId: offer.source.windowId, sourceApp: offer.source.app, items, state: 'open', seq: 0 }
  operations.set(operationId, op)
  return op
}

/** Asks a window what it would do with the items at a point of its content box or at its focus. The answer to an
 * older probe of the same operation comes back with an older `seq`: callers keep only the newest. */
export async function probeTransfer(op: ShellTransfer, windowId: string, at: TransferPoint): Promise<ProbeOutcome> {
  const seq = ++op.seq
  if (op.state !== 'open') return { ok: false, windowId, seq, reason: 'stale' }
  if (windowId === op.sourceWindowId) return { ok: false, windowId, seq, reason: 'none' }
  const target = windowTransferTarget(windowId)
  if (!target) return { ok: false, windowId, seq, reason: 'no-target' }
  if (!op.detached && !desktopTransfers.isLive(op.offerId)) return { ok: false, windowId, seq, reason: 'stale' }
  try {
    const answer = checkAnswer(await within(target.probe({ operationId: op.operationId, seq, items: op.items, at }), PROBE_TIMEOUT), op.items)
    return answer ? { ok: true, windowId, seq, answer } : { ok: false, windowId, seq, reason: 'none' }
  } catch (error) {
    return { ok: false, windowId, seq, reason: failureOf(error) }
  }
}

/**
 * Reads every offered representation of every item into the Shell, once the user chose a window the Shell must switch
 * to before it can ask it anything (a phone shows one window at a time and unloads the others). True when at least
 * one item could be read; the operation then goes on without its source.
 */
export async function detachTransfer(op: ShellTransfer): Promise<boolean> {
  if (op.state !== 'open') return false
  const reads = op.items.flatMap((item) => item.formats.map((format) =>
    within(desktopTransfers.readItem(op.offerId, item.itemKey, format, op.operationId), READ_TIMEOUT).then(() => item.itemKey, () => undefined)))
  const read = new Set((await Promise.all(reads)).filter((key): key is string => key !== undefined))
  if (!read.size || op.state !== 'open') return false
  op.detached = true
  return true
}

const allItems = (op: ShellTransfer, status: TransferItemStatus, error?: string) => op.items.map(({ itemKey }) => ({ itemKey, status, ...(error ? { error } : {}) }))

/**
 * Delivers to the window the user let go on or chose: probes again at that point, reads the accepted items from the
 * source, holds cut items for this operation, and hands them over once. Nothing is delivered when the final probe
 * does not accept.
 */
export async function deliverTransfer(op: ShellTransfer, windowId: string, at: TransferPoint): Promise<DeliverOutcome> {
  if (op.state !== 'open') return { status: 'rejected', windowId, reason: 'stale', items: allItems(op, 'skipped') }
  const final = await probeTransfer(op, windowId, at)
  if (!final.ok) { finishTransfer(op); return { status: 'rejected', windowId, reason: final.reason, items: allItems(op, 'skipped') } }
  op.state = 'delivering'
  const offer = desktopTransfers.get(op.offerId)
  if (!offer) { finishTransfer(op); return { status: 'rejected', windowId, reason: 'stale', items: allItems(op, 'skipped') } }
  const operation = final.answer.accept[0].operation
  const chosen = final.answer.accept.filter((a) => a.operation === operation)
  const skipped = op.items.filter((item) => !chosen.some((a) => a.itemKey === item.itemKey)).map((item) => item.itemKey)
  const failed: TransferDeliveryResult['items'] = []
  const values: TransferDeliveryItem[] = []
  await Promise.all(chosen.map(async ({ itemKey, format }) => {
    try {
      values.push({ itemKey, format, value: await within(desktopTransfers.readItem(op.offerId, itemKey, format, op.operationId), READ_TIMEOUT) })
    } catch (error) {
      failed.push({ itemKey, status: 'failed', error: failureOf(error) })
    }
  }))
  const skippedItems = skipped.map((itemKey) => ({ itemKey, status: 'skipped' as const }))
  if (!values.length) {
    finishTransfer(op)
    const reason = failed.every((f) => f.error === 'stale') ? 'stale' : failed[0]?.error as TransferFailure | undefined ?? 'none'
    return { status: 'rejected', windowId, reason, items: [...failed, ...skippedItems] }
  }
  values.sort((a, b) => Number(a.itemKey) - Number(b.itemKey))
  // A cut moves on delivery: each item is held for this operation only, and taken only once the move is done.
  const keys = values.map((v) => offer.itemKeys[Number(v.itemKey)])
  if (offer.purpose === 'cut' && operation === 'move') {
    const reserved = desktopTransfers.reserve(op.offerId, keys, op.operationId)
    if (!reserved.ok) { finishTransfer(op); return { status: 'rejected', windowId, reason: reserved.reason, items: allItems(op, 'failed', reserved.reason) } }
  }
  op.delivery = { windowId, skipped, value: { operationId: op.operationId, targetId: final.answer.targetId, operation, at, items: values } }
  const outcome = await sendDelivery(op)
  return outcome.status === 'pending' || outcome.status === 'rejected' ? outcome : { ...outcome, items: [...outcome.items, ...failed, ...skippedItems] }
}

async function sendDelivery(op: ShellTransfer): Promise<DeliverOutcome> {
  const { windowId, value } = op.delivery!
  const target = windowTransferTarget(windowId)
  if (!target) { settle(op, null); return { status: 'rejected', windowId, reason: 'no-target', items: value.items.map(({ itemKey }) => ({ itemKey, status: 'failed' as const, error: 'no-target' })) } }
  op.state = 'delivering'
  try {
    const result = await within(target.deliver(value), DELIVER_TIMEOUT)
    settle(op, result)
    return { status: result.status, windowId, items: result.items }
  } catch (error) {
    const reason = failureOf(error)
    if (reason === 'timeout') {
      op.state = 'pending'
      return { status: 'pending', windowId, items: value.items.map(({ itemKey }) => ({ itemKey, status: 'accepted' as const, error: 'pending' })) }
    }
    settle(op, null)
    return { status: 'rejected', windowId, reason, items: value.items.map(({ itemKey }) => ({ itemKey, status: 'failed' as const, error: reason })) }
  }
}

/** Applies a final result to the record (a cut gives up the items that moved, gets back the others) and ends. */
function settle(op: ShellTransfer, result: TransferDeliveryResult | null): void {
  const offer = desktopTransfers.get(op.offerId)
  if (offer && op.delivery && offer.purpose === 'cut' && op.delivery.value.operation === 'move') {
    const key = (itemKey: string) => offer.itemKeys[Number(itemKey)]
    const done = (result?.items ?? []).filter((i) => i.status === 'done').map((i) => key(i.itemKey))
    const rest = op.delivery.value.items.map((i) => key(i.itemKey)).filter((k) => !done.includes(k))
    if (done.length) desktopTransfers.consume(op.offerId, done, op.operationId)
    if (rest.length) desktopTransfers.release(op.offerId, rest, op.operationId)
  }
  if (result) op.result = result
  finishTransfer(op)
}

/** Checks a delivery that is pending again, with its original operation id: the target's status when it has one,
 * otherwise the same delivery once more (targets commit once per operation id). */
export async function retryTransfer(op: ShellTransfer): Promise<DeliverOutcome> {
  if (op.state !== 'pending' || !op.delivery) return { status: 'rejected', windowId: op.delivery?.windowId ?? '', reason: 'stale', items: [] }
  const { windowId, value } = op.delivery
  const target = windowTransferTarget(windowId)
  if (target?.status) {
    try {
      const result = await within(target.status(op.operationId, value.items), DELIVER_TIMEOUT)
      if (result) { settle(op, result); return { status: result.status, windowId, items: result.items } }
    } catch { /* ask with the delivery itself */ }
  }
  return sendDelivery(op)
}

/** Drops an operation that was not delivered (Esc, a cancelled gesture, a closed dialog). */
export function cancelTransfer(op: ShellTransfer): void {
  if (op.state === 'finished' || op.state === 'cancelled') return
  if (op.state === 'pending' && op.delivery) windowTransferTarget(op.delivery.windowId)?.cancel?.(op.operationId)
  op.state = 'cancelled'
  release(op)
}

function finishTransfer(op: ShellTransfer): void {
  if (op.state === 'cancelled') return
  op.state = 'finished'
  release(op)
}

function release(op: ShellTransfer): void {
  operations.delete(op.operationId)
  desktopTransfers.unhold(op.offerId, op.operationId)
}

// ─── Offers the Shell can route ───

/** The newest offer of a window the drag handle carries (a drag or copy selection); undefined when there is none. */
export function liveOfferOf(windowId: string): TransferOffer | undefined {
  return desktopTransfers.routable().find((r) => r.source.windowId === windowId && (r.purpose === 'drag' || r.purpose === 'copy'))
}

/** What "Paste desktop object…" puts into a window: the newest copied or cut selection of another window, else the
 * newest selection another window offers. */
export function pendingOfferFor(windowId: string): TransferOffer | undefined {
  const others = desktopTransfers.routable().filter((r) => r.source.windowId !== windowId)
  return others.find((r) => r.purpose !== 'drag') ?? others[0]
}

export interface WindowSelectionOffer {
  /** `drag` (default): the live selection, for the drag handle; `copy` / `cut` also make it the pending desktop object. */
  purpose?: 'drag' | 'copy' | 'cut'
  items: TransferOfferItem[]
  /** Defaults to copy. */
  operations?: TransferOperation[]
  scope?: Record<string, string>
}

const selections = new Map<string, string>()

/**
 * A built-in App's selection for the Shell (the drag handle in its window's title bar, "Paste desktop object…" in other
 * windows). A newer selection of the same window replaces the previous one. Returns the function that withdraws it;
 * call it when the selection goes away or the window closes.
 */
export function offerWindowSelection(app: string, windowId: string, selection: WindowSelectionOffer): () => void {
  const previous = selections.get(windowId)
  if (previous) desktopTransfers.withdraw(previous)
  const offer = desktopTransfers.offer({
    purpose: selection.purpose ?? 'drag', source: { app, windowId }, scope: selection.scope,
    itemKeys: selection.items.map((_, index) => String(index)), items: selection.items, formats: {},
    operations: selection.operations ?? ['copy'],
  })
  selections.set(windowId, offer.offerId)
  return () => {
    if (selections.get(windowId) === offer.offerId) selections.delete(windowId)
    desktopTransfers.withdraw(offer.offerId)
  }
}
