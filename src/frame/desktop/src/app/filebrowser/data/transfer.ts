/* Files on the clipboard and in drags (doc/desktop/BuckyOS 剪贴板管理与拖拽支持规范.md §2.2, §2.3, §3, §6).
 *
 * A copy, cut or drag writes the Zone reference list with its readable representations (names, `cyfs://` links) for
 * every App, plus the handle of a desktop record that holds the captured entries for Files itself: a cut moves on paste
 * through that record, and computer files (which have no Zone reference) can still be pasted in this desktop. A paste
 * or drop is read in the registered order: a live Files record, Zone references (from any App), local files, BuckyOS
 * addresses (only after the user chooses), text. Nothing here falls back to an earlier copy. */
import { CONTENT_REFS_FORMAT, MAX_CONTENT_REFS, contentRefsRepresentations, mayCarryContentRefs, type TransferReader, type TransferRepresentations } from 'buckyos/transfer'
import type { TransferableContentRef, ZoneContent } from 'buckyos/content'
import { desktopTransfers, dfsPathOfContent, isZoneEntry, withTransferHandle, zoneContentOfEntry, type TransferOffer, type TransferOperation, type TransferPurpose } from '../../../transfer'
import type { FileEntry } from '../types'
import { classifyFileKind } from './fileKinds'
import { operationError, type OperationResult } from './folderOps'
import { toUiError } from './state'

/** Files' own content behind the handle: the entries as the source listed them. */
export const FILES_ENTRIES_FORMAT = 'application/vnd.buckyos-files.entries+json'
/** Marks the `text/html` of a Files cut, so a paste after its record is gone (a reload, another tab) still knows it was
 * a cut and asks before copying (§6). It carries no path, id or credential. */

export type FilesRealm = 'zone' | 'computer'

export interface FilesTransferPayload {
  v: 1
  entries: FileEntry[]
  /** The window and pane the selection came from. */
  source: { windowId?: string; side: 'left' | 'right' }
  realm: FilesRealm
  /** Why the source refuses an operation it does not allow (the menu's reason). */
  refused: Partial<Record<TransferOperation, string>>
}

export const realmOfPath = (path: string): FilesRealm => path.startsWith('/') ? 'zone' : 'computer'

const escapeHtml = (s: string) => s.replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`)

/** Representations of a selection: the Zone reference list when every item is a Zone entry (and the list is not too
 * long), names otherwise; always the record's handle, marked as a cut for a cut. */
export function entriesRepresentations(entries: FileEntry[], offerId: string, cut: boolean): TransferRepresentations {
  const refs = entries.length <= MAX_CONTENT_REFS && entries.every(isZoneEntry)
  const names = entries.map((entry) => entry.name)
  const base = refs
    ? contentRefsRepresentations(entries.map(zoneContentOfEntry))
    : { 'text/plain': names.join('\n'), 'text/html': `<meta charset="utf-8">${names.map(escapeHtml).join('<br>')}` }
  return withTransferHandle(base, offerId, { cut })
}

/** Registers a selection with the desktop and returns what to write. */
export function offerEntries(purpose: TransferPurpose, entries: FileEntry[], source: FilesTransferPayload['source'], operations: TransferOperation[], refused: FilesTransferPayload['refused'] = {}) {
  const captured = entries.map((entry) => ({ ...entry }))
  const payload: FilesTransferPayload = { v: 1, entries: captured, source, realm: captured.every(isZoneEntry) ? 'zone' : 'computer', refused }
  // Zone items also go to the Shell's router one by one (its drag handle, "Paste desktop object…").
  const items = payload.realm === 'zone' ? captured.map((entry) => ({ kind: entry.kind === 'folder' ? 'zone-folder' as const : 'zone-file' as const, formats: { [CONTENT_REFS_FORMAT]: zoneContentOfEntry(entry) } })) : undefined
  const offer = desktopTransfers.offer({
    purpose, source: { app: 'files', windowId: source.windowId },
    itemKeys: captured.map((_, index) => String(index)), formats: { [FILES_ENTRIES_FORMAT]: payload }, operations,
    ...(items ? { items } : {}),
  })
  return { offer, representations: entriesRepresentations(captured, offer.offerId, purpose === 'cut') }
}

/** The `effectAllowed` of a drag that allows these operations. */
export function effectAllowedOf(operations: TransferOperation[]): DataTransfer['effectAllowed'] {
  const copy = operations.includes('copy'), move = operations.includes('move'), link = operations.includes('link')
  if (copy && move && link) return 'all'
  if (copy && move) return 'copyMove'
  if (copy && link) return 'copyLink'
  if (link && move) return 'linkMove'
  return copy ? 'copy' : move ? 'move' : link ? 'link' : 'none'
}

/** A drop effect the source allows: the preferred one, else copy, else none. */
export function allowedEffect(preferred: 'copy' | 'move' | 'link', effectAllowed: string): DataTransfer['dropEffect'] {
  const allowed = effectAllowed.toLowerCase()
  const ok = (effect: string) => allowed === 'all' || allowed === 'uninitialized' || allowed.includes(effect)
  return ok(preferred) ? preferred : ok('copy') ? 'copy' : 'none'
}

export const payloadOf = (offer: TransferOffer | undefined): FilesTransferPayload | undefined => {
  const payload = offer?.source.app === 'files' ? offer.formats[FILES_ENTRIES_FORMAT] as FilesTransferPayload | undefined : undefined
  return payload?.v === 1 ? payload : undefined
}

/** The entries of a record its items still name (a cut loses the moved ones), with their item keys. */
export function remainingEntries(offer: TransferOffer, payload: FilesTransferPayload): { key: string; entry: FileEntry }[] {
  return offer.itemKeys.filter((key) => offer.remaining.has(key)).map((key) => ({ key, entry: payload.entries[Number(key)] })).filter((item) => !!item.entry)
}

/** Ids of the entries a window's pending cut still marks. */
export function cutEntryIds(offer: TransferOffer | undefined): Set<string> {
  const payload = payloadOf(offer)
  return new Set(offer && payload ? remainingEntries(offer, payload).map((item) => item.entry.id) : [])
}

/** What a paste or drop carries, in the registered order (§2.3). */
export type Incoming =
  | { kind: 'files'; offer: TransferOffer; payload: FilesTransferPayload }
  | { kind: 'refs'; items: ZoneContent[]; staleCut: boolean }
  | { kind: 'local' }
  | { kind: 'addresses'; refs: TransferableContentRef[] }
  /** A Files handle whose record is gone, with nothing else Files can use (computer files after a reload). */
  | { kind: 'stale' }
  | { kind: 'text' }
  | { kind: 'empty' }

/** Reads a paste or drop synchronously (call it inside the event). A handle only counts when its record is alive. */
export function classifyIncoming(reader: TransferReader): Incoming {
  const handle = reader.handle()
  const offer = handle ? desktopTransfers.get(handle.offerId) : undefined
  const payload = payloadOf(offer)
  if (offer && payload) return { kind: 'files', offer, payload }
  const stale = !!handle && !offer
  const refs = reader.contentRefs()
  if (refs) return { kind: 'refs', items: refs.items, staleCut: stale && handle?.cut === true }
  if (reader.files.length) return { kind: 'local' }
  const addresses = reader.addresses()
  if (addresses) return { kind: 'addresses', refs: addresses }
  if (stale) return { kind: 'stale' }
  return reader.text?.trim() || reader.html ? { kind: 'text' } : { kind: 'empty' }
}

/** Whether a drag may be something Files takes, from its types alone (all `dragover` may look at). */
export const mayCarryFiles = (types: string[]): boolean =>
  types.includes('files') || types.includes(CONTENT_REFS_FORMAT) || mayCarryContentRefs(types)

const nameOfPath = (path: string) => path.split('/').filter(Boolean).pop() ?? path

/** A Zone reference as the entry a collection adds (its path is all a reference needs). */
export function entryOfContent(item: ZoneContent, path: string): FileEntry {
  const name = item.hints.name || nameOfPath(path)
  return { id: path, name, path, kind: item.kind === 'folder' ? 'folder' : classifyFileKind(name), modifiedAt: '', ...(item.hints.size !== undefined ? { sizeBytes: item.hints.size } : {}) }
}

export const contentOfAddress = (ref: TransferableContentRef): ZoneContent => ({ origin: 'zone', kind: 'file', source: ref, hints: {} })

const unsupportedItem = (item: ZoneContent): OperationResult => {
  const label = item.source.kind === 'cyfs-path' ? item.source.path : `obj://${item.source.objectId}`
  return { entry: { id: label, name: item.hints.name || nameOfPath(label), path: label, kind: 'other', modifiedAt: '' }, status: 'failed', error: operationError('UNSUPPORTED', 'This service does not support this operation') }
}

/** Zone references as the entries a collection adds; object references cannot be added. */
export function referenceEntries(items: ZoneContent[]): { entries: FileEntry[]; failed: OperationResult[] } {
  const entries: FileEntry[] = []
  const failed: OperationResult[] = []
  for (const item of items) {
    const path = dfsPathOfContent(item)
    if (path) entries.push(entryOfContent(item, path))
    else failed.push(unsupportedItem(item))
  }
  return { entries, failed }
}

/** Zone references as live entries to copy: each path is looked up with the user's own session (names, kinds and
 * sizes in a reference are only hints). Object references and paths that no longer exist fail. */
export async function resolveZoneItems(items: ZoneContent[], stat: (parent: string, name: string) => Promise<FileEntry | null>): Promise<{ entries: FileEntry[]; failed: OperationResult[] }> {
  const entries: FileEntry[] = []
  const failed: OperationResult[] = []
  const lookUp = async (item: ZoneContent) => {
    const path = dfsPathOfContent(item)
    if (!path || path === '/') return { item, path, entry: null, error: operationError('UNSUPPORTED', 'This service does not support this operation') }
    const parent = path.slice(0, path.lastIndexOf('/')) || '/'
    try { return { item, path, entry: await stat(parent, nameOfPath(path)), error: null } } catch (error) { return { item, path, entry: null, error } }
  }
  // A few lookups at a time: a selection may hold thousands of items.
  const looked: Awaited<ReturnType<typeof lookUp>>[] = []
  for (let i = 0; i < items.length; i += 16) looked.push(...await Promise.all(items.slice(i, i + 16).map(lookUp)))
  for (const { item, path, entry, error } of looked) {
    if (entry) { entries.push(entry); continue }
    if (!path) { failed.push(unsupportedItem(item)); continue }
    const placeholder = entryOfContent(item, path)
    failed.push({ entry: placeholder, status: 'failed', error: error ? toUiError(error) : operationError('NOT_FOUND', 'The requested item no longer exists') })
  }
  return { entries, failed }
}
