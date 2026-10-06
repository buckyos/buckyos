/* Annotation anchors in the rich text editor (design §3.7).
 *
 *   capture   selection → the first adapter that claims it (applications first), else the block
 *             around the cursor
 *   show      every annotation at the deepest level that resolves now: its range (adapter), its
 *             block, a block found again by its quote, or the entity (listed, not placed)
 *
 * `richtext_text` is the built-in range: Loro cursors on both ends follow concurrent typing; a
 * block rebuilt by a block operation loses them and the range is found again by `context.quote`
 * — the same rules as the backend. Adapters return either a text range (an inline highlight) or a
 * node: the node decoration's spec carries the annotations, so an application's NodeView can draw
 * its own range inside a block of its type. */

import { Cursor, LoroList, LoroMap, LoroText, type LoroDoc } from 'loro-crdt'
import type { Node as PMNode } from 'prosemirror-model'
import { Plugin, PluginKey, type EditorState } from 'prosemirror-state'
import { Decoration, DecorationSet, type EditorView } from 'prosemirror-view'
import { base64ToBytes, bytesToBase64 } from '../api/ids'
import type { AnnotationRange, Json } from '../api/types'
import type { AnnotationMark } from '../state/hooks'
import { blockIdAt } from '../richtext/blockId'
import { ATOM_CHAR, BLOCK_SEPARATOR, MAX_QUOTE_CHARS, QUOTE_SIDE_CHARS, findBest, findUnique, head, tail } from './quote'
import { AnchorRegistry, type CapturedAnchor } from './registry'

export interface RichTextAnchorHost {
  entityId: string
  /** The editor document now. */
  doc: PMNode
  /** The working CRDT document bound to it. */
  loro: LoroDoc
  selection: { from: number; to: number }
  /** `block_id` → position of the block node. */
  blocks: ReadonlyMap<string, { pos: number; node: PMNode }>
}

export type RichTextHit =
  | { type: 'text'; from: number; to: number; relocated?: boolean }
  /** A whole node; `detail` reaches the node's view through the decoration spec. */
  | { type: 'node'; pos: number; detail?: Json; relocated?: boolean }

export const richTextAnchors = new AnchorRegistry<RichTextAnchorHost, RichTextHit>()

export function makeHost(entityId: string, doc: PMNode, loro: LoroDoc, selection: { from: number; to: number }): RichTextAnchorHost {
  const blocks = new Map<string, { pos: number; node: PMNode }>()
  doc.descendants((node, pos) => {
    if (typeof node.attrs.block_id === 'string') blocks.set(node.attrs.block_id, { pos, node })
    return !node.isTextblock
  })
  return { entityId, doc, loro, selection, blocks }
}

/** Anchor text of a textblock: text as is, every inline atom one U+FFFC. */
export function blockText(node: PMNode): string {
  return node.textBetween(0, node.content.size, undefined, ATOM_CHAR)
}

// ---- Loro ↔ positions

function blockIdOf(map: LoroMap): string | null {
  const attrs = map.get('attributes')
  const id = attrs instanceof LoroMap ? attrs.get('block_id') : undefined
  return typeof id === 'string' ? id : null
}

function loroBlock(map: LoroMap, blockId: string): LoroMap | null {
  const children = map.get('children')
  if (!(children instanceof LoroList)) return null
  for (let i = 0; i < children.length; i++) {
    const child = children.get(i)
    if (!(child instanceof LoroMap)) continue
    if (blockIdOf(child) === blockId) return child
    const found = loroBlock(child, blockId)
    if (found) return found
  }
  return null
}

/** Text runs of a textblock's CRDT node with their offsets in the block (atoms count one). */
function runs(block: LoroMap): { text: LoroText; from: number; to: number }[] {
  const children = block.get('children')
  const out: { text: LoroText; from: number; to: number }[] = []
  if (!(children instanceof LoroList)) return out
  let at = 0
  for (let i = 0; i < children.length; i++) {
    const child = children.get(i)
    if (child instanceof LoroText) {
      out.push({ text: child, from: at, to: at + child.length })
      at += child.length
    } else {
      at += 1
    }
  }
  return out
}

/** An encoded Loro cursor at `offset` inside a textblock: on the first selected char for a start, after the last for an end. */
function cursorAt(loro: LoroDoc, blockId: string, offset: number, edge: 'start' | 'end'): string | null {
  const block = loroBlock(loro.getMap('doc'), blockId)
  if (!block) return null
  const list = runs(block)
  const run = edge === 'start'
    ? list.find((r) => offset >= r.from && offset < r.to) ?? list.find((r) => offset === r.to)
    : list.find((r) => offset > r.from && offset <= r.to) ?? list.find((r) => offset === r.from)
  const cursor = run?.text.getCursor(offset - run.from)
  return cursor ? bytesToBase64(cursor.encode()) : null
}

/** Where an encoded cursor points in the editor now; null once its text left the document. */
function cursorPos(host: RichTextAnchorHost, encoded: Json | undefined): { pos: number; blockId: string } | null {
  if (typeof encoded !== 'string') return null
  let cursor: Cursor
  try { cursor = Cursor.decode(base64ToBytes(encoded)) } catch { return null }
  const text = host.loro.getContainerById(cursor.containerId())
  if (!(text instanceof LoroText) || text.isDeleted()) return null
  const block = text.parent()?.parent()
  if (!(block instanceof LoroMap)) return null
  const blockId = blockIdOf(block)
  const at = blockId ? host.blocks.get(blockId) : undefined
  const query = blockId && at ? host.loro.getCursorPos(cursor) : undefined
  const run = runs(block).find((r) => r.text.id === text.id)
  if (!blockId || !at || !query || !run) return null
  return { pos: Math.min(at.pos + 1 + run.from + Math.min(query.offset, run.to - run.from), at.pos + at.node.nodeSize - 1), blockId }
}

// ---- searching anchor text

interface Segment { flat: number; pos: number; length: number }

/** Anchor text of the textblocks from `fromBlock` to `toBlock` (whole document without them), with their positions. */
function flatText(doc: PMNode, fromPos = 0, toPos = doc.content.size): { text: string; segments: Segment[] } {
  let text = ''
  const segments: Segment[] = []
  doc.nodesBetween(fromPos, toPos, (node, pos) => {
    if (!node.isTextblock) return true
    if (segments.length > 0) text += BLOCK_SEPARATOR
    const t = blockText(node)
    segments.push({ flat: text.length, pos: pos + 1, length: t.length })
    text += t
    return false
  })
  return { text, segments }
}

function toPos(segments: Segment[], offset: number): number {
  let seg = segments[0]
  for (const s of segments) if (s.flat <= offset) seg = s
  return seg.pos + Math.min(offset - seg.flat, seg.length)
}

function search(doc: PMNode, quote: { exact: string; prefix?: string; suffix?: string }, unique: boolean, fromPos?: number, toPosition?: number): [number, number] | null {
  const { text, segments } = flatText(doc, fromPos, toPosition)
  if (segments.length === 0) return null
  const found = unique ? findUnique(text, quote) : findBest(text, quote)
  return found ? [toPos(segments, found[0]), toPos(segments, found[1])] : null
}

// ---- the built-in text range

interface TextEnd { block_id: string; cursor?: string }

richTextAnchors.register({
  kind: 'richtext_text',
  capture(host) {
    const { from, to } = host.selection
    if (from >= to) return null
    const $from = host.doc.resolve(from)
    const $to = host.doc.resolve(to)
    const startId = $from.parent.attrs.block_id
    const endId = $to.parent.attrs.block_id
    if (!$from.parent.isTextblock || !$to.parent.isTextblock || typeof startId !== 'string' || typeof endId !== 'string') return null
    const exact = host.doc.textBetween(from, to, BLOCK_SEPARATOR, ATOM_CHAR)
    if (exact.trim() === '') return null
    const end = (blockId: string, offset: number, edge: 'start' | 'end'): TextEnd => {
      const cursor = cursorAt(host.loro, blockId, offset, edge)
      return cursor ? { block_id: blockId, cursor } : { block_id: blockId }
    }
    const range = { kind: 'richtext_text', start: end(startId, $from.parentOffset, 'start'), end: end(endId, $to.parentOffset, 'end') } as unknown as AnnotationRange
    const quote = Array.from(exact).length > MAX_QUOTE_CHARS ? undefined : {
      exact,
      prefix: tail(blockText($from.parent).slice(0, $from.parentOffset), QUOTE_SIDE_CHARS),
      suffix: head(blockText($to.parent).slice($to.parentOffset), QUOTE_SIDE_CHARS),
    }
    const label = `「${head(exact.replace(/\s+/g, ' '), 24)}${Array.from(exact).length > 24 ? '…' : ''}」`
    return { target: { entity_id: host.entityId, selector: { kind: 'richtext_block', block_id: startId } }, range, context: { ...(quote ? { quote } : {}), label }, label }
  },
  locate(host, mark) {
    const range = mark.payload.range as unknown as { start?: TextEnd; end?: TextEnd } | undefined
    const blockId = mark.payload.target.selector?.block_id
    const start = cursorPos(host, range?.start?.cursor)
    const end = cursorPos(host, range?.end?.cursor)
    if (start && end && start.pos < end.pos && start.blockId === blockId) return { type: 'text', from: start.pos, to: end.pos }
    const quote = mark.payload.context?.quote
    if (!quote) return null
    const first = blockId ? host.blocks.get(blockId) : undefined
    if (first) {
      const last = range?.end?.block_id ? host.blocks.get(range.end.block_id) : undefined
      const stop = last && last.pos >= first.pos ? last.pos + last.node.nodeSize : first.pos + first.node.nodeSize
      const found = search(host.doc, quote, false, first.pos, stop)
      return found ? { type: 'text', from: found[0], to: found[1], relocated: true } : null
    }
    const found = search(host.doc, quote, true)
    return found ? { type: 'text', from: found[0], to: found[1], relocated: true } : null
  },
})

/** The anchor for the current selection: an adapter's range, else the block around the cursor. */
export function captureAnchor(host: RichTextAnchorHost): CapturedAnchor | null {
  for (const adapter of richTextAnchors.capturers()) {
    const captured = adapter.capture?.(host)
    if (captured) return captured
  }
  const blockId = blockIdAt(host.doc, host.selection.from)
  const at = blockId ? host.blocks.get(blockId) : undefined
  if (!blockId || !at) return null
  const text = at.node.textBetween(0, at.node.content.size, BLOCK_SEPARATOR, ATOM_CHAR)
  const exact = head(text, 200)
  const label = exact.trim() ? `块「${head(exact.replace(/\s+/g, ' '), 16)}${Array.from(exact).length > 16 ? '…' : ''}」` : `块 ${blockId}`
  return {
    target: { entity_id: host.entityId, selector: { kind: 'richtext_block', block_id: blockId } },
    context: exact.trim() ? { quote: { exact }, label } : { label },
    label,
  }
}

/** The selection, read from the DOM when it is there (a read-only view does not track it). */
export function selectionOf(view: EditorView): { from: number; to: number } {
  const dom = window.getSelection()
  if (dom && dom.rangeCount > 0 && !dom.isCollapsed && dom.anchorNode && dom.focusNode && view.dom.contains(dom.anchorNode) && view.dom.contains(dom.focusNode)) {
    try {
      const a = view.posAtDOM(dom.anchorNode, dom.anchorOffset)
      const b = view.posAtDOM(dom.focusNode, dom.focusOffset)
      return { from: Math.min(a, b), to: Math.max(a, b) }
    } catch { /* fall back to the editor selection */ }
  }
  return { from: view.state.selection.from, to: view.state.selection.to }
}

// ---- showing annotations

export interface Placement {
  id: string
  level: 'range' | 'target' | 'entity'
  relocated: boolean
  /** Document position used for ordering; -1 for annotations listed without a place. */
  pos: number
}

function locateMark(host: RichTextAnchorHost, mark: AnnotationMark): { hit: RichTextHit; level: 'range' | 'target' } | null {
  const range = mark.payload.range
  const adapter = range ? richTextAnchors.get(range.kind) : undefined
  if (adapter) {
    try {
      const hit = adapter.locate(host, mark)
      if (hit) return { hit, level: 'range' }
    } catch (error) { console.error('[aiworkspace] anchor adapter failed', range?.kind, error) }
  }
  const blockId = mark.payload.target.selector?.kind === 'richtext_block' ? mark.payload.target.selector.block_id : undefined
  const at = blockId ? host.blocks.get(blockId) : undefined
  if (at) return { hit: { type: 'node', pos: at.pos }, level: 'target' }
  // a block-level annotation whose block is gone: the block its quote is in now
  const quote = mark.payload.context?.quote
  if (blockId && !range && quote) {
    const found = search(host.doc, quote, true)
    if (found) {
      const $pos = host.doc.resolve(found[0])
      return { hit: { type: 'node', pos: $pos.before($pos.depth), relocated: true }, level: 'target' }
    }
  }
  return null
}

function decorate(host: RichTextAnchorHost, marks: AnnotationMark[], active: string | null): { decorations: DecorationSet; placements: Placement[] } {
  const placements: Placement[] = []
  const inline: Decoration[] = []
  const nodes = new Map<number, { ids: string[]; details: { id: string; detail?: Json }[] }>()
  for (const mark of marks) {
    const located = locateMark(host, mark)
    if (!located) {
      placements.push({ id: mark.entityId, level: 'entity', relocated: false, pos: -1 })
      continue
    }
    const { hit, level } = located
    placements.push({ id: mark.entityId, level, relocated: Boolean(hit.relocated), pos: hit.type === 'text' ? hit.from : hit.pos })
    if (hit.type === 'text') {
      inline.push(Decoration.inline(hit.from, hit.to, {
        class: `aiws-anno-range${mark.entityId === active ? ' is-active' : ''}`, [`data-anno-${mark.entityId}`]: '',
      }, { annotations: [{ id: mark.entityId }] }))
    } else {
      const entry = nodes.get(hit.pos) ?? { ids: [], details: [] }
      entry.ids.push(mark.entityId)
      entry.details.push({ id: mark.entityId, detail: hit.detail })
      nodes.set(hit.pos, entry)
    }
  }
  const decorations = [...inline]
  for (const [pos, entry] of nodes) {
    const node = host.doc.nodeAt(pos)
    if (!node) continue
    const attrs: Record<string, string> = { class: `aiws-anno-block${entry.ids.includes(active ?? '') ? ' is-active' : ''}` }
    for (const id of entry.ids) attrs[`data-anno-${id}`] = ''
    decorations.push(Decoration.node(pos, pos + node.nodeSize, attrs, { annotations: entry.details }))
  }
  placements.sort((a, b) => a.pos - b.pos)
  return { decorations: DecorationSet.create(host.doc, decorations), placements }
}

interface AnnotationsState {
  marks: AnnotationMark[]
  active: string | null
  decorations: DecorationSet
  placements: Placement[]
  /** The document changed since the last placement: mapped for now, placed again shortly. */
  stale: boolean
}

interface AnnotationsMeta { marks?: AnnotationMark[]; active?: string | null; recompute?: true }

export const annotationsKey = new PluginKey<AnnotationsState>('aiws-annotations')

/** Push the annotations of this rich text (and the active one) into the editor. */
export function setAnnotations(view: EditorView, marks: AnnotationMark[], active: string | null) {
  view.dispatch(view.state.tr.setMeta(annotationsKey, { marks, active } satisfies AnnotationsMeta))
}

/**
 * Placement runs on a consistent pair of documents only: right after `setAnnotations`, and a moment
 * after edits (the CRDT document follows the editor in the sync plugin's view update), never while
 * an input method is composing.
 */
export function annotationsPlugin({ entityId, loro, onPlaced, onClick }: {
  entityId: string
  loro: () => LoroDoc
  onPlaced: (placements: Placement[]) => void
  /** A click on annotated content (the caret still moves there). */
  onClick: (annotationId: string) => void
}): Plugin<AnnotationsState> {
  const place = (state: EditorState, marks: AnnotationMark[], active: string | null) =>
    decorate(makeHost(entityId, state.doc, loro(), { from: state.selection.from, to: state.selection.to }), marks, active)
  return new Plugin<AnnotationsState>({
    key: annotationsKey,
    state: {
      init: () => ({ marks: [], active: null, decorations: DecorationSet.empty, placements: [], stale: false }),
      apply(tr, value, _old, state) {
        const meta = tr.getMeta(annotationsKey) as AnnotationsMeta | undefined
        if (meta) {
          const marks = meta.marks ?? value.marks
          const active = meta.active === undefined ? value.active : meta.active
          return { marks, active, ...place(state, marks, active), stale: false }
        }
        if (!tr.docChanged) return value
        return { ...value, decorations: value.decorations.map(tr.mapping, tr.doc), stale: true }
      },
    },
    props: {
      decorations: (state) => annotationsKey.getState(state)?.decorations,
      handleClick(view, pos) {
        const found = annotationsKey.getState(view.state)?.decorations.find(pos, pos)
        const spec = found?.at(-1)?.spec as { annotations?: { id: string }[] } | undefined
        if (spec?.annotations?.[0]) onClick(spec.annotations[0].id)
        return false
      },
    },
    view(editorView) {
      let timer: number | null = null
      let reported: Placement[] | null = null
      // an adapter registered or removed later (an application loading) changes what can be placed
      const offRegistry = richTextAnchors.subscribe(() => {
        if (!editorView.isDestroyed) editorView.dispatch(editorView.state.tr.setMeta(annotationsKey, { recompute: true } satisfies AnnotationsMeta))
      })
      return {
        update(view) {
          const state = annotationsKey.getState(view.state)
          if (!state) return
          if (state.stale && timer === null) {
            timer = window.setTimeout(function again() {
              timer = null
              if (view.isDestroyed) return
              if (view.composing) { timer = window.setTimeout(again, 200); return }
              view.dispatch(view.state.tr.setMeta(annotationsKey, { recompute: true } satisfies AnnotationsMeta))
            }, 50)
          }
          if (state.placements !== reported) {
            reported = state.placements
            onPlaced(state.placements)
          }
        },
        destroy() {
          if (timer !== null) window.clearTimeout(timer)
          offRegistry()
        },
      }
    },
  })
}
