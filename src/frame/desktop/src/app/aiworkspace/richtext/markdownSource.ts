/* Markdown source of a rich text (流式页交互改进 §4): the "notepad" input mode edits plain Markdown text while
 * the document stays the collaborative rich text.
 *
 * The source is cut into segments, one per top-level block (or, after an edit, one per group of blocks the
 * edit produced), with the gaps between them kept verbatim:
 *
 *   text = gap₀ + seg₀ + gap₁ + seg₁ + … + segₙ₋₁ + gapₙ
 *
 * An edit (old text → new text) is reduced to the changed range (common prefix / suffix); only the segments
 * it touches are parsed again and replace their blocks in the editor, so every other block — its id, its
 * annotations, a colleague's concurrent edit — stays as it is, and the text the user typed is never rewritten
 * under the caret. Changes from elsewhere regenerate only the segments whose blocks changed (`rebase`).
 *
 * Conversion uses the core's CommonMark + GFM converter (the same one wish results go through). Object links
 * are written `[label](entity:<id>)`; an embedded Block is a line of its own, `![嵌入 · name](embed:<cell id>)`. */

import { Node as PMNode, type Schema } from 'prosemirror-model'
import type { EditorView } from 'prosemirror-view'
import { randomId } from '../api/ids'
import type { AiwsCore } from '../api/wasm'
import { aiwsT } from '../i18n'
import { newBlockId } from './blockId'

export interface MdSeg {
  text: string
  /** Top-level block ids this text stands for (in document order); empty for text that produced no block. */
  ids: string[]
  /** The blocks as they were when the text was taken (JSON), to notice a change from elsewhere. */
  snap: string
}

export interface MdSource { gaps: string[]; segs: MdSeg[] }

type JsonNode = { type: string; attrs?: Record<string, unknown>; content?: JsonNode[]; [key: string]: unknown }

export interface MdCodec {
  toMd: (node: PMNode) => string
  parse: (markdown: string) => JsonNode[]
}

const EMBED_LINE = /^!\[[^\]]*\]\(embed:([a-z0-9][a-z0-9_-]{0,63})\)$/

export function markdownCodec(core: AiwsCore, nameOf: (entityId: string) => string): MdCodec {
  return {
    toMd: (node) => {
      if (node.type.name === 'object_embed') {
        const id = String((node.attrs.ref as { entity_id?: string } | null)?.entity_id ?? '')
        return `![${aiwsT('aiworkspace.richtext.markdown.embed', { name: nameOf(id).replace(/[[\]]/g, '') })}](embed:${id})`
      }
      return core.richtext_to_markdown(JSON.stringify({ type: 'doc', content: [node.toJSON()] })).replace(/\s+$/, '')
    },
    parse: (markdown) => {
      const prefix = `m${randomId().slice(0, 8)}`
      let n = 0
      const out: JsonNode[] = []
      let chunk: string[] = []
      let fence = false
      const flush = () => {
        const text = chunk.join('\n')
        chunk = []
        if (!text.trim()) return
        const links: Record<string, { entity_id: string }> = {}
        for (const match of text.matchAll(/\]\((entity:[a-z0-9][a-z0-9_-]{0,63})\)/g)) links[match[1]] = { entity_id: match[1].slice(7) }
        n += 1
        const ast = JSON.parse(core.markdown_to_richtext(text, `${prefix}${n}`, JSON.stringify(links))) as JsonNode
        out.push(...(ast.content ?? []))
      }
      for (const line of markdown.split('\n')) {
        if (/^\s*(```|~~~)/.test(line)) fence = !fence
        const embed = fence ? null : EMBED_LINE.exec(line.trim())
        if (embed) {
          flush()
          n += 1
          out.push({ type: 'object_embed', attrs: { ref: { entity_id: embed[1] }, block_id: `${prefix}-e${n}` } })
        } else chunk.push(line)
      }
      flush()
      return out
    },
  }
}

const snapOf = (nodes: PMNode[]) => JSON.stringify(nodes.map((node) => node.toJSON()))
const idOf = (node: PMNode): string | null => (typeof node.attrs.block_id === 'string' ? node.attrs.block_id : null)

export function sourceText(source: MdSource): string {
  let out = source.gaps[0] ?? ''
  source.segs.forEach((seg, i) => { out += seg.text + (source.gaps[i + 1] ?? '') })
  return out
}

/** One segment per top-level block, separated by a blank line. */
export function buildSource(doc: PMNode, codec: MdCodec): MdSource {
  const segs: MdSeg[] = []
  doc.forEach((node) => {
    const id = idOf(node)
    segs.push({ text: codec.toMd(node), ids: id ? [id] : [], snap: snapOf([node]) })
  })
  const gaps = segs.length === 0 ? [''] : ['', ...segs.slice(1).map(() => '\n\n'), '']
  return { gaps, segs }
}

/** Top-level blocks by id, with position and index. */
function topLevel(doc: PMNode): Map<string, { node: PMNode; pos: number; index: number }> {
  const out = new Map<string, { node: PMNode; pos: number; index: number }>()
  doc.forEach((node, pos, index) => { const id = idOf(node); if (id && !out.has(id)) out.set(id, { node, pos, index }) })
  return out
}

/** New blocks keep the ids of the blocks they replace where they correspond: same content first, then same type
 * in order — so a paragraph that was only retyped keeps its annotations. Applied to children of kept containers. */
function keepIds(next: JsonNode[], old: PMNode[]) {
  const used = new Set<number>()
  const bare = (value: unknown): string => JSON.stringify(value, (k, v: unknown) => (k === 'block_id' ? undefined : v))
  const oldBare = old.map((node) => bare(node.toJSON()))
  const paired = new Map<number, number>()
  next.forEach((node, i) => {
    if (!node.attrs || !('block_id' in node.attrs)) return
    const same = oldBare.findIndex((text, j) => !used.has(j) && text === bare(node))
    if (same >= 0) { used.add(same); paired.set(i, same) }
  })
  let cursor = 0
  next.forEach((node, i) => {
    if (paired.has(i) || !node.attrs || !('block_id' in node.attrs)) return
    for (let j = cursor; j < old.length; j++) {
      if (!used.has(j) && old[j].type.name === node.type) { used.add(j); paired.set(i, j); cursor = j + 1; break }
    }
  })
  for (const [i, j] of paired) {
    const id = idOf(old[j])
    if (id) next[i].attrs = { ...next[i].attrs, block_id: id }
    const kids: PMNode[] = []
    old[j].forEach((child) => kids.push(child))
    if (next[i].content && kids.length > 0) keepIds(next[i].content, kids)
  }
}

function commonPrefix(a: string, b: string): number {
  const max = Math.min(a.length, b.length)
  let i = 0
  while (i < max && a.charCodeAt(i) === b.charCodeAt(i)) i++
  return i
}

function commonSuffix(a: string, b: string, limit: number): number {
  let i = 0
  while (i < limit && a.charCodeAt(a.length - 1 - i) === b.charCodeAt(b.length - 1 - i)) i++
  return i
}

/** The changed range of an edit: `[from, oldTo)` of the old text became `[from, newTo)` of the new one. */
export function changedRange(before: string, after: string): { from: number; oldTo: number; newTo: number } {
  const from = commonPrefix(before, after)
  const tail = commonSuffix(before, after, Math.min(before.length, after.length) - from)
  return { from, oldTo: before.length - tail, newTo: after.length - tail }
}

/** Apply the edit `base → next` of the source to the editor; returns the new source (whose text is `next`),
 * or null when the new text could not be turned into blocks (nothing was changed then). */
export function applyEdit(view: EditorView, source: MdSource, base: string, next: string, codec: MdCodec): MdSource | null {
  if (base === next) return source
  const schema: Schema = view.state.schema
  const { from: p, oldTo, newTo } = changedRange(base, next)
  const delta = next.length - base.length
  const starts: number[] = []
  const ends: number[] = []
  let at = 0
  source.segs.forEach((seg, i) => {
    at += source.gaps[i].length
    starts.push(at)
    at += seg.text.length
    ends.push(at)
  })
  let touched = source.segs.map((_, i) => i).filter((i) => starts[i] <= oldTo && ends[i] >= p)
  if (touched.length === 0 && source.segs.length > 0) {
    let before = -1
    source.segs.forEach((_, i) => { if (ends[i] < p) before = i })
    touched = [Math.max(0, before)]
  }
  const first = touched[0] ?? 0
  const last = touched[touched.length - 1] ?? -1
  const hasSegs = source.segs.length > 0
  const regionStart = hasSegs ? Math.min(starts[first], p) : 0
  const regionEndOld = hasSegs ? Math.max(ends[last], oldTo) : base.length
  const regionEndNew = regionEndOld + delta
  const regionText = next.slice(regionStart, regionEndNew)
  void newTo

  let parsed: JsonNode[]
  try { parsed = codec.parse(regionText) } catch { return null }

  // where the touched blocks are now (some may have been removed or moved by someone else meanwhile)
  const doc = view.state.doc
  const blocks = topLevel(doc)
  const oldIds = touched.flatMap((i) => source.segs[i]?.ids ?? [])
  const present = oldIds.map((id) => blocks.get(id)).filter((b): b is { node: PMNode; pos: number; index: number } => Boolean(b)).sort((a, b) => a.index - b.index)
  let from: number
  let to: number
  if (!hasSegs) { from = 0; to = doc.content.size }
  else if (present.length > 0) { from = present[0].pos; to = present[present.length - 1].pos + present[present.length - 1].node.nodeSize }
  else {
    // nothing of the touched text is in the document: insert after the nearest earlier segment still there
    from = 0
    for (let i = first - 1; i >= 0; i--) {
      const found = source.segs[i].ids.map((id) => blocks.get(id)).filter(Boolean).pop()
      if (found) { from = found.pos + found.node.nodeSize; break }
    }
    to = from
  }
  keepIds(parsed, present.map((b) => b.node))
  let nodes: PMNode[]
  try { nodes = parsed.map((json) => PMNode.fromJSON(schema, json)) } catch { return null }
  for (const node of nodes) { try { node.check() } catch { return null } }
  // the document keeps at least one block
  if (nodes.length === 0 && from === 0 && to === doc.content.size) nodes = [schema.nodes.paragraph.create({ block_id: newBlockId() })]
  const ids = nodes.map(idOf).filter((id): id is string => Boolean(id))
  if (from !== to || nodes.length > 0) view.dispatch(view.state.tr.replaceWith(from, to, nodes))

  // the blocks as the editor now has them (the id plugin may have renamed a clash)
  const after = topLevel(view.state.doc)
  const kept = ids.map((id) => after.get(id)).filter((b): b is { node: PMNode; pos: number; index: number } => Boolean(b))
  const seg: MdSeg = { text: regionText, ids: kept.map((b) => idOf(b.node) as string), snap: snapOf(kept.map((b) => b.node)) }
  if (!hasSegs) return { gaps: ['', ''], segs: [seg] }
  const gapStart = starts[first] - source.gaps[first].length
  const gapBefore = regionStart < starts[first] ? base.slice(gapStart, regionStart) : source.gaps[first]
  const gapEnd = ends[last] + source.gaps[last + 1].length
  const gapAfter = regionEndOld > ends[last] ? base.slice(regionEndOld, gapEnd) : source.gaps[last + 1]
  return {
    gaps: [...source.gaps.slice(0, first), gapBefore, gapAfter, ...source.gaps.slice(last + 2)],
    segs: [...source.segs.slice(0, first), seg, ...source.segs.slice(last + 1)],
  }
}

/** Bring the source up to date with the document after changes made elsewhere: segments whose blocks did not
 * change keep their text exactly; changed ones are regenerated; blocks the source does not know are added after
 * their predecessor. A reordering from elsewhere regenerates everything. */
export function rebase(doc: PMNode, source: MdSource, codec: MdCodec): MdSource {
  const blocks = topLevel(doc)
  const owner = new Map<string, number>()
  source.segs.forEach((seg, i) => seg.ids.forEach((id) => owner.set(id, i)))
  // the known blocks must still come in the order of their segments
  let lastSeg = -1
  let ordered = true
  doc.forEach((node) => {
    const i = owner.get(idOf(node) ?? '')
    if (i === undefined) return
    if (i < lastSeg) ordered = false
    lastSeg = i
  })
  if (!ordered) return buildSource(doc, codec)
  const segs = source.segs.map((seg): MdSeg => {
    const nodes = seg.ids.map((id) => blocks.get(id)).filter((b): b is { node: PMNode; pos: number; index: number } => Boolean(b)).sort((a, b) => a.index - b.index).map((b) => b.node)
    if (nodes.length === seg.ids.length && snapOf(nodes) === seg.snap) return seg
    if (nodes.length === 0) return { text: '', ids: [], snap: '' }
    return { text: nodes.map(codec.toMd).join('\n\n'), ids: nodes.map((node) => idOf(node) as string), snap: snapOf(nodes) }
  })
  const gaps = [...source.gaps]
  // unknown blocks (added elsewhere): a new segment after the segment of the block before them
  const inserts: { after: number; node: PMNode }[] = []
  let previous = -1
  doc.forEach((node) => {
    const id = idOf(node)
    const i = id ? owner.get(id) : undefined
    if (i !== undefined) { previous = i; return }
    inserts.push({ after: previous, node })
  })
  // from the last to the first, so earlier indices stay valid; same anchor → keep document order
  for (let k = inserts.length - 1; k >= 0; k--) {
    const { after, node } = inserts[k]
    const id = idOf(node)
    const seg: MdSeg = { text: codec.toMd(node), ids: id ? [id] : [], snap: snapOf([node]) }
    segs.splice(after + 1, 0, seg)
    gaps.splice(after + 1 === 0 ? 1 : after + 1, 0, '\n\n')
  }
  if (segs.length > 0 && gaps.length !== segs.length + 1) return buildSource(doc, codec)
  return { gaps, segs }
}

/** The top-level block index of a source offset (to place the editor's caret when leaving source mode). */
export function segmentAt(source: MdSource, offset: number): MdSeg | null {
  let at = 0
  for (let i = 0; i < source.segs.length; i++) {
    at += source.gaps[i].length
    const end = at + source.segs[i].text.length
    if (offset <= end + (source.gaps[i + 1]?.length ?? 0)) return source.segs[i]
    at = end
  }
  return source.segs[source.segs.length - 1] ?? null
}

/** The source offset at the end of the segment holding block `blockId` (entering source mode from the caret). */
export function offsetOfBlock(source: MdSource, blockId: string | null): number {
  let at = 0
  for (let i = 0; i < source.segs.length; i++) {
    at += source.gaps[i].length + source.segs[i].text.length
    if (blockId && source.segs[i].ids.includes(blockId)) return at
  }
  return at
}
