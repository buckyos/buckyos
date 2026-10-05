/* ProseMirror schema generated from the shared rich text schema definition
 * (`src/frame/aiworkspace/schemas/richtext.basic.v1.json`, also `richtext_schema()` of the WASM core).
 * The structure (content model, groups, attrs, defaults, mark rules) comes from that JSON only;
 * this file adds what the JSON deliberately does not contain: how nodes look in the DOM. */

import { Schema, type AttributeSpec, type DOMOutputSpec, type MarkSpec, type Node as PMNode, type NodeSpec, type TagParseRule } from 'prosemirror-model'

export interface ContentItem { of: string[]; min: number; max?: number }
export interface AttrDef { type: 'integer' | 'string' | 'reference' | 'href'; required?: boolean; default?: unknown; min?: number; max?: number; schemes?: string[]; target_types?: string[] }
export interface NodeDef { group?: string; block_id?: boolean; atom?: boolean; text?: boolean; attrs?: Record<string, AttrDef>; content?: ContentItem[] }
export interface MarkDef { inclusive?: boolean; exclusive?: boolean; attrs?: Record<string, AttrDef> }
export interface RichTextSchemaDef {
  id: string
  encoding: string
  top: string
  limits: { max_blocks: number; max_text_utf16: number; max_depth: number; max_update_bytes: number }
  nodes: Record<string, NodeDef>
  marks: Record<string, MarkDef>
}

export const BLOCK_ID_PATTERN = /^[a-z0-9][a-z0-9_-]{0,63}$/

/** `[{ of: ['@inline'], min: 0 }]` → `inline*`; `[{of:['paragraph'],min:1,max:1},{of:[a,b],min:0}]` → `paragraph (a | b)*`. */
export function contentExpression(items: ContentItem[] | undefined): string {
  if (!items || items.length === 0) return ''
  return items.map((item) => {
    const names = item.of.map((name) => (name.startsWith('@') ? name.slice(1) : name))
    const base = names.length === 1 ? names[0] : `(${names.join(' | ')})`
    const { min, max } = item
    if (max === undefined) return min === 0 ? `${base}*` : min === 1 ? `${base}+` : `${base}{${min},}`
    if (min === 1 && max === 1) return base
    if (min === 0 && max === 1) return `${base}?`
    return `${base}{${min},${max}}`
  }).join(' ')
}

function attrSpecs(def: { attrs?: Record<string, AttrDef>; block_id?: boolean }): Record<string, AttributeSpec> {
  const out: Record<string, AttributeSpec> = {}
  // block_id is assigned by the block-id plugin right after a node appears; `null` never reaches a commit.
  if (def.block_id) out.block_id = { default: null }
  for (const [name, attr] of Object.entries(def.attrs ?? {})) {
    out[name] = attr.default !== undefined ? { default: attr.default } : attr.required ? {} : { default: null }
  }
  return out
}

const blockAttrs = (node: PMNode): Record<string, string> => (node.attrs.block_id ? { 'data-block-id': String(node.attrs.block_id) } : {})
const blockIdFromDom = (dom: HTMLElement) => dom.getAttribute('data-block-id')

/** Presentation only, keyed by node name. A node of the JSON without an entry here is rendered generically. */
const nodeDom: Record<string, Pick<NodeSpec, 'toDOM' | 'parseDOM' | 'selectable' | 'draggable' | 'defining' | 'isolating'>> = {
  paragraph: {
    toDOM: (node): DOMOutputSpec => ['p', blockAttrs(node), 0],
    parseDOM: [{ tag: 'p', getAttrs: (dom) => ({ block_id: blockIdFromDom(dom) }) }],
  },
  heading: {
    defining: true,
    toDOM: (node): DOMOutputSpec => [`h${node.attrs.level}`, blockAttrs(node), 0],
    parseDOM: [1, 2, 3].map((level): TagParseRule => ({ tag: `h${level}`, getAttrs: (dom) => ({ level, block_id: blockIdFromDom(dom) }) })),
  },
  bullet_list: {
    toDOM: (node): DOMOutputSpec => ['ul', blockAttrs(node), 0],
    parseDOM: [{ tag: 'ul', getAttrs: (dom) => ({ block_id: blockIdFromDom(dom) }) }],
  },
  ordered_list: {
    toDOM: (node): DOMOutputSpec => ['ol', { ...blockAttrs(node), ...(node.attrs.start === 1 ? {} : { start: String(node.attrs.start) }) }, 0],
    parseDOM: [{ tag: 'ol', getAttrs: (dom) => ({ block_id: blockIdFromDom(dom), start: Math.max(1, Number(dom.getAttribute('start') ?? 1) || 1) }) }],
  },
  list_item: {
    defining: true,
    toDOM: (node): DOMOutputSpec => ['li', blockAttrs(node), 0],
    parseDOM: [{ tag: 'li', getAttrs: (dom) => ({ block_id: blockIdFromDom(dom) }) }],
  },
  object_embed: {
    selectable: true,
    draggable: true,
    isolating: true,
    toDOM: (node): DOMOutputSpec => ['div', { ...blockAttrs(node), class: 'aiws-embed', 'data-ref': JSON.stringify(node.attrs.ref) }],
    parseDOM: [{
      tag: 'div.aiws-embed[data-ref]',
      getAttrs: (dom) => {
        try { return { ref: JSON.parse(dom.getAttribute('data-ref') ?? ''), block_id: blockIdFromDom(dom) } } catch { return false }
      },
    }],
  },
  hard_break: {
    selectable: false,
    toDOM: (): DOMOutputSpec => ['br'],
    parseDOM: [{ tag: 'br' }],
  },
  object_link: {
    selectable: true,
    toDOM: (node): DOMOutputSpec => ['span', { class: 'aiws-object-link', 'data-ref': JSON.stringify(node.attrs.ref), 'data-label': String(node.attrs.label ?? '') }, String(node.attrs.label || node.attrs.ref?.entity_id || '')],
    parseDOM: [{
      tag: 'span.aiws-object-link[data-ref]',
      getAttrs: (dom) => {
        try { return { ref: JSON.parse(dom.getAttribute('data-ref') ?? ''), label: dom.getAttribute('data-label') ?? '' } } catch { return false }
      },
    }],
  },
}

function allowedHref(href: string, schemes: string[]): boolean {
  return schemes.some((scheme) => href.startsWith(scheme))
}

function markDom(name: string, def: MarkDef): Pick<MarkSpec, 'toDOM' | 'parseDOM'> {
  switch (name) {
    case 'strong': return { toDOM: () => ['strong', 0], parseDOM: [{ tag: 'strong' }, { tag: 'b' }, { style: 'font-weight=bold' }] }
    case 'em': return { toDOM: () => ['em', 0], parseDOM: [{ tag: 'em' }, { tag: 'i' }] }
    case 'strike': return { toDOM: () => ['s', 0], parseDOM: [{ tag: 's' }, { tag: 'del' }, { tag: 'strike' }] }
    case 'code': return { toDOM: () => ['code', 0], parseDOM: [{ tag: 'code' }] }
    case 'link': {
      const schemes = def.attrs?.href?.schemes ?? []
      return {
        toDOM: (mark) => ['a', { href: String(mark.attrs.href), rel: 'noopener noreferrer', target: '_blank' }, 0],
        parseDOM: [{
          tag: 'a[href]',
          // A pasted link with a scheme the schema forbids is dropped here: the backend would reject the whole update.
          getAttrs: (dom) => { const href = dom.getAttribute('href') ?? ''; return allowedHref(href, schemes) ? { href } : false },
        }],
      }
    }
    default: return { toDOM: () => ['span', { 'data-mark': name }, 0] }
  }
}

export function buildRichTextSchema(def: RichTextSchemaDef): Schema {
  const nodes: Record<string, NodeSpec> = {}
  // ProseMirror takes the first node of the map as the top node unless `topNode` is given; keep the JSON order otherwise.
  for (const [name, node] of Object.entries(def.nodes)) {
    if (node.text) { nodes[name] = { group: node.group }; continue }
    const inline = node.group === 'inline'
    nodes[name] = {
      ...(node.group ? { group: node.group } : {}),
      ...(inline ? { inline: true } : {}),
      ...(node.atom ? { atom: true } : {}),
      content: contentExpression(node.content),
      attrs: attrSpecs(node),
      ...(nodeDom[name] ?? { toDOM: (): DOMOutputSpec => (inline ? ['span', { 'data-node': name }] : ['div', { 'data-node': name }, 0]) }),
    }
  }
  const marks: Record<string, MarkSpec> = {}
  for (const [name, mark] of Object.entries(def.marks)) {
    marks[name] = {
      attrs: attrSpecs(mark),
      // `inclusive: false` is also what makes the Loro text style `expand: none` (same rule in the backend).
      ...(mark.inclusive === false ? { inclusive: false } : {}),
      ...(mark.exclusive ? { excludes: '_' } : {}),
      ...markDom(name, mark),
    }
  }
  return new Schema({ nodes, marks, topNode: def.top })
}

/** Loro text style configuration, derived exactly as loro-prosemirror derives it from the schema. */
export function loroTextStyles(def: RichTextSchemaDef): Record<string, { expand: 'after' | 'none' }> {
  return Object.fromEntries(Object.entries(def.marks).map(([name, mark]) => [name, { expand: mark.inclusive === false ? 'none' as const : 'after' as const }]))
}
