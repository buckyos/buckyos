/* draw.io page → import plan (draw.io 导入 TODO §3). One pass reads the cells, a second lays out the vertices
 * (absolute rectangles, containers, images, text and look keys), a third turns edges into connectors bound to
 * the anchors of the objects they meet; `operations.ts` turns the plan into commit operations.
 *
 *   order       XML order within each parent is the stacking order; layers are flattened in their order
 *   containers  a vertex with children becomes a group; a swimlane gets a frame covering it as first child
 *               (header = `startSize`, orientation from `horizontal`), any other visible container a shape
 *   ends        an edge's `exitX/exitY` (`entryX/entryY`) becomes the target's anchor at that spot (an own anchor
 *               is added when none sits there); a floating end takes the anchor nearest to where draw.io would
 *               meet the outline, an orthogonal one the middle of the side facing its way
 *   waypoints   absolute bends → controls in the endpoint frame of the resolved ends
 *   loss        everything not kept is counted in the report (§5), the original style is kept in `config.drawio` */

import { cleanAnchors, COMPASS, defaultAnchors, findAnchor, RECT_ANCHORS, type AnchorDef, type Shape } from './anchors.ts'
import { base64ToBytes } from './bytes.ts'
import { anchorWorld, boxOf, decompose, leftOf, pointAt, routeConnector, type Binding, type ConnectorData, type Control, type Route, type TargetGeom } from './connector.ts'
import type { DrawioPage } from './file.ts'
import { centerOf, normalizeRotation, rotatedBounds, type Point, type Rect } from './geometry.ts'
import { fillPlaceholders, htmlLabel, plainLabel, type LabelText } from './html.ts'
import { builtinSvg } from './builtinSvg.ts'
import { readLook, shapeAnchors, shapeOutline, type ShapeKind, type SvgBody } from './shapes.ts'
import { color, flag, num, parseStyle, resolveStyle, type Style } from './style.ts'
import { elements, firstElement, type XmlElement } from './xml.ts'

export type Json = string | number | boolean | null | Json[] | { [key: string]: Json }

/** A converted library or inline stencil: its SVG body, own anchors and what was not converted. */
export interface ResolvedStencil { title: string; svg: SvgBody; anchors: { id: string; x: number; y: number }[]; unsupported: string[] }

export interface ConvertOptions {
  /** A library shape (`shape=mxgraph.x.y`) as SVG, or null when not available (§2.4). */
  stencil?: (name: string) => Promise<ResolvedStencil | null>
  /** An inline stencil (`shape=stencil(…)`, the value inside the parentheses). */
  inlineStencil?: (value: string) => Promise<ResolvedStencil | null>
  /** The text of the dashed box that stands in for a picture that cannot be had (its URL, if any), in the host's
   * language; English by default. */
  imageText?: (url: string | null) => string
}

/** An image the host uploads before writing (a data URI's bytes, or a URL to fetch). */
export interface PlanImage { key: string; name: string; mediaType: string; bytes?: Uint8Array; url?: string }

export interface PlanEnd { key: string; anchor: string }
export interface PlanLine {
  start: PlanEnd | null
  end: PlanEnd | null
  /** Where the ends are now (absolute): the stored box of the line. */
  startPoint: Point
  endPoint: Point
  route: Route
  controls: Control[]
  label: { t: number; offset: number } | null
}

export type ItemType = 'group' | 'shape' | 'frame' | 'asset' | 'connector'

export interface PlanItem {
  key: string
  /** The enclosing group's key, or null on the page. */
  parent: string | null
  type: ItemType
  /** Absolute page coordinates (a connector: the box of its two ends). */
  rect: Rect
  rotation: number
  title?: string
  config: Record<string, Json>
  locked?: boolean
  /** An image Block's picture; when it cannot be uploaded the item becomes `fallback` (a shape). */
  image?: PlanImage
  fallback?: { config: Record<string, Json>; title?: string }
  line?: PlanLine
  /** Further payload keys of the created Block (a PDF page's Frame: `presentation`). */
  payload?: Record<string, Json>
}

export interface PagePlan {
  id: string
  name: string
  background: string | null
  pageSize: { w: number; h: number } | null
  /** Paint order, every parent before its children. Coordinates start at (0, 0). */
  items: PlanItem[]
  bounds: Rect
  images: PlanImage[]
}

export interface ReportNote { kind: string; message: string; count: number; samples: string[] }

/** What an import kept and what it could not (§5): counted per kind with a few examples. `kind` is a stable code
 * (`stencil:<instruction>` for a stencil instruction) the host words in its own language; `message` is English,
 * for developers. */
export class ImportReport {
  readonly notes = new Map<string, ReportNote>()
  objects = 0
  lines = 0
  images = 0
  pages = 0
  add(kind: string, message: string, sample?: string) {
    let note = this.notes.get(kind)
    if (!note) { note = { kind, message, count: 0, samples: [] }; this.notes.set(kind, note) }
    note.count += 1
    if (sample && note.samples.length < 5 && !note.samples.includes(sample)) note.samples.push(sample)
  }
  list(): ReportNote[] { return [...this.notes.values()] }
}

// ---- reading cells

interface RawGeo { x: number; y: number; w: number; h: number; relative: boolean; points: Point[]; sourcePoint?: Point; targetPoint?: Point; offset?: Point; alternate?: { w: number; h: number } }

interface RawCell {
  id: string
  parent: string | null
  value: string
  style: Style
  vertex: boolean
  edge: boolean
  source: string | null
  target: string | null
  visible: boolean
  collapsed: boolean
  geo: RawGeo | null
  data: Record<string, string>
  link?: string
  tooltip?: string
  placeholders: boolean
  order: number
}

const n0 = (v: string | undefined) => { const n = Number(v); return v !== undefined && Number.isFinite(n) ? n : 0 }
const pt = (e: XmlElement): Point => ({ x: n0(e.attrs.x), y: n0(e.attrs.y) })

function readGeometry(cell: XmlElement): RawGeo | null {
  const g = elements(cell, 'mxGeometry').find((e) => (e.attrs.as ?? 'geometry') === 'geometry')
  if (!g) return null
  const geo: RawGeo = { x: n0(g.attrs.x), y: n0(g.attrs.y), w: n0(g.attrs.width), h: n0(g.attrs.height), relative: g.attrs.relative === '1', points: [] }
  for (const child of elements(g)) {
    const as = child.attrs.as
    if (child.name === 'mxPoint' && as === 'sourcePoint') geo.sourcePoint = pt(child)
    else if (child.name === 'mxPoint' && as === 'targetPoint') geo.targetPoint = pt(child)
    else if (child.name === 'mxPoint' && as === 'offset') geo.offset = pt(child)
    else if (child.name === 'Array' && as === 'points') geo.points = elements(child, 'mxPoint').map(pt)
    else if (child.name === 'mxRectangle' && as === 'alternateBounds') geo.alternate = { w: n0(child.attrs.width), h: n0(child.attrs.height) }
  }
  return geo
}

const META_ATTRS = new Set(['id', 'label', 'placeholders', 'link', 'tooltip', 'linkTarget'])

function readCells(model: XmlElement): RawCell[] {
  const root = firstElement(model, 'root')
  if (!root) return []
  const out: RawCell[] = []
  let order = 0
  for (const child of elements(root)) {
    let cell: XmlElement
    let wrapper: XmlElement | null = null
    if (child.name === 'UserObject' || child.name === 'object') { wrapper = child; cell = firstElement(child, 'mxCell') ?? { name: 'mxCell', attrs: {}, children: [] } }
    else if (child.name === 'mxCell') cell = child
    else continue
    const id = wrapper?.attrs.id ?? cell.attrs.id
    if (!id) continue
    const data: Record<string, string> = {}
    if (wrapper) for (const [k, v] of Object.entries(wrapper.attrs)) if (!META_ATTRS.has(k)) data[k] = v
    out.push({
      id,
      parent: cell.attrs.parent ?? null,
      value: wrapper ? wrapper.attrs.label ?? '' : cell.attrs.value ?? '',
      style: parseStyle(cell.attrs.style),
      vertex: cell.attrs.vertex === '1',
      edge: cell.attrs.edge === '1',
      source: cell.attrs.source ?? null,
      target: cell.attrs.target ?? null,
      visible: cell.attrs.visible !== '0',
      collapsed: cell.attrs.collapsed === '1',
      geo: readGeometry(cell),
      data,
      ...(wrapper?.attrs.link ? { link: wrapper.attrs.link } : {}),
      ...(wrapper?.attrs.tooltip ? { tooltip: wrapper.attrs.tooltip } : {}),
      placeholders: wrapper?.attrs.placeholders === '1',
      order: order++,
    })
  }
  return out
}

// ---- style → keys

/** Built-in draw.io shape names → the shape Block's kinds. */
const KIND_OF: Record<string, ShapeKind> = {
  rectangle: 'rect', label: 'rect', rect: 'rect', partialRectangle: 'rect', ext: 'rect',
  ellipse: 'ellipse', doubleEllipse: 'ellipse',
  rhombus: 'diamond', triangle: 'triangle', hexagon: 'hexagon', parallelogram: 'parallelogram', trapezoid: 'trapezoid',
  cylinder: 'cylinder', cylinder2: 'cylinder', cylinder3: 'cylinder', datastore: 'cylinder', cloud: 'cloud', document: 'document', process: 'process',
  card: 'card', note: 'note', note2: 'note', callout: 'callout', cross: 'cross', step: 'step',
  singleArrow: 'arrow', doubleArrow: 'double_arrow', line: 'line', text: 'text',
}

const DIRECTIONS: Record<string, 'e' | 's' | 'w' | 'n'> = { east: 'e', south: 's', west: 'w', north: 'n' }

function colorKey(map: Map<string, string>, key: string): string | null {
  const c = color(map, key)
  if (c === null) return null
  return c === 'default' ? 'auto' : c
}

function labelOf(cell: RawCell, map: Map<string, string>, report: ImportReport): LabelText {
  if (flag(map, 'noLabel')) return plainLabel('')
  let value = cell.value
  if (cell.placeholders || flag(map, 'placeholders')) value = fillPlaceholders(value, { id: cell.id, ...cell.data })
  const label = map.get('html') === '1' ? htmlLabel(value) : plainLabel(value)
  if (label.complex) report.add('complex_html', 'Complex HTML labels (lists, tables, images…) imported as plain text', firstLine(label.text) || cell.id)
  else if (label.partial) report.add('rich_label', 'Formatting on part of a label (bold, color…) not imported', firstLine(label.text) || cell.id)
  return label
}

/** An SVG body larger than this does not fit a Block's 64 KiB configuration with its other keys. */
const SVG_BODY_LIMIT = 48 * 1024
const hexOr = (c: string | null, fallback: string) => (c && /^#[0-9a-f]{6}$/.test(c) ? c : fallback)

const firstLine = (text: string) => text.split('\n').find((l) => l.trim())?.trim().slice(0, 256) ?? ''

/** Text keys of a label (`fontStyle` bits: 1 bold, 2 italic, 4 underline). */
function textKeys(label: LabelText, map: Map<string, string>): Record<string, Json> {
  const out: Record<string, Json> = {}
  if (label.text) out.text = label.text
  out.font_size = num(map, 'fontSize', 12)
  const bits = num(map, 'fontStyle', 0)
  if (label.bold || bits & 1) out.bold = true
  if (label.italic || bits & 2) out.italic = true
  if (label.underline || bits & 4) out.underline = true
  const fc = label.color ?? colorKey(map, 'fontColor')
  if (fc && fc !== 'none') out.font_color = fc
  const align = map.get('align')
  out.align = align === 'left' || align === 'right' ? align : 'center'
  const valign = map.get('verticalAlign')
  out.valign = valign === 'top' || valign === 'bottom' ? valign : 'middle'
  const lp = map.get('labelPosition')
  const vlp = map.get('verticalLabelPosition')
  if (vlp === 'top' || vlp === 'bottom') out.label_position = vlp
  else if (lp === 'left' || lp === 'right') out.label_position = lp
  if (map.get('whiteSpace') !== 'wrap') out.wrap = false
  const bg = colorKey(map, 'labelBackgroundColor')
  if (bg && bg !== 'none') out.label_fill = bg
  return out
}

/** Look keys shared by every shape: fill, stroke, width, dash, opacity, rounding, direction, flips. */
function lookKeys(map: Map<string, string>, kind: ShapeKind, rect: Rect, report: ImportReport, id: string): Record<string, Json> {
  const out: Record<string, Json> = { shape: kind }
  out.fill = colorKey(map, 'fillColor') ?? 'auto'
  out.stroke = colorKey(map, 'strokeColor') ?? 'auto'
  out.stroke_width = num(map, 'strokeWidth', 1)
  if (flag(map, 'dashed')) out.dash = dashOf(map)
  const opacity = num(map, 'opacity', 100)
  if (opacity < 100) out.opacity = Math.max(0, opacity) / 100
  if (flag(map, 'rounded')) {
    const polygonal = kind !== 'rect' && kind !== 'process' && kind !== 'text'
    const arc = num(map, 'arcSize', polygonal ? 20 : 15)
    out.radius = Math.round((flag(map, 'absoluteArcSize') || polygonal ? arc / 2 : (Math.min(rect.w, rect.h) * arc) / 100) * 100) / 100
  }
  const dir = DIRECTIONS[map.get('direction') ?? '']
  if (dir && dir !== 'e') out.direction = dir
  if (flag(map, 'flipH')) out.flip_h = true
  if (flag(map, 'flipV')) out.flip_v = true
  if (map.has('size')) out.shape_size = num(map, 'size', 0)
  const gradient = color(map, 'gradientColor')
  if (gradient && gradient !== 'none') report.add('gradient', 'Gradients imported as the fill color', id)
  if (flag(map, 'shadow')) report.add('shadow', 'Shadows not imported', id)
  if (flag(map, 'sketch') || flag(map, 'comic')) report.add('sketch', 'Sketch / comic style imported as plain lines', id)
  if (flag(map, 'glass')) report.add('glass', 'Glass effect not imported', id)
  return out
}

function dashOf(map: Map<string, string>): 'dashed' | 'dotted' {
  const pattern = (map.get('dashPattern') ?? '').trim().split(/[\s,]+/).map(Number).filter(Number.isFinite)
  return pattern.length > 0 && pattern[0] <= 2 ? 'dotted' : 'dashed'
}

/** draw.io's `points=[[x,y,(perimeter,dx,dy)],…]`: the shape's own connection points. */
function ownPoints(map: Map<string, string>): AnchorDef[] {
  const raw = map.get('points')
  if (!raw) return []
  try {
    const list = JSON.parse(raw) as unknown
    if (!Array.isArray(list)) return []
    const out: AnchorDef[] = []
    for (const p of list) {
      if (!Array.isArray(p) || typeof p[0] !== 'number' || typeof p[1] !== 'number') continue
      const dx = typeof p[3] === 'number' ? p[3] : 0
      const dy = typeof p[4] === 'number' ? p[4] : 0
      out.push({ id: anchorIdOf(p[0], p[1], dx, dy), x: p[0], y: p[1], ...(dx ? { dx } : {}), ...(dy ? { dy } : {}) })
    }
    return cleanAnchors(out) ?? []
  } catch {
    return []
  }
}

const r4 = (v: number) => Math.round(v * 10000) / 10000
function anchorIdOf(x: number, y: number, dx = 0, dy = 0): string {
  return `xy_${r4(x)}_${r4(y)}${dx || dy ? `_d${Math.round(dx * 100) / 100}_${Math.round(dy * 100) / 100}` : ''}`.slice(0, 64)
}

/** Library shapes draw.io draws in script, approximated by a basic outline (their inner icons are lost). */
const APPROXIMATE: Record<string, Record<string, Json>> = {
  'mxgraph.bpmn.task': { shape: 'rect', radius: 10 }, 'mxgraph.bpmn.task2': { shape: 'rect', radius: 10 },
  'mxgraph.bpmn.event': { shape: 'ellipse' }, 'mxgraph.bpmn.gateway2': { shape: 'diamond' }, 'mxgraph.bpmn.conversation': { shape: 'hexagon' },
  'mxgraph.bpmn.data': { shape: 'note' }, 'mxgraph.bpmn.data2': { shape: 'note' },
  'mxgraph.flowchart.process': { shape: 'rect' }, 'mxgraph.aws4.group': { shape: 'rect' }, 'mxgraph.aws4.groupCenter': { shape: 'rect' },
  'mxgraph.basic.rect': { shape: 'rect' }, 'mxgraph.basic.ellipse': { shape: 'ellipse' },
}

// ---- images

const IMAGE_EXT: Record<string, string> = { 'image/png': 'png', 'image/jpeg': 'jpg', 'image/gif': 'gif', 'image/svg+xml': 'svg', 'image/webp': 'webp', 'image/bmp': 'bmp' }

/** An image reference of a style (`image=…`): a data URI (draw.io writes base64 without `;base64`) or a URL. */
function imageOf(value: string, key: string): PlanImage | null {
  const v = value.trim()
  if (!v) return null
  const data = /^data:([^,;]+)((?:;[^,]*)*),(.*)$/s.exec(v)
  if (data) {
    const mediaType = data[1].toLowerCase()
    const payload = data[3]
    let bytes: Uint8Array
    try {
      bytes = data[2].includes('base64') || /^[A-Za-z0-9+/=\s]+$/.test(payload) ? base64ToBytes(payload) : new TextEncoder().encode(decodeURIComponent(payload))
    } catch {
      return null
    }
    return { key, name: `drawio-${key}.${IMAGE_EXT[mediaType] ?? 'bin'}`, mediaType, bytes }
  }
  const url = /^https?:\/\//i.test(v) ? v : v.startsWith('img/') ? `https://app.diagrams.net/${v}` : null
  if (!url) return null
  const ext = /\.(png|jpe?g|gif|svg|webp)(?:[?#]|$)/i.exec(url)?.[1]?.toLowerCase()
  const mediaType = ext === 'svg' ? 'image/svg+xml' : ext === 'jpg' || ext === 'jpeg' ? 'image/jpeg' : ext ? `image/${ext}` : 'image/png'
  return { key, name: decodeURIComponent(url.split('/').pop()?.split(/[?#]/)[0] || `drawio-${key}`), mediaType, url }
}

// ---- the conversion

interface Ctx {
  cells: Map<string, RawCell>
  children: Map<string, RawCell[]>
  report: ImportReport
  options: ConvertOptions
  abs: Map<string, Rect>
  items: PlanItem[]
  byKey: Map<string, PlanItem>
  /** What each item's outline is (line ends meet it): outline and anchors. */
  forms: Map<string, { shape: Shape; anchors: AnchorDef[] }>
  images: PlanImage[]
  layerOf: Map<string, { name: string; locked: boolean }>
  layers: number
  edgeLabels: Map<string, RawCell[]>
  styleMap: Map<string, Map<string, string>>
}

const round2 = (v: number) => Math.round(v * 100) / 100
const roundRect = (r: Rect): Rect => ({ x: round2(r.x), y: round2(r.y), w: Math.max(1, round2(r.w)), h: Math.max(1, round2(r.h)) })

function styleMapOf(ctx: Ctx, cell: RawCell): Map<string, string> {
  let map = ctx.styleMap.get(cell.id)
  if (!map) { map = resolveStyle(cell.style, cell.edge); ctx.styleMap.set(cell.id, map) }
  return map
}

/** The absolute rectangle of a vertex (its parents' origins added; a relative child placed in its parent). */
function absOf(ctx: Ctx, cell: RawCell): Rect {
  const known = ctx.abs.get(cell.id)
  if (known) return known
  const geo = cell.geo ?? { x: 0, y: 0, w: 0, h: 0, relative: false, points: [] }
  const parent = cell.parent ? ctx.cells.get(cell.parent) : undefined
  const origin = parent && parent.vertex ? absOf(ctx, parent) : { x: 0, y: 0, w: 0, h: 0 }
  const size = cell.collapsed && geo.alternate ? geo.alternate : { w: geo.w, h: geo.h }
  let x = origin.x + geo.x
  let y = origin.y + geo.y
  if (geo.relative && parent?.vertex) { x = origin.x + geo.x * origin.w + (geo.offset?.x ?? 0); y = origin.y + geo.y * origin.h + (geo.offset?.y ?? 0) }
  const rect = { x, y, w: size.w, h: size.h }
  ctx.abs.set(cell.id, rect)
  return rect
}

/** The origin edge points are relative to: the edge's parent vertex, or the page. */
function edgeOrigin(ctx: Ctx, cell: RawCell): Point {
  const parent = cell.parent ? ctx.cells.get(cell.parent) : undefined
  return parent?.vertex ? absOf(ctx, parent) : { x: 0, y: 0 }
}

function meta(ctx: Ctx, cell: RawCell, extra: Record<string, Json> = {}): Record<string, Json> {
  const layer = ctx.layerOf.get(cell.id)
  return { drawio: { id: cell.id, style: cell.style.raw, ...(layer && ctx.layers > 1 ? { layer: layer.name } : {}), ...extra } }
}

function metadataKeys(cell: RawCell): Record<string, Json> {
  const out: Record<string, Json> = {}
  if (cell.link) out.link = cell.link
  if (cell.tooltip) out.tooltip = cell.tooltip
  const data = Object.fromEntries(Object.entries(cell.data).filter(([, v]) => typeof v === 'string'))
  if (Object.keys(data).length > 0) out.data = data
  return out
}

function push(ctx: Ctx, item: PlanItem) {
  ctx.items.push(item)
  ctx.byKey.set(item.key, item)
}

/** A shape item for a vertex drawn as one of the basic kinds or an SVG body. */
function shapeItem(ctx: Ctx, cell: RawCell, map: Map<string, string>, parent: string | null, rect: Rect, key = cell.id, overrides: Record<string, Json> = {}, svg?: { body: SvgBody; anchors: AnchorDef[] }): PlanItem {
  const label = labelOf(cell, map, ctx.report)
  const textStyle = cell.style.names.includes('text') && !cell.style.keys.has('shape')
  const kind: ShapeKind = (overrides.shape as ShapeKind | undefined) ?? (svg ? 'svg' : textStyle ? 'text' : KIND_OF[map.get('shape') ?? 'rectangle'] ?? 'rect')
  const config: Record<string, Json> = {
    ...lookKeys(map, kind, rect, ctx.report, cell.id),
    ...textKeys(label, map),
    ...metadataKeys(cell),
    ...overrides,
  }
  if (svg) config.svg = svg.body as unknown as Json
  if (map.get('shape') === 'partialRectangle' || map.get('shape') === 'tableRow') {
    if (['top', 'left', 'bottom', 'right'].some((side) => map.get(side) !== '0')) ctx.report.add('partial_border', 'Rectangles with only some borders drawn (table rows, cells) imported without borders', cell.id)
    config.stroke = 'none'
  }
  // own connection points: a stencil's named ports, then draw.io's `points`; none where the outline already has one
  const outlineAnchors = defaultAnchors(shapeOutline(readLook({ ...config })))
  const anchors: AnchorDef[] = []
  for (const a of [...(svg?.anchors ?? []), ...ownPoints(map)]) {
    const taken = [...outlineAnchors, ...anchors].some((b) => Math.abs(b.x - a.x) < 0.002 && Math.abs(b.y - a.y) < 0.002 && !b.dx && !b.dy && !a.dx && !a.dy)
    if (!taken || (svg?.anchors.includes(a) && !outlineAnchors.some((b) => b.id === a.id))) anchors.push(a)
  }
  if (anchors.length > 0) config.anchors = anchors as unknown as Json
  Object.assign(config, meta(ctx, cell))
  const item: PlanItem = { key, parent, type: 'shape', rect, rotation: normalizeRotation(num(map, 'rotation', 0)), config, ...(firstLine(label.text) ? { title: firstLine(label.text) } : {}) }
  return item
}

/** The outline and anchors an item offers to line ends. */
function formOf(item: PlanItem): { shape: Shape; anchors: AnchorDef[] } {
  if (item.type === 'shape') {
    const look = readLook(item.config as Record<string, unknown>)
    const shape = shapeOutline(look)
    return { shape, anchors: [...shapeAnchors(item.config as Record<string, unknown>, shape)] }
  }
  return { shape: 'rect', anchors: [...RECT_ANCHORS] }
}

async function vertexItems(ctx: Ctx, cell: RawCell, parent: string | null, locked: boolean): Promise<void> {
  const map = styleMapOf(ctx, cell)
  const rect = roundRect(absOf(ctx, cell))
  const kids = (ctx.children.get(cell.id) ?? []).filter((c) => c.vertex || c.edge)
  const shapeName = map.get('shape') ?? 'rectangle'
  const lock = locked || flag(map, 'locked')
  if (cell.collapsed) ctx.report.add('collapsed', 'Collapsed containers imported expanded', cell.id)
  if (shapeName === 'table' || shapeName === 'tableRow') ctx.report.add('table', 'draw.io tables imported as groups of shapes (not as table data)', cell.id)

  // a container: a group, a backing frame (swimlane) or shape, then the children
  if (kids.length > 0) {
    const rotation = normalizeRotation(num(map, 'rotation', 0))
    if (rotation) ctx.report.add('group_rotation', 'Rotation of containers not imported', cell.id)
    const label = labelOf(cell, map, ctx.report)
    // the group draws nothing: its look and label are its first child's
    const group: PlanItem = { key: cell.id, parent, type: 'group', rect, rotation: 0, config: {}, ...(lock ? { locked: true } : {}) }
    push(ctx, group)
    // partial borders (table rows and cells) are not drawn
    const partial = shapeName === 'tableRow' || shapeName === 'partialRectangle'
    const visible = (colorKey(map, 'fillColor') ?? 'auto') !== 'none' || (!partial && (colorKey(map, 'strokeColor') ?? 'auto') !== 'none') || label.text !== ''
    const collapsedSize: Record<string, Json> = cell.collapsed && cell.geo ? { collapsed: { w: cell.geo.w, h: cell.geo.h } } : {}
    if (shapeName === 'swimlane' || shapeName === 'table') {
      const horizontal = map.get('horizontal') !== '0'
      const stroke = colorKey(map, 'strokeColor') ?? 'auto'
      const fill = colorKey(map, 'fillColor')
      // a table's fill covers its body too
      const body = shapeName === 'table' ? fill : colorKey(map, 'swimlaneFillColor')
      const config: Record<string, Json> = {
        color: stroke,
        header: num(map, 'startSize', 23),
        ...(horizontal ? {} : { orientation: 'vertical' }),
        ...(fill && fill !== 'none' ? { header_fill: fill } : { header_fill: 'none' }),
        ...(body && body !== 'none' ? { fill: body } : {}),
        ...textKeys(label, map),
        stroke_width: num(map, 'strokeWidth', 1),
        ...(flag(map, 'dashed') ? { dash: dashOf(map) } : {}),
        ...(flag(map, 'rounded') ? { radius: num(map, 'arcSize', 15) / 2 } : {}),
        ...metadataKeys(cell),
        ...meta(ctx, cell, { container: true, ...collapsedSize }),
      }
      delete config.label_position
      delete config.text
      push(ctx, { key: `${cell.id}~frame`, parent: cell.id, type: 'frame', rect, rotation: 0, config, ...(lock ? { locked: true } : {}), ...(firstLine(label.text) ? { title: firstLine(label.text) } : {}) })
    } else if (visible) {
      const backing = shapeItem(ctx, cell, map, cell.id, rect, `${cell.id}~shape`, partial ? { stroke: 'none' } : {})
      backing.config.drawio = { ...(backing.config.drawio as Record<string, Json>), container: true, ...collapsedSize }
      if (lock) backing.locked = true
      push(ctx, backing)
    } else {
      group.config = meta(ctx, cell, { container: true, ...collapsedSize })
    }
    for (const kid of kids) await cellItems(ctx, kid, cell.id, lock)
    return
  }

  const item = await leafItem(ctx, cell, map, parent, rect)
  if (!item) return
  for (const i of Array.isArray(item) ? item : [item]) { if (lock) i.locked = true; push(ctx, i) }
}

/** One vertex without children: a shape, an SVG shape, an image, or a picture with text (`label`, an image with a caption). */
async function leafItem(ctx: Ctx, cell: RawCell, map: Map<string, string>, parent: string | null, rect: Rect): Promise<PlanItem | PlanItem[] | null> {
  const shapeName = map.get('shape') ?? 'rectangle'
  const imageRef = map.get('image')
  if (shapeName === 'image' || (imageRef && (shapeName === 'label' || map.has('imageWidth')))) {
    const image = imageRef ? imageOf(imageRef, cell.id) : null
    const label = labelOf(cell, map, ctx.report)
    const plain = shapeName === 'image'
    // the picture: the whole box of an image shape, the icon spot of a `label` shape (left, vertically centred)
    const iw = num(map, 'imageWidth', 42), ih = num(map, 'imageHeight', 42)
    const pictureRect = plain ? rect : roundRect({ x: rect.x + num(map, 'spacingLeft', 2) / 2 + 4, y: rect.y + (rect.h - ih) / 2, w: iw, h: ih })
    const fallbackConfig: Record<string, Json> = { shape: 'rect', fill: 'none', stroke: 'auto', stroke_width: 1, dash: 'dashed', text: ctx.options.imageText?.(image?.url ?? null) ?? (image?.url ? `Image: ${image.url}` : 'Image'), font_size: 11, ...(image?.url ? { link: image.url } : {}), ...meta(ctx, cell) }
    const picture: PlanItem = image
      ? { key: plain && !label.text ? cell.id : `${cell.id}~image`, parent, type: 'asset', rect: pictureRect, rotation: normalizeRotation(num(map, 'rotation', 0)), config: { ...metadataKeys(cell), ...meta(ctx, cell) }, image, fallback: { config: fallbackConfig, ...(firstLine(label.text) ? { title: firstLine(label.text) } : {}) } }
      : { key: plain && !label.text ? cell.id : `${cell.id}~image`, parent, type: 'shape', rect: pictureRect, rotation: 0, config: fallbackConfig }
    if (image) ctx.images.push(image)
    else ctx.report.add('image_missing', 'Images that could not be fetched replaced by dashed boxes with a link', cell.id)
    if (plain && !label.text) return picture
    // with text: a group of the picture and the text (and the label shape's box)
    const group: PlanItem = { key: cell.id, parent, type: 'group', rect, rotation: 0, config: {} }
    picture.parent = cell.id
    const parts: PlanItem[] = [group]
    if (!plain) {
      const box = shapeItem(ctx, cell, map, cell.id, rect, `${cell.id}~box`, { text: '' })
      parts.push(box)
    }
    parts.push(picture)
    const textRect = plain ? rect : roundRect({ x: pictureRect.x + pictureRect.w + 4, y: rect.y, w: Math.max(8, rect.x + rect.w - pictureRect.x - pictureRect.w - 8), h: rect.h })
    const text = shapeItem(ctx, cell, map, cell.id, textRect, `${cell.id}~text`, { shape: 'text', fill: 'none', stroke: 'none', ...(plain ? {} : { label_position: 'inside' }) })
    delete text.config.anchors
    parts.push(text)
    return parts
  }
  // an SVG body: an inline stencil, a library shape, a built-in drawn in script
  const stencil = /^stencil\((.*)\)$/s.exec(shapeName)
  if (stencil || shapeName.startsWith('mxgraph.')) {
    const resolved = stencil ? await ctx.options.inlineStencil?.(stencil[1]) ?? null : await ctx.options.stencil?.(shapeName) ?? null
    if (resolved) {
      for (const u of resolved.unsupported) ctx.report.add(`stencil:${u}`, `Stencil instruction ${u} not imported`, cell.id)
      if (resolved.svg.body.length <= SVG_BODY_LIMIT) return shapeItem(ctx, cell, map, parent, rect, cell.id, {}, { body: resolved.svg, anchors: cleanAnchors(resolved.anchors) ?? [] })
      // too large for a Block's configuration: the picture alone, as an image (no recolouring, no anchors of its own)
      ctx.report.add('svg_too_large', 'SVG shapes over the configuration limit imported as images (cannot be recolored)', cell.id)
      const document = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${resolved.svg.viewBox}" preserveAspectRatio="none" style="--aiws-shape-fill:${hexOr(colorKey(map, 'fillColor'), '#ffffff')};--aiws-shape-stroke:${hexOr(colorKey(map, 'strokeColor'), '#000000')};--aiws-shape-stroke-width:${num(map, 'strokeWidth', 1)}">${resolved.svg.body}</svg>`
      const image: PlanImage = { key: cell.id, name: `drawio-${cell.id}.svg`, mediaType: 'image/svg+xml', bytes: new TextEncoder().encode(document) }
      ctx.images.push(image)
      const fallback = shapeItem(ctx, cell, map, parent, rect, cell.id, { shape: 'rect' })
      return { key: cell.id, parent, type: 'asset', rect, rotation: normalizeRotation(num(map, 'rotation', 0)), config: { ...metadataKeys(cell), ...meta(ctx, cell) }, image, fallback: { config: fallback.config, ...(fallback.title ? { title: fallback.title } : {}) } }
    }
    // shapes draw.io draws in script: the nearest basic outline where there is one
    const near = APPROXIMATE[shapeName]
    if (!stencil && near) {
      ctx.report.add('approximated', 'Shapes draw.io draws in script imported as the nearest basic shape (inner icons not imported)', shapeName)
      return shapeItem(ctx, cell, map, parent, rect, cell.id, near)
    }
    ctx.report.add(stencil ? 'stencil_failed' : 'library_shape', stencil ? 'Embedded stencils that could not be read imported as rectangles' : 'Library shapes that are not built in (mostly drawn in script by draw.io) imported as a rectangle with the name', stencil ? cell.id : shapeName)
    const item = shapeItem(ctx, cell, map, parent, rect, cell.id, { shape: 'rect' })
    if (!item.config.text) item.config.text = shapeName.replace(/^mxgraph\./, '')
    return item
  }
  const builtin = builtinSvg(shapeName)
  if (builtin) return shapeItem(ctx, cell, map, parent, rect, cell.id, {}, { body: builtin, anchors: [] })
  if (!KIND_OF[shapeName]) ctx.report.add('unknown_shape', 'Unknown shapes imported as rectangles', shapeName)
  return shapeItem(ctx, cell, map, parent, rect)
}

async function cellItems(ctx: Ctx, cell: RawCell, parent: string | null, locked: boolean): Promise<void> {
  if (!cell.visible) { ctx.report.add('hidden_cell', 'Hidden objects not imported', cell.id); return }
  if (cell.edge) {
    const map = styleMapOf(ctx, cell)
    push(ctx, { key: cell.id, parent, type: 'connector', rect: { x: 0, y: 0, w: 0, h: 0 }, rotation: 0, config: {}, ...(locked || flag(map, 'locked') ? { locked: true } : {}) })
    return
  }
  if (cell.vertex) await vertexItems(ctx, cell, parent, locked)
}

// ---- edges

const CAPS: Record<string, [string, string]> = {
  classic: ['triangle', 'triangle_outline'], classicThin: ['triangle', 'triangle_outline'], block: ['triangle', 'triangle_outline'], blockThin: ['triangle', 'triangle_outline'],
  open: ['arrow', 'arrow'], openThin: ['arrow', 'arrow'], openAsync: ['arrow', 'arrow'], async: ['triangle', 'triangle_outline'],
  oval: ['circle', 'circle_outline'], circle: ['circle', 'circle_outline'], halfCircle: ['circle_outline', 'circle_outline'],
  diamond: ['diamond', 'diamond_outline'], diamondThin: ['diamond', 'diamond_outline'],
  dash: ['bar', 'bar'], baseDash: ['bar', 'bar'], cross: ['bar', 'bar'],
  ERone: ['cardinality_one', 'cardinality_one'], ERmandOne: ['cardinality_exactly_one', 'cardinality_exactly_one'], ERmany: ['cardinality_many', 'cardinality_many'],
  ERoneToMany: ['cardinality_one_or_many', 'cardinality_one_or_many'], ERzeroToOne: ['cardinality_zero_or_one', 'cardinality_zero_or_one'], ERzeroToMany: ['cardinality_zero_or_many', 'cardinality_zero_or_many'],
  none: ['none', 'none'],
}

function capOf(map: Map<string, string>, which: 'start' | 'end', report: ImportReport): string {
  const name = map.get(`${which}Arrow`) ?? (which === 'end' ? 'classic' : 'none')
  if (!name) return 'none'
  const pair = CAPS[name]
  if (!pair) { report.add('cap', 'Unsupported line ends imported as plain arrows', name); return 'arrow' }
  return pair[map.get(`${which}Fill`) === '0' ? 1 : 0]
}

function routeOf(map: Map<string, string>): { route: Route; axis: 'h' | 'v' | 'auto' } {
  const style = map.get('edgeStyle') ?? ''
  const vertical = map.get('elbow') === 'vertical'
  if (flag(map, 'curved') || style === 'entityRelationEdgeStyle') return { route: 'curve', axis: style === 'entityRelationEdgeStyle' ? 'h' : 'auto' }
  switch (style) {
    case 'elbowEdgeStyle': return { route: 'elbow', axis: vertical ? 'v' : 'h' }
    case 'sideToSideEdgeStyle': return { route: 'elbow', axis: 'h' }
    case 'topToBottomEdgeStyle': return { route: 'elbow', axis: 'v' }
    case 'orthogonalEdgeStyle': case 'segmentEdgeStyle': case 'loopEdgeStyle': return { route: 'elbow', axis: 'auto' }
    default: return { route: 'straight', axis: 'auto' }
  }
}

function targetGeom(ctx: Ctx, key: string): TargetGeom | null {
  const item = ctx.byKey.get(key)
  if (!item || item.type === 'connector') return null
  let form = ctx.forms.get(key)
  if (!form) { form = formOf(item); ctx.forms.set(key, form) }
  return { rect: item.rect, rotation: item.rotation, shape: form.shape, anchors: form.anchors }
}

/** The anchor at a fraction of the target (draw.io constraint), added to the target's own anchors when missing. */
function constraintAnchor(ctx: Ctx, key: string, x: number, y: number, dx: number, dy: number, perimeter: boolean): string {
  const form = ctx.forms.get(key)!
  let px = x, py = y
  if (perimeter && form.shape !== 'rect') {
    const t: TargetGeom = { rect: { x: 0, y: 0, w: 1, h: 1 }, rotation: 0, shape: form.shape, anchors: [] }
    const c = { x: 0.5, y: 0.5 }
    if (Math.hypot(x - 0.5, y - 0.5) > 1e-6) { const p = anchorWorld(t, { kind: 'named', id: '\u0000' }, { x: c.x + (x - c.x) * 4, y: c.y + (y - c.y) * 4 }); px = p.x; py = p.y }
  }
  // the outline's compass anchors first: a spot they cover keeps its compass id
  const at = (a: AnchorDef) => Math.abs(a.x - px) < 0.002 && Math.abs(a.y - py) < 0.002 && Math.abs((a.dx ?? 0) - dx) < 0.01 && Math.abs((a.dy ?? 0) - dy) < 0.01
  const match = form.anchors.find((a) => COMPASS_IDS.has(a.id) && at(a)) ?? form.anchors.find(at)
  if (match) return match.id
  const anchor: AnchorDef = { id: anchorIdOf(px, py, dx, dy), x: Math.min(1, Math.max(0, r4(px))), y: Math.min(1, Math.max(0, r4(py))), ...(dx ? { dx } : {}), ...(dy ? { dy } : {}) }
  form.anchors = [anchor, ...form.anchors.filter((a) => a.id !== anchor.id)]
  const item = ctx.byKey.get(key)!
  if (item.type === 'shape') {
    const own = cleanAnchors(item.config.anchors) ?? []
    item.config.anchors = [...own.filter((a) => a.id !== anchor.id), anchor] as unknown as Json
  } else ctx.report.add('anchor_on_box', 'Connection points at non-default spots on images, boxes or groups imported as the nearest anchor', key)
  return item.type === 'shape' ? anchor.id : nearestAnchor(form.anchors.filter((a) => a.id !== anchor.id), { x: px, y: py }).id
}

const COMPASS_IDS = new Set<string>(COMPASS)

function nearestAnchor(anchors: readonly AnchorDef[], p: Point): AnchorDef {
  let best = anchors[0]
  let bestD = Infinity
  for (const a of anchors) { const d = Math.hypot(a.x - p.x, a.y - p.y); if (d < bestD - 1e-9) { best = a; bestD = d } }
  return best
}

/** A floating end: where draw.io would meet the outline (straight and curved lines), or the middle of the side
 * an orthogonal line leaves by (`axis`, the target's `portConstraint`). */
function floatingAnchor(ctx: Ctx, key: string, toward: Point, route: Route, axis: 'h' | 'v' | 'auto', portConstraint: string | undefined): string {
  const t = targetGeom(ctx, key)!
  const anchors = t.anchors ?? defaultAnchors(t.shape)
  const c = centerOf(t.rect)
  if (route === 'elbow' || axis !== 'auto') {
    let dirAxis: 'h' | 'v' = axis === 'auto' ? (Math.abs(toward.x - c.x) / Math.max(t.rect.w, 1) >= Math.abs(toward.y - c.y) / Math.max(t.rect.h, 1) ? 'h' : 'v') : axis
    const pc = portConstraint ?? ''
    if (/eastwest|^east$|^west$/.test(pc)) dirAxis = 'h'
    else if (/northsouth|^north$|^south$/.test(pc)) dirAxis = 'v'
    const side = pc === 'east' ? 'e' : pc === 'west' ? 'w' : pc === 'north' ? 'n' : pc === 'south' ? 's'
      : dirAxis === 'h' ? (toward.x >= c.x ? 'e' : 'w') : (toward.y >= c.y ? 's' : 'n')
    if (findAnchor(anchors, side)) return side
  }
  const local = { x: (anchorWorld({ ...t, anchors }, { kind: 'named', id: '\u0000' }, toward).x - t.rect.x) / Math.max(t.rect.w, 1e-9), y: (anchorWorld({ ...t, anchors }, { kind: 'named', id: '\u0000' }, toward).y - t.rect.y) / Math.max(t.rect.h, 1e-9) }
  return nearestAnchor(anchors.filter((a) => !a.dx && !a.dy), local).id
}

function measure(text: string, font: number): { w: number; h: number } {
  const lines = text.split('\n')
  let widest = 0
  for (const line of lines) { let w = 0; for (const ch of line) w += /[⺀-￿]/.test(ch) ? font : font * 0.6; widest = Math.max(widest, w) }
  return { w: Math.ceil(widest + 8), h: Math.ceil(lines.length * font * 1.3 + 4) }
}

function edgeItem(ctx: Ctx, item: PlanItem) {
  const cell = ctx.cells.get(item.key)!
  const map = styleMapOf(ctx, cell)
  const origin = edgeOrigin(ctx, cell)
  const geo = cell.geo ?? { x: 0, y: 0, w: 0, h: 0, relative: true, points: [] }
  const waypoints = geo.points.map((p) => ({ x: p.x + origin.x, y: p.y + origin.y }))
  const { route, axis } = routeOf(map)
  const bindable = (id: string | null): string | null => {
    if (!id) return null
    const other = ctx.cells.get(id)
    if (!other) return null
    if (other.edge) { ctx.report.add('edge_to_edge', 'Line ends attached to another line changed to free ends', cell.id); return null }
    return ctx.byKey.has(id) && ctx.byKey.get(id)!.type !== 'connector' ? id : null
  }
  const sKey = bindable(cell.source)
  const tKey = bindable(cell.target)
  for (const key of [sKey, tKey]) if (key) targetGeom(ctx, key)
  const centre = (key: string) => centerOf(ctx.byKey.get(key)!.rect)
  const freeAt = (point: Point | undefined, terminal: string | null): Point => (point ? { x: point.x + origin.x, y: point.y + origin.y } : terminal && ctx.byKey.has(terminal) ? centre(terminal) : { x: origin.x, y: origin.y })
  const sFree = freeAt(geo.sourcePoint, cell.source)
  const tFree = freeAt(geo.targetPoint, cell.target)
  const endSpec = (which: 'start' | 'end', key: string | null): PlanEnd | null => {
    if (!key) return null
    const p = which === 'start' ? 'exit' : 'entry'
    if (map.has(`${p}X`) && map.has(`${p}Y`)) {
      return { key, anchor: constraintAnchor(ctx, key, num(map, `${p}X`, 0.5), num(map, `${p}Y`, 0.5), num(map, `${p}Dx`, 0), num(map, `${p}Dy`, 0), map.get(`${p}Perimeter`) !== '0') }
    }
    const otherKey = which === 'start' ? tKey : sKey
    const toward = (which === 'start' ? waypoints[0] : waypoints[waypoints.length - 1]) ?? (otherKey ? centre(otherKey) : which === 'start' ? tFree : sFree)
    const terminal = ctx.cells.get(key)
    return { key, anchor: floatingAnchor(ctx, key, toward, route, axis, terminal ? styleMapOf(ctx, terminal).get('portConstraint') : undefined) }
  }
  let start = endSpec('start', sKey)
  let end = endSpec('end', tKey)
  // an orthogonal line between two floating ends: stacked objects meet top to bottom, objects side by side meet side
  // to side, diagonal ones leave sideways and enter from the top or bottom (draw.io's orthogonal router)
  if (route === 'elbow' && axis === 'auto' && start && end && start.key !== end.key && !waypoints.length && !map.has('exitX') && !map.has('entryX')) {
    const a = ctx.byKey.get(start.key)!.rect
    const b = ctx.byKey.get(end.key)!.rect
    const pcS = styleMapOf(ctx, ctx.cells.get(start.key)!).get('portConstraint')
    const pcE = styleMapOf(ctx, ctx.cells.get(end.key)!).get('portConstraint')
    const ca = centerOf(a), cb = centerOf(b)
    const overlapX = Math.abs(ca.x - cb.x) < (a.w + b.w) / 2
    const overlapY = Math.abs(ca.y - cb.y) < (a.h + b.h) / 2
    const h = (from: Point, to: Point) => (to.x >= from.x ? 'e' : 'w')
    const v = (from: Point, to: Point) => (to.y >= from.y ? 's' : 'n')
    const [sSide, eSide] = overlapX && !overlapY ? [v(ca, cb), v(cb, ca)] : overlapY ? [h(ca, cb), h(cb, ca)] : [h(ca, cb), v(cb, ca)]
    const has = (key: string, id: string) => Boolean(findAnchor(ctx.forms.get(key)?.anchors ?? [], id))
    if (!pcS && has(start.key, sSide)) start = { key: start.key, anchor: sSide }
    if (!pcE && has(end.key, eSide)) end = { key: end.key, anchor: eSide }
  }
  let finalRoute = route
  // a loop on one object needs two different anchors and a route that leaves and comes back
  if (start && end && start.key === end.key) {
    if (start.anchor === end.anchor) end = { key: end.key, anchor: start.anchor === 'n' ? 'e' : 'n' }
    if (finalRoute === 'straight') finalRoute = 'elbow'
  }
  const toBinding = (e: PlanEnd | null): Binding | null => (e ? { entity_id: e.key, anchor: { kind: 'named', id: e.anchor } } : null)
  const lookup = (id: string) => targetGeom(ctx, id) ?? { broken: 'missing' as const }
  const data: ConnectorData = { start: toBinding(start), end: toBinding(end), flip: {}, route: finalRoute, controls: [], label: { t: 0.5, offset: 0 } }
  const first = routeConnector(data, [sFree, tFree], lookup, { pad: 0 })
  const s = first.start.point
  const e = first.end.point
  data.controls = waypoints.map((p) => decompose(p, s, e))
  const g = routeConnector({ ...data, controls: data.controls }, [s, e], lookup, { pad: 0 })
  // labels: the edge's own value, then its label children (the first one becomes the line's label if it has none)
  const own = labelOf(cell, map, ctx.report)
  const children = (ctx.edgeLabels.get(cell.id) ?? []).filter((c) => c.visible)
  const placeOf = (lg: RawGeo | null) => {
    const t = Math.min(1, Math.max(0, ((lg?.x ?? 0) + 1) / 2))
    const { tangent } = pointAt(g, t)
    const n = leftOf(tangent)
    const off = lg?.offset ?? { x: 0, y: 0 }
    const along = g.length > 0 ? (off.x * tangent.x + off.y * tangent.y) / g.length : 0
    return { t: Math.min(1, Math.max(0, t + along)), offset: (lg?.y ?? 0) + off.x * n.x + off.y * n.y }
  }
  let title = own.text
  let labelPos: { t: number; offset: number } | null = title ? placeOf(geo) : null
  let labelMap = map
  const extra: RawCell[] = []
  for (const child of children) {
    const childMap = styleMapOf(ctx, child)
    const text = labelOf(child, childMap, ctx.report).text
    if (!text) continue
    if (!title) { title = text; labelPos = placeOf(child.geo); labelMap = childMap } else extra.push(child)
  }
  const config: Record<string, Json> = {}
  const stroke = color(map, 'strokeColor')
  if (stroke && stroke !== 'default' && stroke !== 'none') config.stroke = stroke
  if (stroke === 'none') config.opacity = 0.1
  config.width = Math.min(24, Math.max(0.5, num(map, 'strokeWidth', 1)))
  if (flag(map, 'dashed')) config.dash = dashOf(map)
  const startCap = capOf(map, 'start', ctx.report)
  const endCap = capOf(map, 'end', ctx.report)
  if (startCap !== 'none') config.start_cap = startCap
  if (endCap !== 'arrow') config.end_cap = endCap
  if (map.get('rounded') === '0') config.rounded = false
  const opacity = num(map, 'opacity', 100)
  if (opacity < 100) config.opacity = Math.max(0.1, opacity / 100)
  if (map.get('elbow') === 'vertical' || map.get('elbow') === 'horizontal') config.elbow = map.get('elbow')!
  if (title) {
    const size = num(labelMap, 'fontSize', 11)
    config.label_size = size <= 11 ? 's' : size <= 15 ? 'm' : 'l'
    const lc = color(labelMap, 'fontColor')
    if (lc && lc !== 'default' && lc !== 'none') config.label_color = lc
    const lf = color(labelMap, 'labelBackgroundColor')
    if (lf && lf !== 'default' && lf !== 'none') config.label_fill = lf
  }
  if (map.has('startSize') || map.has('endSize')) ctx.report.add('cap_size', 'Separate line-end sizes not imported (line ends scale with the line width)', cell.id)
  if (map.has('jumpStyle') && map.get('jumpStyle') !== 'none') ctx.report.add('jump', 'Line jumps (arcs over crossings) not imported', cell.id)
  if (flag(map, 'flowAnimation')) ctx.report.add('flow_animation', 'Flow animation not imported', cell.id)
  Object.assign(config, metadataKeys(cell), meta(ctx, cell))
  const box = boxOf(s, e)
  item.rect = box.rect
  item.config = config
  if (title) item.title = title.slice(0, 256)
  item.line = { start, end, startPoint: s, endPoint: e, route: finalRoute, controls: data.controls, label: labelPos }
  // further labels: text where they are now, after the line (they do not follow it)
  const after = ctx.items.indexOf(item)
  const texts: PlanItem[] = []
  for (const child of extra) {
    ctx.report.add('extra_labels', 'Second and later labels of a line imported as text that does not follow the line', cell.id)
    const childMap = styleMapOf(ctx, child)
    const label = labelOf(child, childMap, ctx.report)
    const pos = placeOf(child.geo)
    const at = pointAt(g, pos.t)
    const n = leftOf(at.tangent)
    const p = { x: at.point.x + n.x * pos.offset, y: at.point.y + n.y * pos.offset }
    const size = measure(label.text, num(childMap, 'fontSize', 11))
    const rect = roundRect({ x: p.x - size.w / 2, y: p.y - size.h / 2, w: size.w, h: size.h })
    const text = shapeItem(ctx, child, childMap, item.parent, rect, child.id, { shape: 'text', fill: 'none', stroke: 'none', align: 'center', valign: 'middle', wrap: false })
    delete text.config.anchors
    texts.push(text)
  }
  for (const [i, t] of texts.entries()) { ctx.items.splice(after + 1 + i, 0, t); ctx.byKey.set(t.key, t) }
}

// ---- page

/** Convert one draw.io page into an import plan. */
export async function convertPage(page: DrawioPage, report: ImportReport, options: ConvertOptions = {}): Promise<PagePlan> {
  const cells = readCells(page.model)
  const ctx: Ctx = {
    cells: new Map(cells.map((c) => [c.id, c])), children: new Map(), report, options, abs: new Map(), items: [], byKey: new Map(), forms: new Map(),
    images: [], layerOf: new Map(), layers: 0, edgeLabels: new Map(), styleMap: new Map(),
  }
  for (const cell of cells) {
    if (!cell.parent) continue
    const parent = ctx.cells.get(cell.parent)
    if (cell.vertex && parent?.edge) { const list = ctx.edgeLabels.get(parent.id) ?? []; list.push(cell); ctx.edgeLabels.set(parent.id, list); continue }
    const list = ctx.children.get(cell.parent) ?? []
    list.push(cell)
    ctx.children.set(cell.parent, list)
  }
  const root = cells.find((c) => !c.parent)
  const layers = root ? (ctx.children.get(root.id) ?? []).filter((c) => !c.vertex && !c.edge) : []
  ctx.layers = layers.length
  const mark = (id: string, info: { name: string; locked: boolean }) => { ctx.layerOf.set(id, info); for (const c of ctx.children.get(id) ?? []) mark(c.id, info) }
  for (const [i, layer] of layers.entries()) {
    const info = { name: layer.value || (i === 0 ? 'Background' : `Layer ${i}`), locked: flag(parseStyle(layer.style.raw).keys, 'locked') }
    mark(layer.id, info)
    for (const c of ctx.edgeLabels.values()) for (const l of c) if (!ctx.layerOf.has(l.id) && l.parent && ctx.layerOf.has(l.parent)) ctx.layerOf.set(l.id, info)
  }
  for (const layer of layers) {
    const info = ctx.layerOf.get(layer.id)!
    if (!layer.visible) { report.add('hidden_layer', 'Hidden layers not imported', info.name); continue }
    for (const cell of ctx.children.get(layer.id) ?? []) await cellItems(ctx, cell, null, info.locked)
  }
  for (const item of [...ctx.items]) if (item.type === 'connector') edgeItem(ctx, item)
  ctx.items = ctx.items.filter((i) => i.type !== 'connector' || i.line)
  // move to the positive quadrant: content starts at (0, 0)
  const rects: Rect[] = []
  for (const item of ctx.items) {
    if (item.parent !== null) continue
    if (item.line) {
      const g = routeConnector({ start: null, end: null, flip: {}, route: item.line.route, controls: item.line.controls, label: { t: 0.5, offset: 0 } }, [item.line.startPoint, item.line.endPoint], () => ({ broken: 'missing' }), { pad: 0 })
      rects.push(g.bounds)
    } else rects.push(rotatedBounds(item.rect, item.rotation))
  }
  let x1 = Infinity, y1 = Infinity, x2 = -Infinity, y2 = -Infinity
  for (const r of rects) { x1 = Math.min(x1, r.x); y1 = Math.min(y1, r.y); x2 = Math.max(x2, r.x + r.w); y2 = Math.max(y2, r.y + r.h) }
  if (!Number.isFinite(x1)) { x1 = 0; y1 = 0; x2 = 0; y2 = 0 }
  const dx = -x1, dy = -y1
  for (const item of ctx.items) {
    item.rect = { ...item.rect, x: round2(item.rect.x + dx), y: round2(item.rect.y + dy) }
    if (item.line) { item.line.startPoint = { x: item.line.startPoint.x + dx, y: item.line.startPoint.y + dy }; item.line.endPoint = { x: item.line.endPoint.x + dx, y: item.line.endPoint.y + dy } }
  }
  const model = page.model.attrs
  const bg = model.background && model.background !== 'none' ? model.background.toLowerCase() : null
  report.pages += 1
  report.objects += ctx.items.filter((i) => i.type !== 'connector' && i.type !== 'group').length
  report.lines += ctx.items.filter((i) => i.type === 'connector').length
  report.images += ctx.images.length
  return {
    id: page.id, name: page.name,
    background: bg && /^#[0-9a-f]{6}$/.test(bg) ? bg : null,
    pageSize: model.page === '1' || model.pageWidth ? { w: n0(model.pageWidth) || 850, h: n0(model.pageHeight) || 1100 } : null,
    items: ctx.items,
    bounds: { x: 0, y: 0, w: round2(x2 - x1), h: round2(y2 - y1) },
    images: ctx.images,
  }
}
