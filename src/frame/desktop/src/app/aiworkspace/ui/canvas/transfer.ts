/* The canvas selection as a transfer (剪贴板规范 §2, §2.3, §3.4): what a copy, a cut or the Shell's delivery puts down
 * and what a paste looks for.
 *
 *   private     the objects themselves (`application/vnd.buckyos-aiws.clip+json`) stay in a desktop transfer record,
 *               scoped to the source Workspace; the system clipboard carries only its handle
 *   standard    a Zone reference list for the resource Blocks (their FileObject), so Files and chat take the files
 *   for people  `text/plain`: the selected Blocks' text or titles; `text/html` the same with the files as links (it
 *               also carries the handle where custom formats do not travel)
 *
 * Nothing internal (ids, JSON, markers) goes into `text/plain`. */

import { createContext } from 'react'
import { plainText as plainInline } from '@controls/inline'
import type { ZoneContent } from 'buckyos/content'
import { buckyosAddressUri, CONTENT_REFS_FORMAT, contentRefsHtml, contentRefsValue, type TransferRepresentations } from 'buckyos/transfer'
import type { AssetPayload } from '../../api/types'
import type { WorkspaceStore } from '../../state/store'
import { blockRegistry } from '../blocks/registry'
import { controlRegistry } from '../controls/registry'
import { randomId } from '../../api/ids'

/** The AI Workspace's object clip (registered in 剪贴板规范 §2.1). */
export const CLIP_FORMAT = 'application/vnd.buckyos-aiws.clip+json'
/** The App name transfer records carry. */
export const AIWS_APP = 'aiworkspace'

/** The desktop window this app runs in (absent on a standalone route): transfer records and the Shell's targets are
 * per window. */
export const TransferWindowContext = createContext<string | undefined>(undefined)

/** Cut records of this document that were moved or cancelled: a paste still carrying their handle is told so instead
 * of falling to the clipboard's text. */
const spent = new Set<string>()
export const spendCut = (offerId: string) => { spent.add(offerId); if (spent.size > 64) spent.delete(spent.values().next().value as string) }
export const isSpentCut = (offerId: string) => spent.has(offerId)

const escapeHtml = (s: string) => s.replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!)

/** One selected Block as people read it, and its file when it is a resource Block. */
export interface SelectionItem { text: string; ref?: ZoneContent }

/** A resource Block's FileObject as a Zone reference (its name, type and size as hints). */
export function assetRef(asset: AssetPayload, name?: string): ZoneContent {
  const fileName = asset.file_name ?? name
  return {
    origin: 'zone', kind: 'file', source: { kind: 'object-id', objectId: asset.object_id }, objId: asset.object_id,
    hints: { ...(fileName && !/[/\\\0]/.test(fileName) ? { name: fileName } : {}), ...(asset.media_type ? { mime: asset.media_type } : {}), ...(typeof asset.size === 'number' ? { size: asset.size } : {}) },
  }
}

/**
 * The selected Blocks (top level, in order) as people read them, from what is known now — a copy event writes in its
 * dispatch: a text or note Block's words as the canvas shows them, a shape's or node's label, a resource Block's file
 * name (with its reference when its data was read, `assets`), a group's or control's title and contents, else the
 * Block's title or its type.
 */
export function selectionItems(store: WorkspaceStore, ids: string[], root: ParentNode, assets: ReadonlyMap<string, AssetPayload>): SelectionItem[] {
  const shown = (id: string) => (root.querySelector<HTMLElement>(`.aiws-frame-block[data-block-id="${CSS.escape(id)}"], [data-flow-item][data-cell-id="${CSS.escape(id)}"]`)?.innerText ?? '').replace(/\n{3,}/g, '\n\n').trim()
  const textOf = (id: string, depth: number): string => {
    const entity = store.outline.get(id)
    if (!entity) return ''
    const title = entity.title ? plainInline(entity.title) : ''
    if (entity.type_id === 'buckyos.container') {
      const inner = depth < 8 ? store.outline.childrenOf(id).map((child) => textOf(child.entity_id, depth + 1)).filter(Boolean) : []
      return [title, ...inner].filter(Boolean).join('\n') || controlRegistry.get(entity.kind)?.title || ''
    }
    const source = entity.source_id ? store.outline.get(entity.source_id) : undefined
    const view = entity.view_type ?? ''
    if (view === 'richtext' || view === 'note') return shown(id) || title || source?.title || ''
    if (view === 'asset') return assets.get(entity.source_id ?? '')?.file_name ?? source?.name ?? title
    return title || source?.title || source?.name || (view === 'connector' ? '' : blockRegistry.get(view)?.title ?? '')
  }
  const items = ids.map((id) => {
    const entity = store.outline.get(id)
    const asset = entity?.view_type === 'asset' ? assets.get(entity.source_id ?? '') : undefined
    return { text: textOf(id, 0), ...(asset ? { ref: assetRef(asset, store.outline.get(entity?.source_id ?? '')?.name ?? undefined) } : {}) }
  })
  // nothing to read (lines without labels): their types, so the clipboard still holds something for people
  if (items.every((item) => !item.text)) for (const [i, item] of items.entries()) item.text = blockRegistry.get(store.outline.get(ids[i])?.view_type ?? '')?.title ?? ''
  return items
}

/** Everything one write of the selection puts down besides the private clip: the text, the HTML (files as marked
 * links), and the reference list of its files (`text/uri-list` too when every item is a file). */
export function selectionRepresentations(items: SelectionItem[]): TransferRepresentations {
  const words = items.filter((item) => item.text)
  const refs = items.flatMap((item) => (item.ref ? [item.ref] : []))
  const html = items.map((item) => (item.ref ? `<p>${contentRefsHtml([item.ref]).replace(/^<meta charset="utf-8">/, '')}</p>` : item.text ? `<p>${escapeHtml(item.text).replace(/\n/g, '<br>')}</p>` : '')).join('')
  const out: TransferRepresentations = { 'text/plain': words.map((item) => item.text).join('\n\n'), 'text/html': `<meta charset="utf-8">${html}` }
  if (refs.length) {
    out[CONTENT_REFS_FORMAT] = contentRefsValue(refs)
    if (refs.length === items.length) out['text/uri-list'] = refs.map((ref) => buckyosAddressUri(ref.source)).join('\r\n')
  }
  return out
}

/** The richtext schema's limits (`richtext.basic.v1.json`): a longer text is not made into one text Block. */
const MAX_PARAGRAPHS = 5000
const MAX_TEXT = 500_000
const URL_IN_TEXT = /https?:\/\/[^\s<>"]+[^\s<>".,;:!?)\]'’”]/g

/** Pasted text as a rich text document: a paragraph per line, web addresses as links; null when it is too long. */
export function textDocument(text: string): Record<string, unknown> | null {
  const lines = text.replace(/\r\n?/g, '\n').replace(/\n+$/, '').split('\n')
  if (lines.length > MAX_PARAGRAPHS || text.length > MAX_TEXT) return null
  const inline = (line: string) => {
    const out: Record<string, unknown>[] = []
    let at = 0
    for (const match of line.matchAll(URL_IN_TEXT)) {
      const start = match.index ?? 0
      if (start > at) out.push({ type: 'text', text: line.slice(at, start) })
      out.push({ type: 'text', text: match[0], marks: [{ type: 'link', attrs: { href: match[0] } }] })
      at = start + match[0].length
    }
    if (at < line.length) out.push({ type: 'text', text: line.slice(at) })
    return out
  }
  return { type: 'doc', content: lines.map((line) => ({ type: 'paragraph', attrs: { block_id: randomId('b') }, content: inline(line) })) }
}

/**
 * A toolbar or menu copy whose representations are still being made (a table's rows being fetched): the clipboard
 * item is written at once, inside the user's click (Safari asks for that), with the data to follow. The Web standard
 * formats always; the others where the browser writes Web custom formats.
 */
export async function copyDeferredWithCommand(formats: string[], representations: Promise<TransferRepresentations>): Promise<{ ok: boolean }> {
  const clipboard = globalThis.navigator?.clipboard
  const Item = (globalThis as { ClipboardItem?: (new (items: Record<string, Promise<Blob>>) => ClipboardItem) & { supports?: (type: string) => boolean } }).ClipboardItem
  if (!clipboard?.write || !Item) {
    try { await clipboard?.writeText((await representations)['text/plain'] ?? ''); return { ok: Boolean(clipboard) } } catch { return { ok: false } }
  }
  const supports = (type: string) => { try { return Item.supports?.(type) ?? false } catch { return false } }
  const blob = (format: string) => representations.then((r) => new Blob([r[format] ?? ''], { type: format }))
  const standard: string[] = formats.filter((format) => format === 'text/plain' || format === 'text/html')
  const custom = formats.filter((format) => !standard.includes(format) && supports(`web ${format}`))
  const item = (withCustom: boolean) => new Item(Object.fromEntries([...standard.map((f) => [f, blob(f)]), ...(withCustom ? custom.map((f) => [`web ${f}`, blob(f)]) : [])]))
  try {
    await clipboard.write([item(custom.length > 0)])
    return { ok: true }
  } catch {
    if (!custom.length) return { ok: false }
    try { await clipboard.write([item(false)]); return { ok: true } } catch { return { ok: false } }
  }
}
