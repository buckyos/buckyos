/* The rich text AST the docx importer writes (Office 导入 TODO §3, O0): `buckyos.richtext.basic.v1`
 * (`../schemas/richtext.basic.v1.json`, checked by the core in `core/src/richtext.rs`). Headings go to level 3;
 * table cells hold inline content only; a list item starts with exactly one paragraph; `object_embed` may only point
 * at a Block (a Cell), so pictures are not embedded (§12). `validateDoc` mirrors the core's checks that an importer
 * can get wrong — the node set, content rules, block ids, nesting depth, the block and text limits — so Deno tests
 * and the Agent host catch them without the WASM core; the Desktop validates again through WASM before committing. */

export type Mark = { type: 'strong' | 'em' | 'strike' | 'code' } | { type: 'link'; attrs: { href: string } }
export interface AstNode {
  type: string
  attrs?: Record<string, string | number | Record<string, string>>
  content?: AstNode[]
  text?: string
  marks?: Mark[]
}

/** `limits` of the schema (core `Limits::default`); `richtext_test.ts` compares them with the JSON. */
export const LIMITS = { max_blocks: 5000, max_text_utf16: 500000, max_depth: 8 } as const
export const MAX_HEADING = 3

const BLOCK_ID = new Set(['paragraph', 'heading', 'bullet_list', 'ordered_list', 'list_item', 'blockquote', 'code_block', 'horizontal_rule', 'table', 'object_embed'])
const BLOCK_GROUP = new Set(['paragraph', 'heading', 'bullet_list', 'ordered_list', 'blockquote', 'code_block', 'horizontal_rule', 'table', 'object_embed'])
const INLINE = new Set(['text', 'hard_break', 'object_link'])
const HREF = /^(https?:|mailto:|#)/

export const text = (value: string, marks?: Mark[]): AstNode => (marks && marks.length ? { type: 'text', text: value, marks } : { type: 'text', text: value })

/** Inline nodes tidied: empty text dropped, neighbours with the same marks merged. */
export function tidyInline(nodes: AstNode[]): AstNode[] {
  const out: AstNode[] = []
  const key = (n: AstNode) => JSON.stringify(n.marks ?? [])
  for (const n of nodes) {
    if (n.type === 'text') {
      if (!n.text) continue
      const last = out[out.length - 1]
      if (last && last.type === 'text' && key(last) === key(n)) { last.text += n.text; continue }
      out.push({ ...n, ...(n.marks ? { marks: n.marks.slice() } : {}) })
    } else out.push(n)
  }
  // a hard break at the very end shows as nothing in a paragraph
  while (out.length && out[out.length - 1].type === 'hard_break') out.pop()
  return out
}

/** The plain text of inline content (a hard break as a newline). */
export function inlineText(nodes: AstNode[]): string {
  return nodes.map((n) => (n.type === 'text' ? n.text ?? '' : n.type === 'hard_break' ? '\n' : '')).join('')
}

/** Number of nodes carrying a block id, and UTF-16 length of the text. */
export function measure(node: AstNode): { blocks: number; text: number } {
  let blocks = BLOCK_ID.has(node.type) ? 1 : 0
  let length = node.type === 'text' ? (node.text ?? '').length : 0
  for (const c of node.content ?? []) { const m = measure(c); blocks += m.blocks; length += m.text }
  return { blocks, text: length }
}

/** The deepest nesting below `node` (a text node in a paragraph of the doc is at depth 2). */
export function depthOf(node: AstNode, at = 0): number {
  let max = at
  for (const c of node.content ?? []) max = Math.max(max, depthOf(c, at + 1))
  return max
}

function allowed(parent: string, children: AstNode[], index: number): boolean {
  const t = children[index].type
  switch (parent) {
    case 'doc': return BLOCK_GROUP.has(t)
    case 'paragraph': case 'heading': case 'table_cell': return INLINE.has(t)
    case 'bullet_list': case 'ordered_list': return t === 'list_item'
    case 'list_item': return index === 0 ? t === 'paragraph' : t === 'paragraph' || t === 'bullet_list' || t === 'ordered_list'
    case 'blockquote': return ['paragraph', 'heading', 'bullet_list', 'ordered_list', 'code_block'].includes(t)
    case 'code_block': return t === 'text'
    case 'table': return t === 'table_row'
    case 'table_row': return t === 'table_cell'
  }
  return false
}

const MIN_ONE = new Set(['doc', 'bullet_list', 'ordered_list', 'list_item', 'blockquote', 'table', 'table_row'])

/** Problems the core would refuse the document for (empty when none). */
export function validateDoc(doc: AstNode): string[] {
  const problems: string[] = []
  const ids = new Set<string>()
  let blocks = 0
  let length = 0
  const walk = (node: AstNode, depth: number, path: string) => {
    if (depth > LIMITS.max_depth) { problems.push(`${path}: nesting deeper than ${LIMITS.max_depth}`); return }
    if (BLOCK_ID.has(node.type)) {
      const id = node.attrs?.block_id
      if (typeof id !== 'string' || !/^[a-z0-9][a-z0-9_-]{0,63}$/.test(id)) problems.push(`${path}: bad block_id`)
      else if (ids.has(id)) problems.push(`${path}: duplicate block_id ${id}`)
      else ids.add(id)
      blocks += 1
    }
    if (node.type === 'text') {
      if (typeof node.text !== 'string' || node.text === '') problems.push(`${path}: empty text`)
      length += (node.text ?? '').length
      const names = new Set<string>()
      for (const m of node.marks ?? []) {
        if (names.has(m.type)) problems.push(`${path}: duplicate mark ${m.type}`)
        names.add(m.type)
        if (m.type === 'link' && !HREF.test(m.attrs.href)) problems.push(`${path}: link scheme`)
      }
      if (names.has('code') && names.size > 1) problems.push(`${path}: code excludes other marks`)
    }
    if (node.type === 'heading') {
      const level = node.attrs?.level ?? 1
      if (typeof level !== 'number' || level < 1 || level > MAX_HEADING) problems.push(`${path}: heading level ${String(level)}`)
    }
    if (node.type === 'ordered_list' && node.attrs?.start !== undefined && !(typeof node.attrs.start === 'number' && node.attrs.start >= 1)) problems.push(`${path}: list start`)
    const content = node.content ?? []
    if (MIN_ONE.has(node.type) && content.length === 0) problems.push(`${path}: ${node.type} needs content`)
    if (node.type === 'list_item' && content[0]?.type !== 'paragraph') problems.push(`${path}: list item must start with a paragraph`)
    content.forEach((c, i) => {
      if (!allowed(node.type, content, i)) problems.push(`${path}/${i}: ${c.type} not allowed in ${node.type}`)
      walk(c, depth + 1, `${path}/${i}`)
    })
  }
  if (doc.type !== 'doc') problems.push('top node must be doc')
  walk(doc, 0, 'doc')
  if (blocks > LIMITS.max_blocks) problems.push(`more than ${LIMITS.max_blocks} blocks`)
  if (length > LIMITS.max_text_utf16) problems.push('text too long')
  return problems
}
