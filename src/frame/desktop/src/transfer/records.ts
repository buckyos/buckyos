/* Desktop transfer records (剪贴板规范 §3.4–§3.6, §6): what the desktop keeps behind a transfer handle — a cut that
 * moves on paste, a large or late representation, a private selection that is fetched through the Shell. A record
 * only ever answers a paste or drop that carries its handle; nothing here reads the system clipboard.
 *
 * Records live in this top-level desktop document, for the signed-in user and Zone: a reload, sign-out or user switch
 * drops them. Cut items are reserved one operation at a time and consumed only when the move succeeded. */
import { embedTransferHandle, newOfferId, TRANSFER_HANDLE_FORMAT, transferHandleValue, type TransferRepresentations } from 'buckyos/transfer'
import type { TransferProbeItem } from './targets'

export type TransferPurpose = 'copy' | 'cut' | 'drag'
export type TransferOperation = 'copy' | 'move' | 'link'

export interface TransferOfferInput {
  purpose: TransferPurpose
  /** The App and window that made the selection (the Shell's own knowledge, never what an App says about itself). */
  source: { app: string; windowId?: string }
  /** Where the private formats apply (e.g. `{ workspaceId }`); a target outside it falls to the next layer. */
  scope?: Record<string, string>
  /** The selection's item keys, in order. A cut consumes them one by one. */
  itemKeys: string[]
  /** The private formats this record holds by registered name; a function is called on the first read. */
  formats: Record<string, unknown>
  /** Operations the source allows. */
  operations: TransferOperation[]
  /** The items one by one, in `itemKeys` order, for the Shell's router (drag handle, "Paste desktop object…"). A
   * record without them is only reached through its transfer handle. */
  items?: TransferOfferItem[]
}

/** One item of a record as the router delivers it. */
export interface TransferOfferItem {
  kind: TransferProbeItem['kind']
  /** Registered format → value (see `TransferDeliveryItem` for the value types); a function is called on the first
   * read, after the user chose where the item goes, with the operation that reads it. */
  formats: Record<string, unknown>
  /** Defaults to the record's operations. */
  operations?: TransferOperation[]
}

export interface TransferOffer extends Readonly<TransferOfferInput> {
  readonly offerId: string
  readonly createdAt: number
  /** Item keys not yet moved (a cut) — all of them otherwise. */
  readonly remaining: ReadonlySet<string>
}

interface Entry extends TransferOffer {
  remaining: Set<string>
  /** itemKey → operationId holding it. */
  reserved: Map<string, string>
  values: Map<string, Promise<unknown>>
  /** Operations reading or delivering this record: they keep it alive until they finish. */
  holds: Set<string>
  /** Withdrawn by its source while an operation held it. */
  withdrawn?: boolean
}

export type ReserveResult = { ok: true } | { ok: false; reason: 'stale' | 'busy' }

/** Clipboard records kept besides the newest (a newer copy replaces the system clipboard anyway). */
const MAX_RECORDS = 16

class DesktopTransfers {
  private records = new Map<string, Entry>()
  private listeners = new Set<() => void>()
  private version = 0
  private sessionKey: string | null = null

  /** Registers a selection and returns its record; the caller writes `handleRepresentations(offer.offerId)`. */
  offer(input: TransferOfferInput): TransferOffer {
    let offerId = newOfferId()
    while (this.records.has(offerId)) offerId = newOfferId()
    return this.add(offerId, input)
  }

  /** Registers a selection under the id its source made (an App's `transfer.offer`); undefined when that id is not a
   * valid offer id or is already taken: an offer never replaces another. */
  offerWithId(offerId: string, input: TransferOfferInput): TransferOffer | undefined {
    if (!/^[A-Za-z0-9_-]{16,128}$/.test(offerId) || this.records.has(offerId)) return undefined
    return this.add(offerId, input)
  }

  private add(offerId: string, input: TransferOfferInput): TransferOffer {
    if (input.items && input.items.length !== input.itemKeys.length) throw new Error('A record needs one item per item key')
    // A new copy or cut replaces the system clipboard: an older cut can no longer be pasted, its marking goes.
    if (input.purpose !== 'drag') for (const record of [...this.records.values()]) if (record.purpose === 'cut' && !record.reserved.size) this.records.delete(record.offerId)
    const entry: Entry = {
      ...input, itemKeys: [...input.itemKeys], operations: [...input.operations], offerId, createdAt: Date.now(),
      remaining: new Set(input.itemKeys), reserved: new Map(), values: new Map(), holds: new Set(),
      ...(input.items ? { items: input.items.map((item) => ({ ...item, formats: { ...item.formats } })) } : {}),
    }
    this.records.set(offerId, entry)
    const clipboard = [...this.records.values()].filter((r) => r.purpose !== 'drag')
    for (const old of clipboard.slice(0, Math.max(0, clipboard.length - MAX_RECORDS))) if (!old.reserved.size && !old.holds.size) this.records.delete(old.offerId)
    this.changed()
    return entry
  }

  get(offerId: string | undefined): TransferOffer | undefined {
    return offerId ? this.records.get(offerId) : undefined
  }

  /** A private format of a record; undefined when the record is gone or does not hold it. */
  async read(offerId: string, format: string): Promise<unknown> {
    const record = this.records.get(offerId)
    if (!record || !(format in record.formats)) return undefined
    let value = record.values.get(format)
    if (!value) {
      const raw = record.formats[format]
      value = Promise.resolve(typeof raw === 'function' ? (raw as () => unknown)() : raw)
      record.values.set(format, value)
    }
    return value
  }

  /** The items of a record as a probe shows them: kind, formats, operations; no values. Undefined when the record is
   * gone or has no items. Items already moved (a cut) are left out. */
  probeItems(offerId: string): TransferProbeItem[] | undefined {
    const record = this.records.get(offerId)
    if (!record?.items) return undefined
    return record.items.flatMap((item, index) => {
      const itemKey = record.itemKeys[index]
      return record.remaining.has(itemKey) ? [{ itemKey: String(index), kind: item.kind, formats: Object.keys(item.formats), operations: [...(item.operations ?? record.operations)] }] : []
    })
  }

  /** One item's representation, by its position (`itemKey` of the probe); read once and kept for later operations.
   * Rejects when the record or the format is gone, or the source could not give it (a failed read is not kept). */
  async readItem(offerId: string, itemKey: string, format: string, operationId: string): Promise<unknown> {
    const record = this.records.get(offerId)
    const item = record?.items?.[Number(itemKey)]
    if (!record || !item || !/^\d+$/.test(itemKey) || !(format in item.formats)) throw new Error('stale')
    const key = `${itemKey}\n${format}`
    let value = record.values.get(key)
    if (!value) {
      const raw = item.formats[format]
      value = Promise.resolve().then(() => typeof raw === 'function' ? (raw as (context: { operationId: string }) => unknown)({ operationId }) : raw)
      record.values.set(key, value)
      value.catch(() => { if (record.values.get(key) === value) record.values.delete(key) })
    }
    return value
  }

  /** Keeps a record alive while an operation reads or delivers it; `unhold` when it finished. */
  hold(offerId: string, operationId: string): boolean {
    const record = this.records.get(offerId)
    if (!record) return false
    record.holds.add(operationId)
    return true
  }

  unhold(offerId: string, operationId: string): void {
    const record = this.records.get(offerId)
    if (!record?.holds.delete(operationId) || record.holds.size || !record.withdrawn) return
    this.records.delete(offerId)
    this.changed()
  }

  /** Holds items of a cut for one operation; a second operation on the same item is told it is busy. */
  reserve(offerId: string, itemKeys: string[], operationId: string): ReserveResult {
    const record = this.records.get(offerId)
    if (!record || itemKeys.some((key) => !record.remaining.has(key))) return { ok: false, reason: 'stale' }
    if (itemKeys.some((key) => { const holder = record.reserved.get(key); return holder !== undefined && holder !== operationId })) return { ok: false, reason: 'busy' }
    for (const key of itemKeys) record.reserved.set(key, operationId)
    this.changed()
    return { ok: true }
  }

  /** Gives back items whose operation failed or was cancelled. */
  release(offerId: string, itemKeys: string[], operationId: string): void {
    const record = this.records.get(offerId)
    if (!record) return
    for (const key of itemKeys) if (record.reserved.get(key) === operationId) record.reserved.delete(key)
    this.changed()
  }

  /** Marks items as moved: a cut record goes away with its last item. */
  consume(offerId: string, itemKeys: string[], operationId: string): void {
    const record = this.records.get(offerId)
    if (!record) return
    for (const key of itemKeys) {
      if (record.reserved.get(key) !== operationId) continue
      record.reserved.delete(key)
      record.remaining.delete(key)
    }
    if (!record.remaining.size) this.records.delete(offerId)
    this.changed()
  }

  /** Drops a record that was not used (a cancelled cut, a finished drag). Items in flight keep it alive; an operation
   * holding it lets it go when it finishes. */
  withdraw(offerId: string): void {
    const record = this.records.get(offerId)
    if (!record || record.reserved.size) return
    if (record.holds.size) { record.withdrawn = true; this.changed(); return }
    this.records.delete(offerId)
    this.changed()
  }

  /** Whether a record is still offered (not withdrawn by its source). */
  isLive(offerId: string): boolean {
    const record = this.records.get(offerId)
    return !!record && !record.withdrawn
  }

  /** Live records with items, newest first (what the Shell's router can deliver). */
  routable(): TransferOffer[] {
    return [...this.records.values()].filter((r) => r.items && !r.withdrawn && r.remaining.size).reverse()
  }

  /** Drops the records a window made (its document closed). */
  withdrawSource(windowId: string): void {
    for (const record of [...this.records.values()]) if (record.source.windowId === windowId) this.withdraw(record.offerId)
  }

  /** The live cut of a source window, for its marking. */
  cutOf(app: string, windowId?: string): TransferOffer | undefined {
    return [...this.records.values()].reverse().find((r) => r.purpose === 'cut' && r.source.app === app && r.source.windowId === windowId)
  }

  /** The live drag an App started in this document (one drag at a time), for hover feedback only: a drop still reads
   * the handle it carries. */
  dragOf(app: string): TransferOffer | undefined {
    return [...this.records.values()].reverse().find((r) => r.purpose === 'drag' && r.source.app === app)
  }

  /** Records belong to one user and Zone: a different session drops them all. */
  bindSession(key: string): void {
    if (this.sessionKey !== null && this.sessionKey !== key) this.clear()
    this.sessionKey = key
  }

  clear(): void {
    this.records.clear()
    this.changed()
  }

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener)
    return () => this.listeners.delete(listener)
  }

  getVersion = (): number => this.version

  private changed() {
    this.version += 1
    this.listeners.forEach((listener) => listener())
  }
}

export const desktopTransfers = new DesktopTransfers()

/** The handle of a record as one write carries it: the dedicated MIME type, plus the copy embedded in `text/html`
 * (a paste through the async clipboard does not see a copy event's custom types). */
export function withTransferHandle(representations: TransferRepresentations, offerId: string, options: { cut?: boolean } = {}): TransferRepresentations {
  const out: TransferRepresentations = { ...representations, [TRANSFER_HANDLE_FORMAT]: transferHandleValue(offerId, options) }
  if (out['text/html'] !== undefined) out['text/html'] = embedTransferHandle(out['text/html'], offerId, options)
  return out
}
