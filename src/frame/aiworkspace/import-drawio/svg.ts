/* SVG shape bodies (draw.io 导入 TODO §2.4): a body is inlined into the page, so it is cleaned before it is stored
 * and again before it is drawn — no scripts, event attributes, `foreignObject`, animation or external references
 * (a known residual risk otherwise, per "先能力后安全"). A pasted SVG document becomes a recolourable body: its
 * most used fill and stroke colours turn into the shape's fill and stroke (CSS custom properties). */

import type { SvgBody } from './shapes.ts'
import { elements, escapeAttr, escapeText, parseXml, type XmlElement, type XmlNode } from './xml.ts'

const ALLOWED = new Set([
  'g', 'path', 'rect', 'circle', 'ellipse', 'line', 'polyline', 'polygon', 'text', 'tspan', 'textPath', 'defs', 'symbol', 'use',
  'linearGradient', 'radialGradient', 'stop', 'clipPath', 'mask', 'pattern', 'marker', 'title', 'desc', 'style', 'image', 'filter',
  'feGaussianBlur', 'feOffset', 'feBlend', 'feColorMatrix', 'feFlood', 'feComposite', 'feMerge', 'feMergeNode', 'feDropShadow', 'switch',
])
/** Containers whose content is kept while the element itself is dropped (a link around a drawing). */
const UNWRAP = new Set(['a', 'svg'])

const localRef = (v: string) => v.startsWith('#') || /^data:image\/(png|jpeg|gif|webp);/i.test(v)
const cleanCss = (css: string) => css.replace(/@import[^;]*;?/gi, '').replace(/url\(\s*(['"]?)(?!#)[^)]*\1\s*\)/gi, 'none').replace(/expression\s*\(/gi, '(')

function cleanAttrs(el: XmlElement): Record<string, string> {
  const out: Record<string, string> = {}
  for (const [k, v] of Object.entries(el.attrs)) {
    const key = k.toLowerCase()
    if (key.startsWith('on') || key === 'xmlns' || key.startsWith('xmlns:')) continue
    if ((key === 'href' || key === 'xlink:href' || key === 'src') && !localRef(v.trim())) continue
    if (key === 'style') { out[k] = cleanCss(v); continue }
    if (/url\(/i.test(v) && !/url\(\s*['"]?#/i.test(v)) continue
    if (/^\s*javascript:/i.test(v)) continue
    out[k] = v
  }
  return out
}

function cleanNode(node: XmlNode, out: XmlNode[]) {
  if (typeof node === 'string') { out.push(node); return }
  if (UNWRAP.has(node.name)) { for (const c of node.children) cleanNode(c, out); return }
  if (!ALLOWED.has(node.name)) return
  const children: XmlNode[] = []
  for (const c of node.children) {
    if (node.name === 'style') { if (typeof c === 'string') children.push(cleanCss(c)) } else cleanNode(c, children)
  }
  out.push({ name: node.name, attrs: cleanAttrs(node), children })
}

function serialize(nodes: XmlNode[]): string {
  return nodes.map((n) => {
    if (typeof n === 'string') return escapeText(n)
    const attrs = Object.entries(n.attrs).map(([k, v]) => ` ${k}="${escapeAttr(v)}"`).join('')
    return n.children.length ? `<${n.name}${attrs}>${serialize(n.children)}</${n.name}>` : `<${n.name}${attrs}/>`
  }).join('')
}

const bodyCache = new Map<string, string>()

/** The body with everything that could run code or load something removed (cached by content). */
export function sanitizeSvgBody(body: string): string {
  const known = bodyCache.get(body)
  if (known !== undefined) return known
  let out = ''
  try {
    const root = parseXml(`<g>${body}</g>`)
    const kept: XmlNode[] = []
    for (const c of root.children) cleanNode(c, kept)
    out = serialize(kept)
  } catch {
    out = ''
  }
  if (bodyCache.size > 500) bodyCache.clear()
  bodyCache.set(body, out)
  return out
}

const HEX6 = /^#[0-9a-f]{6}$/i
function hexOf(v: string | undefined): string | null {
  if (!v) return null
  const t = v.trim().toLowerCase()
  if (HEX6.test(t)) return t
  if (/^#[0-9a-f]{3}$/.test(t)) return `#${t[1]}${t[1]}${t[2]}${t[2]}${t[3]}${t[3]}`
  if (t === 'black') return '#000000'
  if (t === 'white') return '#ffffff'
  return null
}

function styleProp(style: string | undefined, prop: string): string | undefined {
  if (!style) return undefined
  const m = new RegExp(`(?:^|;)\\s*${prop}\\s*:\\s*([^;]+)`, 'i').exec(style)
  return m?.[1].trim()
}

function setStyleProp(el: XmlElement, prop: string, value: string) {
  const rest = (el.attrs.style ?? '').split(';').map((d) => d.trim()).filter((d) => d && !new RegExp(`^${prop}\\s*:`, 'i').test(d))
  el.attrs.style = [...rest, `${prop}:${value}`].join(';')
  delete el.attrs[prop]
}

const SHAPES = new Set(['path', 'rect', 'circle', 'ellipse', 'line', 'polyline', 'polygon', 'text', 'tspan', 'use'])

/** A pasted or dropped SVG document → a shape body whose dominant colours follow the shape's fill and stroke
 * (`fill` / `stroke`: what to set on the shape so it looks as before). Null when it is not an SVG. */
export function svgShapeFromDocument(text: string): { svg: SvgBody; fill: string; stroke: string | null; w: number; h: number } | null {
  let root: XmlElement
  try { root = parseXml(text.trim()) } catch { return null }
  if (root.name !== 'svg') return null
  const num = (v: string | undefined) => { const n = parseFloat(v ?? ''); return Number.isFinite(n) && n > 0 ? n : null }
  let viewBox = root.attrs.viewBox?.trim()
  const w = num(root.attrs.width), h = num(root.attrs.height)
  if (!viewBox || !/^-?[\d.]+(?:[\s,]+-?[\d.]+){3}$/.test(viewBox)) viewBox = `0 0 ${w ?? 100} ${h ?? 100}`
  const [, , vw, vh] = viewBox.split(/[\s,]+/).map(Number)
  // count colours on drawing elements
  const fills = new Map<string, number>()
  const strokes = new Map<string, number>()
  const walk = (el: XmlElement) => {
    if (SHAPES.has(el.name)) {
      const f = hexOf(styleProp(el.attrs.style, 'fill') ?? el.attrs.fill)
      const s = hexOf(styleProp(el.attrs.style, 'stroke') ?? el.attrs.stroke)
      if (f) fills.set(f, (fills.get(f) ?? 0) + 1)
      if (s) strokes.set(s, (strokes.get(s) ?? 0) + 1)
    }
    for (const c of elements(el)) walk(c)
  }
  walk(root)
  // what the drawing inherits from the root (`fill="none" stroke="currentColor"` of an icon set)
  const rootPaint = (prop: 'fill' | 'stroke') => (styleProp(root.attrs.style, prop) ?? root.attrs[prop])?.trim()
  const rootFill = rootPaint('fill') ?? '#000000'
  const rootStroke = rootPaint('stroke') ?? 'none'
  if (hexOf(rootFill) || rootFill === 'currentColor') fills.set(hexOf(rootFill) ?? 'currentColor', (fills.get(hexOf(rootFill) ?? 'currentColor') ?? 0) + 1)
  if (hexOf(rootStroke) || rootStroke === 'currentColor') strokes.set(hexOf(rootStroke) ?? 'currentColor', (strokes.get(hexOf(rootStroke) ?? 'currentColor') ?? 0) + 1)
  const top = (m: Map<string, number>) => [...m].sort((a, b) => b[1] - a[1])[0]?.[0] ?? null
  const fill = top(fills)
  const stroke = top(strokes)
  const recolour = (el: XmlElement) => {
    if (SHAPES.has(el.name)) {
      if (fill && hexOf(styleProp(el.attrs.style, 'fill') ?? el.attrs.fill) === fill) setStyleProp(el, 'fill', 'var(--aiws-shape-fill)')
      if (stroke && hexOf(styleProp(el.attrs.style, 'stroke') ?? el.attrs.stroke) === stroke) setStyleProp(el, 'stroke', 'var(--aiws-shape-stroke)')
    }
    for (const c of elements(el)) recolour(c)
  }
  recolour(root)
  const paint = (value: string, dominant: string | null, variable: string) => (value === 'currentColor' || (dominant && hexOf(value) === dominant) ? `var(${variable})` : value)
  const inherited = [
    `fill:${paint(rootFill, fill, '--aiws-shape-fill')}`,
    `stroke:${paint(rootStroke, stroke, '--aiws-shape-stroke')}`,
    ...(['stroke-width', 'stroke-linecap', 'stroke-linejoin'] as const).flatMap((k) => { const v = root.attrs[k]; return v && /^[\w.-]+$/.test(v) ? [`${k}:${v}`] : [] }),
  ].join(';')
  const kept: XmlNode[] = []
  for (const c of root.children) cleanNode(c, kept)
  const body = `<g style="${inherited}">${serialize(kept)}</g>`
  const shown = (c: string | null) => (c === 'currentColor' ? 'auto' : c)
  return { svg: { viewBox, body, aspect: 'fixed' }, fill: shown(fill) ?? 'none', stroke: shown(stroke), w: w ?? vw ?? 100, h: h ?? vh ?? 100 }
}
