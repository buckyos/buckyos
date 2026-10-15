/* draw.io stencils (mxStencil) → the SVG body of a `shape` Block of kind `svg` (draw.io 导入 TODO §2.4). A stencil
 * is a small drawing program in its own w × h box: state ops (colours, widths, dashes, save / restore), drawing ops
 * (path, rect, roundrect, ellipse, text) and paint ops (fill, stroke, fillstroke) that paint the current drawing.
 * It is run here once at its own size, the way mxStencil.drawShape runs it on a canvas (background, then
 * foreground, one state carried through both), and every paint becomes one SVG element.
 *
 *   colours   the shape's own colours are CSS variables the renderer sets on the host `<svg>`: --aiws-shape-fill,
 *             --aiws-shape-stroke, --aiws-shape-font (else currentColor). Fixed colours stay literal; a style key a
 *             stencil reads (`fillColor2`) falls back to the stencil's `default`, as in a cell without that key
 *   widths    --aiws-shape-stroke-width is the shape's stroke width; a stencil's widths (`strokewidth="2"` on the
 *             shape, `<strokewidth width>`) multiply it, `fixed="1"` ones are literal. Strokes never scale with
 *             the body (vector-effect: non-scaling-stroke), which is stretched to the Block or, for
 *             `aspect="fixed"`, fitted into it
 *   alpha     alpha, fillalpha and strokealpha all set the one alpha, as mxStencil does
 *   output    a `<g>` carrying the defaults, then one element per paint with only its differences in `style`.
 *             Everything written is built here from numbers, checked colours and escaped text: no script, event
 *             attributes, links, images or foreignObject
 *   skipped   image, include-shape without a resolver, unknown ops: listed in `unsupported`. A stencil that draws
 *             nothing becomes a plain rectangle with its title
 *
 * Names (mxStencilRegistry.parseStencilSet in draw.io's Graph.js): `<shapes name="mxGraph.flowchart">` with
 * `<shape name="Or Gate">` registers `mxgraph.flowchart.or_gate` (package lower-cased, spaces → `_`, lower-cased),
 * the name a style uses (`shape=mxgraph.flowchart.or_gate`). The file is found from the name
 * (getBasenameForStencil): `mxgraph.<a>.<b>.<shape>` → `<a>/<b>.xml`. */

import { ANCHOR_ID } from './anchors.ts'
import { base64ToBytes, inflateRaw, utf8 } from './bytes.ts'
import type { SvgBody } from './shapes.ts'
import { elements, escapeAttr, escapeText, findAll, parseXml, type XmlElement } from './xml.ts'

export interface StencilAnchor { id: string; x: number; y: number }

export interface StencilSvg {
  /** As registered, e.g. `mxgraph.flowchart.decision`; '' for an inline stencil. */
  name: string
  /** The `<shape name>`. */
  title: string
  /** The stencil's own box. */
  w: number
  h: number
  svg: SvgBody
  /** From `<connections>`, in the unit box; ids are anchor ids (the constraint's name, else c0, c1, …). */
  anchors: StencilAnchor[]
  /** Instructions skipped or approximated, once each. */
  unsupported: string[]
}

export interface StencilOptions {
  name?: string
  /** Finds the stencil an `<include-shape name>` draws. */
  resolve?: (name: string) => XmlElement | null
}

const FILL = 'var(--aiws-shape-fill)'
const STROKE = 'var(--aiws-shape-stroke)'
const FONT = 'var(--aiws-shape-font, currentColor)'
const WIDTH = 'var(--aiws-shape-stroke-width)'
const DEFAULT_FONT_SIZE = 11 // mxConstants.DEFAULT_FONTSIZE
const ROUNDING_FACTOR = 0.15 // mxConstants.RECTANGLE_ROUNDING_FACTOR
const MAX_INCLUDE_DEPTH = 8

/** The registered name of a stencil: `stencilName('mxGraph.flowchart', 'Or Gate')` → `mxgraph.flowchart.or_gate`. */
export function stencilName(packageName: string | null | undefined, shapeName: string): string {
  return (packageName ? `${packageName}.`.toLowerCase() : '') + shapeName.replace(/ /g, '_').toLowerCase()
}

/** The stencil file holding a registered name, relative to draw.io's stencil folder (`mxgraph.cisco.routers.router`
 * → `cisco/routers.xml`), or null for a name outside `mxgraph.*`. */
export function stencilFile(name: string): string | null {
  const parts = name.split('.')
  if (parts.length < 2 || parts[0] !== 'mxgraph' || !parts[1]) return null
  return parts.slice(1, Math.max(2, parts.length - 1)).join('/').replace('_-_', '_') + '.xml'
}

/** The value inside a style's `shape=stencil(...)` (the whole `stencil(...)` is accepted too): base64 of the raw
 * deflate of the URL-encoded XML (draw.io's Graph.compress) → the `<shape>` element; null when it is not one. */
export async function decodeStencilStyle(value: string): Promise<XmlElement | null> {
  try {
    let v = value.trim()
    if (v.startsWith('stencil(') && v.endsWith(')')) v = v.slice(8, -1)
    let text = utf8.decode(await inflateRaw(base64ToBytes(v)))
    if (!text.trimStart().startsWith('<')) text = decodeURIComponent(text)
    // Graph.zapGremlins: control characters other than tab, LF and CR
    // control characters are not XML; a broken stencil value may carry some
    // deno-lint-ignore no-control-regex
    const root = parseXml(text.replace(/[\x00-\x08\x0b\x0c\x0e-\x1f]/g, ''))
    return findAll(root, 'shape')[0] ?? null
  } catch {
    return null
  }
}

// ---- values ----

const fmt = (v: number) => String(Math.round(v * 100) / 100)

function num(v: string | undefined, fallback = 0): number {
  if (v === undefined) return fallback
  const n = Number(v)
  return Number.isFinite(n) ? n : fallback
}

const NAMED_COLORS = new Set(('aliceblue antiquewhite aqua aquamarine azure beige bisque black blanchedalmond blue blueviolet brown ' +
  'burlywood cadetblue chartreuse chocolate coral cornflowerblue cornsilk crimson cyan darkblue darkcyan darkgoldenrod darkgray ' +
  'darkgreen darkgrey darkkhaki darkmagenta darkolivegreen darkorange darkorchid darkred darksalmon darkseagreen darkslateblue ' +
  'darkslategray darkslategrey darkturquoise darkviolet deeppink deepskyblue dimgray dimgrey dodgerblue firebrick floralwhite ' +
  'forestgreen fuchsia gainsboro ghostwhite gold goldenrod gray green greenyellow grey honeydew hotpink indianred indigo ivory ' +
  'khaki lavender lavenderblush lawngreen lemonchiffon lightblue lightcoral lightcyan lightgoldenrodyellow lightgray lightgreen ' +
  'lightgrey lightpink lightsalmon lightseagreen lightskyblue lightslategray lightslategrey lightsteelblue lightyellow lime ' +
  'limegreen linen magenta maroon mediumaquamarine mediumblue mediumorchid mediumpurple mediumseagreen mediumslateblue ' +
  'mediumspringgreen mediumturquoise mediumvioletred midnightblue mintcream mistyrose moccasin navajowhite navy oldlace olive ' +
  'olivedrab orange orangered orchid palegoldenrod palegreen paleturquoise palevioletred papayawhip peachpuff peru pink plum ' +
  'powderblue purple rebeccapurple red rosybrown royalblue saddlebrown salmon sandybrown seagreen seashell sienna silver skyblue ' +
  'slateblue slategray slategrey snow springgreen steelblue tan teal thistle tomato turquoise violet wheat white whitesmoke yellow ' +
  'yellowgreen').split(' '))

/** A colour literal as written into CSS, null for none, undefined when the value is no colour. */
function cssColor(value: string): string | null | undefined {
  const v = value.trim().toLowerCase()
  if (v === 'none' || v === 'transparent') return null
  if (/^#(?:[0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/.test(v)) return v
  if (/^(?:rgb|hsl)a?\([\d\s.,%/+-]*\)$/.test(v)) return v.replace(/\s+/g, ' ')
  return NAMED_COLORS.has(v) ? v : undefined
}

/** mxStencil.parseColor for a `<fillcolor>`, `<strokecolor>` or `<fontcolor>`: `fill` / `stroke` / `font` and the
 * style keys holding the shape's colours are the shape's colours; any other style key is absent, so the node's
 * `default` applies (`default` itself being the theme's background or foreground). */
function readColor(node: XmlElement, kind: 'fill' | 'stroke' | 'font'): string | null {
  const value = node.attrs.color
  if (value === undefined) return null
  const v = value.trim()
  if (v === 'fill' || v === 'fillColor') return FILL
  if (v === 'stroke' || v === 'strokeColor') return STROKE
  if (v === 'font' || v === 'fontColor') return FONT
  const literal = cssColor(v)
  if (literal !== undefined) return literal
  const fallback = node.attrs.default
  if (fallback === undefined) return null
  if (fallback === 'default') return kind === 'fill' ? '#ffffff' : '#000000'
  return cssColor(fallback) ?? null
}

// ---- the run ----

interface State {
  fill: string | null
  stroke: string | null
  font: string | null
  /** Stroke width: a multiple of the shape's width, or world units when fixed. */
  width: number
  fixed: boolean
  alpha: number
  /** null: as the shape (its own dash style, if any, is inherited from the host). */
  dashed: boolean | null
  dash: string
  join: string
  cap: string
  miter: number
  fontSize: number
  fontStyle: number
  fontFamily: string | null
}

/** Where a stencil's units land: x0 + x · sx (mxStencil's aspect rectangle). */
interface Aspect { x0: number; y0: number; sx: number; sy: number }

type Pending =
  | { tag: 'path' }
  | { tag: 'rect'; x: number; y: number; w: number; h: number; r: number }
  | { tag: 'ellipse'; x: number; y: number; w: number; h: number }

interface Run {
  out: string[]
  unsupported: Set<string>
  state: State
  stack: State[]
  path: string[] | null
  node: Pending | null
  baseWidth: string
  resolve?: (name: string) => XmlElement | null
}

const widthCss = (s: State) => (s.fixed ? fmt(s.width) : s.width === 1 ? WIDTH : `calc(${WIDTH} * ${fmt(s.width)})`)

/** mxSvgCanvas2D.createDashPattern: the pattern in stroke widths (non-numbers ignored; none left: solid). */
function dashCss(s: State): string | null {
  const values = s.dash.split(' ').filter((t) => t !== '' && Number.isFinite(Number(t))).map(Number)
  if (!values.length) return null
  return values.map((v) => (s.fixed ? fmt(v * s.width) : `calc(${WIDTH} * ${fmt(v * s.width)})`)).join(' ')
}

function initialState(): State {
  return {
    fill: FILL, stroke: STROKE, font: FONT, width: 1, fixed: false, alpha: 1, dashed: null, dash: '3 3',
    join: 'miter', cap: 'flat', miter: 10, fontSize: DEFAULT_FONT_SIZE, fontStyle: 0, fontFamily: null,
  }
}

function attrs(list: [string, string | number][]): string {
  return list.map(([k, v]) => ` ${k}="${escapeAttr(typeof v === 'number' ? fmt(v) : v)}"`).join('')
}

/** mxSvgCanvas2D.addNode: the pending drawing, painted with the current state; invisible results are dropped. */
function paint(run: Run, filled: boolean, stroked: boolean): void {
  const node = run.node
  if (!node) return
  let geometry: string
  if (node.tag === 'path') {
    if (!run.path?.length) return
    geometry = attrs([['d', run.path.join('')]])
  } else if (node.tag === 'rect') {
    const list: [string, number][] = [['x', node.x], ['y', node.y], ['width', node.w], ['height', node.h]]
    if (node.r > 0) list.push(['rx', node.r], ['ry', node.r])
    geometry = attrs(list)
  } else {
    geometry = attrs([['cx', node.x + node.w / 2], ['cy', node.y + node.h / 2], ['rx', node.w / 2], ['ry', node.h / 2]])
  }
  run.node = null
  if (node.tag !== 'path' && (node.w <= 0 || node.h <= 0)) return
  const s = run.state
  const fill = filled ? s.fill : null
  const stroke = stroked ? s.stroke : null
  if (fill === null && stroke === null) return
  const css: string[] = []
  if (fill !== FILL) css.push(`fill:${fill ?? 'none'}`)
  if (fill !== null && s.alpha < 1) css.push(`fill-opacity:${fmt(s.alpha)}`)
  if (stroke === null) css.push('stroke:none')
  else {
    if (stroke !== STROKE) css.push(`stroke:${stroke}`)
    if (s.alpha < 1) css.push(`stroke-opacity:${fmt(s.alpha)}`)
    const width = widthCss(s)
    if (width !== run.baseWidth) css.push(`stroke-width:${width}`)
    if (s.dashed === false) css.push('stroke-dasharray:none')
    else if (s.dashed) css.push(`stroke-dasharray:${dashCss(s) ?? 'none'}`)
    if (node.tag === 'path') {
      // mxSvgCanvas2D.updateStrokeAttributes (paths only); `flat` is SVG's butt
      if (s.join !== 'miter') css.push(`stroke-linejoin:${s.join}`)
      if (s.cap !== 'flat') css.push(`stroke-linecap:${s.cap}`)
      if (s.miter !== 10) css.push(`stroke-miterlimit:${fmt(s.miter)}`)
    }
    css.push('vector-effect:non-scaling-stroke')
  }
  run.out.push(`<${node.tag}${geometry} style="${css.join(';')}"/>`)
}

function text(run: Run, node: XmlElement, a: Aspect): void {
  const s = run.state
  const str = node.attrs.str
  if (!str || s.font === null) return
  if (node.attrs['align-shape'] === '0') run.unsupported.add('text-align-shape')
  if (node.attrs['flip-shape'] === '0') run.unsupported.add('text-flip-shape')
  const tx = a.x0 + num(node.attrs.x) * a.sx
  const ty = a.y0 + num(node.attrs.y) * a.sy
  const align = node.attrs.align ?? 'left'
  const valign = node.attrs.valign ?? 'top'
  const rotation = (node.attrs.vertical === '1' ? -90 : 0) - num(node.attrs.rotation)
  // mxSvgCanvas2D.plainText: the first baseline from the anchor point and the block height
  const size = s.fontSize
  const lines = str.split('\n')
  const lh = Math.round(size * 1.2)
  const height = size + (lines.length - 1) * lh
  let y = ty + size - 1
  if (valign === 'middle') y -= height / 2
  else if (valign === 'bottom') y -= height + 1
  const css = [`fill:${s.font}`, 'stroke:none', `font-size:${fmt(size)}px`]
  if (s.fontFamily) css.push(`font-family:${s.fontFamily}`)
  if (s.fontStyle & 1) css.push('font-weight:bold')
  if (s.fontStyle & 2) css.push('font-style:italic')
  const deco = [s.fontStyle & 4 ? 'underline' : '', s.fontStyle & 8 ? 'line-through' : ''].filter(Boolean).join(' ')
  if (deco) css.push(`text-decoration:${deco}`)
  css.push(`text-anchor:${align === 'right' ? 'end' : align === 'center' ? 'middle' : 'start'}`, 'white-space:pre')
  if (s.alpha < 1) css.push(`opacity:${fmt(s.alpha)}`)
  const transform = rotation ? attrs([['transform', `rotate(${fmt(rotation)} ${fmt(tx)} ${fmt(ty)})`]]) : ''
  const content = lines.length === 1
    ? escapeText(str)
    : lines.map((line, i) => `<tspan${attrs([['x', tx], ['dy', i ? lh : 0]])}>${escapeText(line)}</tspan>`).join('')
  run.out.push(`<text${attrs([['x', tx], ['y', y]])}${transform} style="${css.join(';')}">${content}</text>`)
}

/** mxShape.addPoints with rounded corners: a `<path rounded="1" arcSize>` made of moves and lines only. */
function roundedPoints(run: Run, pts: { x: number; y: number }[], arcSize: number, close: boolean): void {
  const path = run.path!
  if (close) {
    const pe = pts[pts.length - 1]
    pts = [{ x: pe.x + (pts[0].x - pe.x) / 2, y: pe.y + (pts[0].y - pe.y) / 2 }, ...pts]
  }
  let pt = pts[0]
  path.push(`M${fmt(pt.x)} ${fmt(pt.y)}`)
  const mod = (i: number) => ((i % pts.length) + pts.length) % pts.length
  let i = 1
  while (i < (close ? pts.length : pts.length - 1)) {
    let tmp = pts[mod(i)]
    let dx = pt.x - tmp.x
    let dy = pt.y - tmp.y
    if (dx !== 0 || dy !== 0) {
      let dist = Math.sqrt(dx * dx + dy * dy)
      path.push(`L${fmt(tmp.x + (dx * Math.min(arcSize, dist / 2)) / dist)} ${fmt(tmp.y + (dy * Math.min(arcSize, dist / 2)) / dist)}`)
      let next = pts[mod(i + 1)]
      while (i < pts.length - 2 && Math.round(next.x - tmp.x) === 0 && Math.round(next.y - tmp.y) === 0) {
        next = pts[mod(i + 2)]
        i++
      }
      dx = next.x - tmp.x
      dy = next.y - tmp.y
      dist = Math.max(1, Math.sqrt(dx * dx + dy * dy))
      const end = { x: tmp.x + (dx * Math.min(arcSize, dist / 2)) / dist, y: tmp.y + (dy * Math.min(arcSize, dist / 2)) / dist }
      path.push(`Q${fmt(tmp.x)} ${fmt(tmp.y)} ${fmt(end.x)} ${fmt(end.y)}`)
      tmp = end
    } else path.push(`L${fmt(tmp.x)} ${fmt(tmp.y)}`)
    pt = tmp
    i++
  }
  if (close) path.push('Z')
  else {
    const pe = pts[pts.length - 1]
    path.push(`L${fmt(pe.x)} ${fmt(pe.y)}`)
  }
}

/** A rounded path in mxStencil: only moves and lines, else drawn as written. */
function roundedPath(run: Run, node: XmlElement, a: Aspect): boolean {
  const segs: { x: number; y: number }[][] = []
  for (const c of elements(node)) {
    if (c.name !== 'move' && c.name !== 'line') return false
    if (c.name === 'move' || segs.length === 0) segs.push([])
    segs[segs.length - 1].push({ x: a.x0 + num(c.attrs.x) * a.sx, y: a.y0 + num(c.attrs.y) * a.sy })
  }
  if (!segs.length) return false
  const arcSize = num(node.attrs.arcSize)
  for (const seg of segs) {
    const ps = seg[0]
    const pe = seg[seg.length - 1]
    const close = ps.x === pe.x && ps.y === pe.y
    const pts = close ? seg.slice(0, -1) : seg
    if (pts.length) roundedPoints(run, pts, arcSize, close)
  }
  return true
}

/** mxStencil.drawNode. */
function drawNode(run: Run, node: XmlElement, a: Aspect, depth: number): void {
  const s = run.state
  const at = node.attrs
  const X = (k: string) => fmt(a.x0 + num(at[k]) * a.sx)
  const Y = (k: string) => fmt(a.y0 + num(at[k]) * a.sy)
  const minScale = Math.min(a.sx, a.sy)
  switch (node.name) {
    case 'save': run.stack.push({ ...s }); break
    case 'restore': if (run.stack.length) run.state = run.stack.pop()!; break
    case 'path':
      run.path = []
      run.node = { tag: 'path' }
      if (at.rounded === '1' && roundedPath(run, node, a)) break
      for (const child of elements(node)) drawNode(run, child, a, depth)
      break
    case 'move': run.path?.push(`M${X('x')} ${Y('y')}`); break
    case 'line': run.path?.push(`L${X('x')} ${Y('y')}`); break
    case 'quad': run.path?.push(`Q${X('x1')} ${Y('y1')} ${X('x2')} ${Y('y2')}`); break
    case 'curve': run.path?.push(`C${X('x1')} ${Y('y1')} ${X('x2')} ${Y('y2')} ${X('x3')} ${Y('y3')}`); break
    case 'arc':
      run.path?.push(`A${fmt(num(at.rx) * a.sx)} ${fmt(num(at.ry) * a.sy)} ${fmt(num(at['x-axis-rotation']))} ` +
        `${num(at['large-arc-flag']) ? 1 : 0} ${num(at['sweep-flag']) ? 1 : 0} ${X('x')} ${Y('y')}`)
      break
    case 'close': run.path?.push('Z'); break
    case 'rect':
    case 'roundrect':
    case 'ellipse': {
      const box = { x: a.x0 + num(at.x) * a.sx, y: a.y0 + num(at.y) * a.sy, w: num(at.w) * a.sx, h: num(at.h) * a.sy }
      if (node.name === 'ellipse') run.node = { tag: 'ellipse', ...box }
      else {
        const arcsize = node.name === 'roundrect' ? num(at.arcsize) || ROUNDING_FACTOR * 100 : 0
        run.node = { tag: 'rect', ...box, r: Math.min(box.w, box.h) * arcsize / 100 }
      }
      break
    }
    case 'text': text(run, node, a); break
    case 'include-shape': {
      const sub = at.name && run.resolve && depth < MAX_INCLUDE_DEPTH ? run.resolve(at.name) : null
      if (!sub) { run.unsupported.add('include-shape'); break }
      drawShape(run, sub, a.x0 + num(at.x) * a.sx, a.y0 + num(at.y) * a.sy, num(at.w) * a.sx, num(at.h) * a.sy, depth + 1)
      break
    }
    case 'fillstroke': paint(run, true, true); break
    case 'fill': paint(run, true, false); break
    case 'stroke': paint(run, false, true); break
    case 'strokewidth':
      s.fixed = at.fixed === '1'
      s.width = num(at.width, 1) * (s.fixed ? 1 : minScale)
      break
    case 'dashed': s.dashed = at.dashed === '1'; break
    case 'dashpattern':
      if (at.pattern !== undefined) s.dash = at.pattern.split(' ').filter((t) => t !== '').map((t) => fmt(Number(t) * minScale)).join(' ')
      break
    case 'strokecolor': s.stroke = readColor(node, 'stroke'); break
    case 'fillcolor': s.fill = readColor(node, 'fill'); break
    case 'fontcolor': s.font = readColor(node, 'font'); break
    case 'alpha':
    case 'fillalpha':
    case 'strokealpha':
      s.alpha = Math.min(1, Math.max(0, num(at.alpha, 1)))
      break
    case 'linejoin': if (at.join === 'miter' || at.join === 'round' || at.join === 'bevel') s.join = at.join; break
    case 'linecap': if (at.cap === 'flat' || at.cap === 'butt' || at.cap === 'round' || at.cap === 'square') s.cap = at.cap === 'butt' ? 'flat' : at.cap; break
    case 'miterlimit': s.miter = Math.max(1, num(at.limit, 10)); break
    case 'fontsize': s.fontSize = num(at.size, DEFAULT_FONT_SIZE) * minScale; break
    case 'fontstyle': s.fontStyle = num(at.style) | 0; break
    case 'fontfamily': s.fontFamily = (at.family ?? '').replace(/[^\w ,-]/g, '').trim() || null; break
    default: run.unsupported.add(node.name)
  }
}

function stencilSize(shape: XmlElement): { w: number; h: number } {
  const w = num(shape.attrs.w, 100)
  const h = num(shape.attrs.h, 100)
  return { w: w > 0 ? w : 100, h: h > 0 ? h : 100 }
}

/** mxStencil.drawShape (no direction: the Block turns the body itself). */
function drawShape(run: Run, shape: XmlElement, x: number, y: number, w: number, h: number, depth: number): void {
  const size = stencilSize(shape)
  let sx = w / size.w
  let sy = h / size.h
  let x0 = x
  let y0 = y
  if (shape.attrs.aspect === 'fixed') {
    sx = sy = Math.min(sx, sy)
    x0 += (w - size.w * sx) / 2
    y0 += (h - size.h * sy) / 2
  }
  const a = { x0, y0, sx, sy }
  const sw = shape.attrs.strokewidth ?? '1'
  run.state.fixed = false
  run.state.width = sw === 'inherit' ? 1 : num(sw, 1) * Math.min(sx, sy)
  if (!(w > 0 && h > 0)) return
  for (const part of ['background', 'foreground']) {
    const node = findAll(shape, part)[0]
    if (node) for (const child of elements(node)) drawNode(run, child, a, depth)
  }
}

function anchorsOf(shape: XmlElement): StencilAnchor[] {
  const out: StencilAnchor[] = []
  const conns = findAll(shape, 'connections')[0]
  if (!conns) return out
  const seen = new Set<string>()
  const round = (v: number) => Math.round(Math.min(1, Math.max(0, v)) * 1e4) / 1e4
  elements(conns).forEach((c, i) => {
    let x = num(c.attrs.x)
    let y = num(c.attrs.y)
    if (c.attrs.perimeter === '1') {
      // projected from the centre through (x, y) onto the box (draw.io's default rectangle perimeter)
      const dx = x - 0.5
      const dy = y - 0.5
      const m = Math.max(Math.abs(dx), Math.abs(dy))
      if (m > 0) { x = 0.5 + (dx * 0.5) / m; y = 0.5 + (dy * 0.5) / m }
    }
    const base = (c.attrs.name ?? '').replace(/[^A-Za-z0-9_.:-]/g, '_').slice(0, 64) || `c${i}`
    let id = base
    for (let k = 2; seen.has(id) || !ANCHOR_ID.test(id); k++) id = `${base.slice(0, 60)}_${k}`
    seen.add(id)
    out.push({ id, x: round(x), y: round(y) })
  })
  return out
}

/** Run a `<shape>` stencil at its own size and return its SVG body, anchors and what was left out. */
export function stencilToSvg(shape: XmlElement, options: StencilOptions = {}): StencilSvg {
  const { w, h } = stencilSize(shape)
  const title = shape.attrs.name ?? ''
  const run: Run = {
    out: [], unsupported: new Set(), state: initialState(), stack: [], path: null, node: null, baseWidth: WIDTH,
    resolve: options.resolve,
  }
  const sw = shape.attrs.strokewidth
  run.baseWidth = widthCss({ ...run.state, width: sw === undefined || sw === 'inherit' ? 1 : num(sw, 1) })
  drawShape(run, shape, 0, 0, w, h, 0)
  if (!run.out.length) {
    run.unsupported.add('empty')
    run.out.push(`<rect${attrs([['x', 0], ['y', 0], ['width', w], ['height', h]])} style="vector-effect:non-scaling-stroke"/>`)
    if (title) {
      const size = Math.max(6, Math.min(14, h / 4))
      run.out.push(`<text${attrs([['x', w / 2], ['y', h / 2]])} style="fill:${FONT};stroke:none;font-size:${fmt(size)}px;text-anchor:middle;dominant-baseline:central">${escapeText(title)}</text>`)
    }
  }
  const group = `fill:${FILL};stroke:${STROKE};stroke-width:${run.baseWidth};stroke-miterlimit:10`
  return {
    name: options.name ?? '',
    title,
    w,
    h,
    svg: { viewBox: `0 0 ${fmt(w)} ${fmt(h)}`, body: `<g style="${group}">${run.out.join('')}</g>`, aspect: shape.attrs.aspect === 'fixed' ? 'fixed' : 'stretch' },
    anchors: anchorsOf(shape),
    unsupported: [...run.unsupported],
  }
}
