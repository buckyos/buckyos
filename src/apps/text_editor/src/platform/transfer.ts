/* Content to and from other Apps through the desktop Shell (`transfer.v1`, 剪贴板规范 §5): what this editor offers
 * (the selected text, else the open Zone file) and what it takes (text at the drop point or the cursor, Zone files
 * opened in new tabs). Only the decisions live here; the editor and the frame glue are in main.tsx. */
import type { TransferDelivery, TransferDeliveryResult, TransferOfferItem, TransferProbe, TransferProbeAnswer } from 'buckyos/app-frame'
import { CONTENT_REFS_FORMAT, contentRefsRepresentations } from 'buckyos/transfer'
import { contentDescriptor, readZoneContent, type TransferableContentRef, type ZoneContent } from 'buckyos/content'

/** The editor's own tab drag (reordering and moving tabs between groups): never put on `text/plain`. */
export const TAB_DRAG_FORMAT = 'application/vnd.buckyos-text-editor.tab+json'
export const tabDragValue = (doc: string, group: string): string => JSON.stringify({ v: 1, doc, group })
export function readTabDrag(value: string | undefined): { doc: string; group: string } | undefined {
  try {
    const v = JSON.parse(value ?? '') as { v?: unknown; doc?: unknown; group?: unknown }
    return v?.v === 1 && typeof v.doc === 'string' && typeof v.group === 'string' ? { doc: v.doc, group: v.group } : undefined
  } catch { return undefined }
}

/** Selections up to this size travel with the offer; a larger one is read when the Shell delivers it. */
const INLINE_LIMIT = 64 * 1024

export interface OfferedSelection {
  /** Changes whenever the offer would change. */
  key: string
  item: TransferOfferItem
  /** Every representation by format, for `transfer.read`. */
  values: Record<string, string>
}

/** What this editor offers: the selected text, else the open document when it is a Zone file; nothing otherwise. */
export function offeredSelection(input: { selection?: string; doc?: { id: string; source?: TransferableContentRef; name: string; preview?: unknown } }): OfferedSelection | undefined {
  if (input.selection) {
    const text = input.selection
    return { key: `text\n${text.length}\n${text}`, values: { 'text/plain': text },
      item: { kind: 'text', formats: ['text/plain'], operations: ['copy'], ...(text.length <= INLINE_LIMIT ? { inline: { 'text/plain': text } } : {}) } }
  }
  const doc = input.doc
  if (!doc || doc.preview || doc.source?.kind !== 'cyfs-path') return undefined
  const zone: ZoneContent = { origin: 'zone', kind: 'file', source: doc.source, hints: { name: doc.name, ...(contentDescriptor(doc.source, { name: doc.name }).mime ? { mime: contentDescriptor(doc.source, { name: doc.name }).mime } : {}) } }
  const values = contentRefsRepresentations([zone])
  const formats = [CONTENT_REFS_FORMAT, 'text/uri-list', 'text/plain']
  return { key: `file\n${doc.source.path}\n${doc.name}`, values: Object.fromEntries(formats.map(f => [f, values[f]])),
    item: { kind: 'zone-file', formats, operations: ['copy', 'link'], inline: Object.fromEntries(formats.map(f => [f, values[f]])) } }
}

/** Where a probe would land: insert text into a writable editor, or open Zone files. Null when it takes nothing. */
export function probeAnswer(probe: TransferProbe, target: { canInsert: boolean; phrases: { insert: string; open: string } }): TransferProbeAnswer | null {
  const text = target.canInsert ? probe.items.filter(item => item.kind !== 'zone-folder' && item.formats.includes('text/plain') && item.kind !== 'zone-file') : []
  if (text.length) return { targetId: 'cursor', action: target.phrases.insert, accept: text.map(item => ({ itemKey: item.itemKey, format: 'text/plain', operation: 'copy' })) }
  const files = probe.items.filter(item => item.kind === 'zone-file' && item.formats.includes(CONTENT_REFS_FORMAT))
  if (files.length) return { targetId: 'tabs', action: target.phrases.open, accept: files.map(item => ({ itemKey: item.itemKey, format: CONTENT_REFS_FORMAT, operation: item.operations.includes('link') ? 'link' : 'copy' })) }
  return null
}

/** Commits a delivery: `insert` puts the text in at the point (or the cursor), `open` opens each file in a tab. */
export async function deliverToEditor(delivery: TransferDelivery, actions: { insert: (text: string) => boolean; open: (source: TransferableContentRef) => Promise<void> }): Promise<TransferDeliveryResult> {
  if (delivery.targetId === 'cursor') {
    const text = delivery.items.map(item => typeof item.value === 'string' ? item.value : '').join('\n')
    const done = text !== '' && actions.insert(text)
    return { status: done ? 'done' : 'failed', items: delivery.items.map(item => ({ itemKey: item.itemKey, status: done ? 'done' : 'failed', ...(done ? {} : { error: 'read_only' }) })) }
  }
  if (delivery.targetId !== 'tabs') return { status: 'failed', items: delivery.items.map(item => ({ itemKey: item.itemKey, status: 'failed', error: 'unknown_target' })) }
  const items: TransferDeliveryResult['items'] = []
  for (const item of delivery.items) {
    const zone = readZoneContent(item.value)
    if (!zone || zone.kind !== 'file') { items.push({ itemKey: item.itemKey, status: 'skipped', error: 'not_a_file' }); continue }
    try { await actions.open(zone.source); items.push({ itemKey: item.itemKey, status: 'done' }) }
    catch (error) { items.push({ itemKey: item.itemKey, status: 'failed', error: error instanceof Error ? error.message.slice(0, 120) : 'open_failed' }) }
  }
  return { status: items.some(i => i.status === 'done') ? 'done' : 'failed', items }
}
