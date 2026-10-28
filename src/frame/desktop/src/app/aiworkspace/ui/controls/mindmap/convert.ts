/* A mind map and its outline (脑图方案 §8, §9; TODO M3/M4): the same shared conversion both ways. A rich text's lists
 * become a mind map beside it; a mind map becomes a rich text outline beside it, or a Markdown / OPML file; an OPML or
 * Markdown outline file dropped on a canvas becomes a mind map. The map's structure is the outline; the reader's
 * folds play no part (everything is expanded). */

import { formatOpml, parseOpml } from '@controls/opml'
import { controlOutline, formatMarkdownOutline, mindmapOperations, parseMarkdownOutline, type OutlineNode } from '@controls/outline'
import { mindmapConfig } from '@controls/tree'
import { randomId } from '../../../api/ids'
import type { EntityEnvelope, Json, Operation } from '../../../api/types'
import { aiwsT } from '../../../i18n'
import type { WorkspaceStore } from '../../../state/store'
import type { Laid } from '../../canvas/layout'

const count = (nodes: OutlineNode[]): number => nodes.reduce((n, c) => n + 1 + count(c.children), 0)

/** The Markdown outline of a control (two spaces per level). */
export function mindmapMarkdown(store: WorkspaceStore, containerId: string): string {
  return formatMarkdownOutline(controlOutline(store.outline, containerId))
}

export function mindmapOpml(store: WorkspaceStore, containerId: string): string {
  const container = store.outline.get(containerId)
  return formatOpml(controlOutline(store.outline, containerId), container?.title ?? '')
}

/** Where something placed beside `l` goes: its parent (never inside a control) and a world spot to its right. */
function beside(store: WorkspaceStore, laid: Map<string, Laid> | null, entity: EntityEnvelope, surfaceId: string): { parentId: string; orderKey: string; at?: { x: number; y: number } } {
  const l = laid?.get(entity.entity_id)
  // a Block inside a control puts what it makes on the canvas
  const parentId = l?.node ? surfaceId : entity.parent_id ?? surfaceId
  const siblings = store.outline.childrenOf(parentId)
  const i = siblings.findIndex((s) => s.entity_id === entity.entity_id)
  const orderKey = store.core.order_key_between(siblings[i]?.order_key ?? siblings.at(-1)?.order_key ?? undefined, i >= 0 ? siblings[i + 1]?.order_key ?? undefined : undefined)
  if (!l) return { parentId, orderKey }
  const parent = laid?.get(parentId)
  const origin = parent ? parent.origin ?? { x: parent.rect.x, y: parent.rect.y } : { x: 0, y: 0 }
  return { parentId, orderKey, at: { x: Math.round(l.rect.x + l.rect.w + 80 - origin.x), y: Math.round(l.rect.y - origin.y) } }
}

/** The lists of a rich text Block as a mind map beside it (one commit); the id of the new control, or null. */
export async function richtextToMindmap(store: WorkspaceStore, laid: Map<string, Laid> | null, cell: EntityEnvelope, surfaceId: string): Promise<string | null> {
  const source = cell.source_id ? store.outline.get(cell.source_id) : undefined
  if (!source) return null
  const read = await store.session.read<{ payload?: { content?: unknown }; content?: unknown }>(source.entity_id)
  const ast = read.content.payload?.content ?? read.content.content
  const nodes = parseMarkdownOutline(store.core.richtext_to_markdown(JSON.stringify(ast)))
  if (count(nodes) < 2) { store.notify('info', aiwsT('aiworkspace.mindmap.fromListNone')); return null }
  const place = beside(store, laid, cell, surfaceId)
  const title = cell.title ?? source.title ?? undefined
  const built = mindmapOperations(nodes, { ...place, ...(title ? { title, rootText: title } : {}), config: { direction: 'right' } })
  const outcome = await store.submit({ editId: `insert:${built.containerId}`, label: aiwsT('aiworkspace.mindmap.fromListLabel'), operations: built.ops as Operation[] })
  return outcome.status === 'accepted' || outcome.status === 'saved_locally' ? built.containerId : null
}

/** A mind map as a rich text outline beside it (its own data in the canvas's content folder). */
export async function mindmapToRichtext(store: WorkspaceStore, laid: Map<string, Laid> | null, container: EntityEnvelope, surface: EntityEnvelope): Promise<string | null> {
  const markdown = mindmapMarkdown(store, container.entity_id)
  const folder = surface.content_folder_id ?? ''
  const dataId = randomId('d')
  const cellId = randomId('c')
  const content = JSON.parse(store.core.markdown_to_richtext(markdown, `m${randomId().slice(0, 8)}`, '{}')) as Json
  const place = beside(store, laid, container, surface.entity_id)
  const l = laid?.get(container.entity_id)
  const ops: Operation[] = [
    { op: 'entity.create', entity_id: dataId, type_id: 'buckyos.richtext', parent_id: folder, order_key: store.core.order_key_between(store.outline.childrenOf(folder).at(-1)?.order_key ?? undefined, undefined), ...(container.title ? { name: container.title.slice(0, 120) } : {}), payload: { content } },
    { op: 'entity.create', entity_id: cellId, type_id: 'buckyos.cell', parent_id: place.parentId, order_key: place.orderKey,
      ...(place.at ? { placement: { x: place.at.x, y: place.at.y, w: 420, h: Math.max(160, Math.min(800, Math.round(l?.rect.h ?? 320))) } } : {}),
      payload: { view: { type: 'richtext' }, source_ref: { entity_id: dataId }, ...(container.title ? { title: container.title } : {}) } },
  ]
  const outcome = await store.submit({ editId: `insert:${cellId}`, label: aiwsT('aiworkspace.mindmap.toOutlineLabel'), operations: ops })
  return outcome.status === 'accepted' || outcome.status === 'saved_locally' ? cellId : null
}

/** Does this file read as an outline (OPML, or a Markdown list)? */
export function outlineFileKind(name: string, text: string): 'opml' | 'markdown' | null {
  const lower = name.toLowerCase()
  if (lower.endsWith('.opml') || (lower.endsWith('.xml') && /<opml[\s>]/.test(text.slice(0, 2000)))) return 'opml'
  if ((lower.endsWith('.md') || lower.endsWith('.markdown')) && count(parseMarkdownOutline(text)) >= 2) return 'markdown'
  return null
}

/** An OPML or Markdown outline file as a new mind map at `at` (world, on a free canvas) under `parentId`. */
export async function importOutline(store: WorkspaceStore, file: { name: string; text: string }, kind: 'opml' | 'markdown', parentId: string, at: { x: number; y: number } | null): Promise<string | null> {
  let nodes: OutlineNode[]
  let title: string | null = null
  try {
    if (kind === 'opml') ({ nodes, title } = parseOpml(file.text))
    else nodes = parseMarkdownOutline(file.text)
  } catch (error) {
    store.notify('error', aiwsT('aiworkspace.mindmap.importFailed', { reason: error instanceof Error ? error.message : String(error) }))
    return null
  }
  if (nodes.length === 0) { store.notify('error', aiwsT('aiworkspace.mindmap.importFailed', { reason: aiwsT('aiworkspace.mindmap.fromListNone') })); return null }
  const name = title || file.name.replace(/\.(opml|xml|md|markdown)$/i, '')
  const built = mindmapOperations(nodes, { parentId, orderKey: store.core.order_key_between(store.outline.childrenOf(parentId).at(-1)?.order_key ?? undefined, undefined),
    ...(at ? { at } : {}), title: name, rootText: name, config: mindmapConfig({}) })
  const outcome = await store.submit({ editId: `insert:${built.containerId}`, label: aiwsT('aiworkspace.mindmap.importLabel'), operations: built.ops as Operation[] })
  return outcome.status === 'accepted' || outcome.status === 'saved_locally' ? built.containerId : null
}
