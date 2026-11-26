/* Window transfer targets (剪贴板规范 §5): how the Shell asks a window what it would do with a selection at a point
 * (or at its focus) and then hands it the chosen representations. Built-in Apps register their windows here; the
 * App Frame host registers an adapter for each `transfer.v1` iframe. The Shell's router (cross-window drag handle,
 * "Paste desktop object…") is the only caller. */
import type { TransferOperation } from './records'

/** One item as a probe shows it: no names, paths, object ids, text or file bytes before the user lets go (§5). */
export interface TransferProbeItem {
  /** The item's key within this selection (`0`, `1`, …). */
  itemKey: string
  kind: 'zone-file' | 'zone-folder' | 'local-file' | 'text' | 'object'
  /** Registered format names the source can give for this item (no `web ` prefix). */
  formats: string[]
  operations: TransferOperation[]
}

/** A point in the window's content box, in CSS pixels, or the window's current focus (a paste command). */
export type TransferPoint = { x: number; y: number } | 'focus'

export interface TransferProbe {
  operationId: string
  /** Increases with every probe of one operation; an answer to an older probe is ignored. */
  seq: number
  items: TransferProbeItem[]
  at: TransferPoint
}

export interface TransferProbeAnswer {
  /** The target's own id for the drop place (a folder, a canvas, an attachment draft…), given back on delivery. */
  targetId: string
  /** For each accepted item: the representation and operation the target chose. */
  accept: Array<{ itemKey: string; format: string; operation: TransferOperation }>
  /** What the user is told will happen ("Copy into Projects", "Add to the attachment draft"). */
  action: string
}

/**
 * One delivered representation. `value` by format: a string for text and JSON formats (`text/plain`, `text/html`,
 * `text/uri-list`, App custom `+json` formats); for `application/vnd.buckyos.content-refs+json` the item's own
 * `ZoneContent` (one per item, not a list); for `files` a `File`; a `Blob` for binary formats (`image/png`).
 */
export interface TransferDeliveryItem { itemKey: string; format: string; value: unknown }

export interface TransferDelivery {
  operationId: string
  targetId: string
  operation: TransferOperation
  at: TransferPoint
  items: TransferDeliveryItem[]
}

export type TransferItemStatus = 'done' | 'accepted' | 'skipped' | 'failed' | 'cancelled'
export interface TransferDeliveryResult {
  /** done: committed; accepted: taken, waiting for the user (a draft, an import wizard); cancelled; failed. */
  status: 'done' | 'accepted' | 'cancelled' | 'failed'
  items: Array<{ itemKey: string; status: TransferItemStatus; error?: string }>
}

export interface WindowTransferTarget {
  /** What this window would do with the items at the point; null when it takes none of them. Must not read, import,
   * change layout or start uploads. */
  probe(probe: TransferProbe): TransferProbeAnswer | null | Promise<TransferProbeAnswer | null>
  /** Checks the drop place, version and permissions again and commits once per `operationId`. */
  deliver(delivery: TransferDelivery): Promise<TransferDeliveryResult>
  /** The final result of an earlier delivery whose receipt did not arrive in time; null when unknown. Without it, a
   * check sends the same delivery again. */
  status?(operationId: string, items: TransferDeliveryItem[]): Promise<TransferDeliveryResult | null>
  /** The Shell gave up on an operation that had not reported a result. */
  cancel?(operationId: string): void
}

const targets = new Map<string, WindowTransferTarget>()
const listeners = new Set<() => void>()
let version = 0
const changed = () => { version += 1; listeners.forEach((listener) => listener()) }

/** A window's target (one per window; a later registration replaces the earlier one). */
export function registerWindowTransferTarget(windowId: string, target: WindowTransferTarget): () => void {
  targets.set(windowId, target)
  changed()
  return () => { if (targets.get(windowId) === target) { targets.delete(windowId); changed() } }
}

/** Follows registrations (the window menu shows "Paste desktop object…" only for windows with a target). */
export function subscribeWindowTransferTargets(listener: () => void): () => void {
  listeners.add(listener)
  return () => { listeners.delete(listener) }
}
export const windowTransferTargetsVersion = (): number => version

export const windowTransferTarget = (windowId: string): WindowTransferTarget | undefined => targets.get(windowId)
