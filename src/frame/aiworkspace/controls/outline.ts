/* Markdown outline ⇄ mind map (脑图方案 §8, TODO M1/M3): the exchange format of Agents and wishes and the
 * "turn into a mind map / into an outline" actions of the Desktop. A nested list (and headings above it) becomes
 * a tree of text nodes whose placement is all "auto"; the tree reads back as a nested list. Node text keeps its
 * inline Markdown (bold, links): it is stored as the title and drawn formatted. */

import { keysAfter, randomId } from '../import-drawio/keys.ts'
import { ALL_AUTO, type StoredPlacement } from './placement.ts'
import { mindmapConfig, type MindmapConfig } from './tree.ts'

export interface OutlineNode { text: string; children: OutlineNode[] }

/** The core's limit on a title. */
export const MAX_TITLE_CHARS = 256

/** A Markdown outline: headings nest by level, list items by indentation under the heading (or loose line) they
 * follow; a loose line of text heads the list after it; indented continuation lines join their item. Code fences,
 * rules, tables and quotes are skipped. */
export function parseMarkdownOutline(markdown: string): OutlineNode[] {
  const roots: OutlineNode[] = []
  // the heading / loose-line context: level → node
  const heads: { level: number; node: OutlineNode }[] = []
  // list items of the current list: indentation → node
  let items: { indent: number; node: OutlineNode }[] = []
  let fence: string | null = null
  const attach = (node: OutlineNode, parent: OutlineNode | undefined) => (parent ? parent.children : roots).push(node)
  const headParent = (level: number) => {
    while (heads.length && heads[heads.length - 1].level >= level) heads.pop()
    return heads[heads.length - 1]?.node
  }
  for (const raw of markdown.replace(/\r\n?/g, '\n').split('\n')) {
    const line = raw.replace(/\t/g, '    ')
    const trimmed = line.trim()
    const f = /^(```|~~~)/.exec(trimmed)
    if (fence) { if (f && trimmed.startsWith(fence)) fence = null; continue }
    if (f) { fence = f[1]; continue }
    if (!trimmed || /^([-*_])(\s*\1){2,}$/.test(trimmed) || trimmed.startsWith('|') || trimmed.startsWith('>')) continue
    const heading = /^(#{1,6})\s+(.*?)\s*#*\s*$/.exec(trimmed)
    if (heading && line.startsWith('#')) {
      const level = heading[1].length
      const node = { text: clean(heading[2]), children: [] }
      attach(node, headParent(level))
      heads.push({ level, node })
      items = []
      continue
    }
    const item = /^(\s*)(?:[-*+]|\d{1,9}[.)])\s+(.*)$/.exec(line)
    if (item) {
      const indent = item[1].length
      const node = { text: clean(item[2].replace(/^\[[ xX]\]\s+/, '')), children: [] }
      while (items.length && items[items.length - 1].indent >= indent) items.pop()
      attach(node, items[items.length - 1]?.node ?? heads[heads.length - 1]?.node)
      items.push({ indent, node })
      continue
    }
    const indent = line.length - line.trimStart().length
    const last = items[items.length - 1]
    if (last && indent > last.indent) { last.node.text = clean(`${last.node.text} ${trimmed}`); continue }
    // a loose line heads what follows, one level below the current heading
    const level = (heads[heads.length - 1]?.level ?? 0) + 0.5
    const node = { text: clean(trimmed), children: [] }
    attach(node, headParent(level))
    heads.push({ level, node })
    items = []
  }
  return roots
}

function clean(text: string): string {
  return text.replace(/\s+/g, ' ').trim()
}

/** A nested list, two spaces per level. */
export function formatMarkdownOutline(nodes: OutlineNode[]): string {
  const out: string[] = []
  const walk = (n: OutlineNode, depth: number) => {
    out.push(`${'  '.repeat(depth)}- ${n.text.replace(/\n/g, ' ')}`)
    for (const c of n.children) walk(c, depth + 1)
  }
  for (const n of nodes) walk(n, 0)
  return out.join('\n') + (out.length ? '\n' : '')
}

/** A title the core accepts: at most 256 characters, cut with an ellipsis. */
export function nodeTitle(text: string): string {
  const chars = [...text]
  return chars.length <= MAX_TITLE_CHARS ? text : chars.slice(0, MAX_TITLE_CHARS - 1).join('') + '…'
}

export type Op = Record<string, unknown>

/** The operations creating `nodes` as text nodes under `parentId` (a mind map control or one of its nodes), after
 * the sibling key `after`. Placement is all "auto": the control lays them out. */
export function nodeOperations(nodes: OutlineNode[], parentId: string, after: string | null = null, newId: (prefix: string) => string = randomId): { ops: Op[]; ids: string[] } {
  const ops: Op[] = []
  const ids: string[] = []
  const walk = (list: OutlineNode[], parent: string, prev: string | null) => {
    const keys = keysAfter(prev, list.length)
    list.forEach((n, i) => {
      const id = newId('n')
      if (parent === parentId) ids.push(id)
      ops.push({ op: 'entity.create', entity_id: id, type_id: 'buckyos.cell', parent_id: parent, order_key: keys[i], placement: { ...ALL_AUTO },
        payload: { view: { type: 'mindnode' }, title: nodeTitle(n.text) } })
      walk(n.children, id, null)
    })
  }
  walk(nodes, parentId, after)
  return { ops, ids }
}

export interface MindmapCreate {
  /** Where the control goes (a Surface or a group) and its sibling key. */
  parentId: string
  orderKey: string
  /** Its origin on a free canvas (the first root's top-left); omitted on a flow page. */
  at?: { x: number; y: number }
  title?: string
  config?: Partial<MindmapConfig>
  containerId?: string
  /** Several top-level items get one root with this text (otherwise they are several roots). */
  rootText?: string
  newId?: (prefix: string) => string
}

/** A mind map control holding `nodes`: the container, then its nodes. */
export function mindmapOperations(nodes: OutlineNode[], args: MindmapCreate): { containerId: string; rootIds: string[]; ops: Op[] } {
  const newId = args.newId ?? randomId
  const containerId = args.containerId ?? newId('m')
  const roots = nodes.length > 1 && args.rootText ? [{ text: args.rootText, children: nodes }] : nodes
  const config = mindmapConfig(args.config ?? {})
  const placement: StoredPlacement | undefined = args.at ? { x: Math.round(args.at.x), y: Math.round(args.at.y), w: 'auto', h: 'auto' } : undefined
  const title = args.title ?? (roots.length === 1 ? roots[0].text : undefined)
  const container: Op = { op: 'entity.create', entity_id: containerId, type_id: 'buckyos.container', parent_id: args.parentId, order_key: args.orderKey,
    ...(placement ? { placement } : {}),
    payload: { kind: 'mindmap', layout: { mode: 'tree' }, ...(title ? { title: nodeTitle(title) } : {}), config } }
  const built = nodeOperations(roots, containerId, null, newId)
  return { containerId, rootIds: built.ids, ops: [container, ...built.ops] }
}

/** What reading a control back needs: the outline (an `OutlineModel`, an outline array…) by parent. */
export interface TreeReader {
  childrenOf(id: string): { entity_id: string; type_id: string; kind?: string | null; view_type?: string | null; title?: string | null }[]
}

/** A control's Blocks as an outline (all expanded): text nodes by their title, other Blocks by their title or view,
 * a nested control by its title with its own tree below. */
export function controlOutline(tree: TreeReader, containerId: string): OutlineNode[] {
  const read = (parent: string): OutlineNode[] => tree.childrenOf(parent).map((e) => {
    if (e.type_id === 'buckyos.container') {
      const nested = e.kind === 'mindmap'
      return { text: e.title || (nested ? 'mindmap' : e.kind ?? 'group'), children: nested ? read(e.entity_id) : [] }
    }
    return { text: e.title || `[${e.view_type ?? 'block'}]`, children: read(e.entity_id) }
  })
  return read(containerId)
}

/** An outline from an array of envelopes (the `doc.outline` result, a package…). */
export function readerOf(entities: { entity_id: string; parent_id?: string | null; order_key?: string | null; deleted?: boolean }[]): TreeReader {
  const by = new Map<string, typeof entities>()
  for (const e of entities) {
    if (e.deleted || !e.parent_id) continue
    const list = by.get(e.parent_id) ?? []
    list.push(e)
    by.set(e.parent_id, list)
  }
  for (const list of by.values()) list.sort((a, b) => ((a.order_key ?? '') < (b.order_key ?? '') ? -1 : (a.order_key ?? '') > (b.order_key ?? '') ? 1 : a.entity_id < b.entity_id ? -1 : 1))
  return { childrenOf: (id) => (by.get(id) ?? []) as ReturnType<TreeReader['childrenOf']> }
}
