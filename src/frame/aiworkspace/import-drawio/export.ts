/* Canvas → draw.io (draw.io 导入 TODO §2.6, D5): one free Surface becomes one `<diagram>`. Shapes, frames (as
 * swimlanes), groups (as draw.io groups, or as the container their backing frame or shape stands for), pictures
 * and connectors map back to cells; the draw.io style an import kept (`config.drawio.style`) is the starting point,
 * so keys the canvas does not know survive a round trip, and the keys the canvas owns are written from its current
 * look. Line ends become `exitX/exitY` (`entryX/entryY`) at the anchor's spot, bends become waypoints; metadata
 * (`link`, `tooltip`, `data`) becomes a `UserObject`. Blocks with no draw.io counterpart (rich text, notes, tables)
 * export as text in a box and are counted. */

import { cleanAnchors, defaultAnchors, findAnchor, RECT_ANCHORS, type AnchorDef, type Shape } from './anchors.ts'
import { compose, routeConnector, type Binding, type ConnectorData, type Control, type Route, type TargetGeom } from './connector.ts'
import type { Point, Rect } from './geometry.ts'
import { readLook, shapeAnchors, shapeOutline, type ShapeLook } from './shapes.ts'
import { parseStyle, styleString } from './style.ts'
import { el, escapeAttr, toXml, type XmlElement } from './xml.ts'

export interface ExportNode {
  id: string
  /** The parent node's id, or null on the Surface. Siblings come in stacking order. */
  parent: string | null
  kind: 'group' | 'cell'
  /** A Cell's renderer (`shape`, `frame`, `asset`, `connector`, …). */
  view?: string
  placement: { x: number; y: number; w: number; h: number; rotation?: number }
  title?: string
  config?: Record<string, unknown>
  /** A connector's keys (`start`, `end`, `flip`, `route`, `controls`, `label`). */
  line?: Record<string, unknown>
  /** The text of a Block with no draw.io counterpart (a note's body, a rich text's words). */
  text?: string
  /** A picture as a data URI (`data:<type>;base64,…`). */
  image?: string
  locked?: boolean
}

export interface ExportPage { name: string; background?: string | null; pageSize?: { w: number; h: number } | null }
export interface ExportResult { xml: string; skipped: Map<string, number> }

const fmt = (n: number) => String(Math.round(n * 100) / 100)
const color = (v: string | null | undefined, auto = 'default') => (!v ? auto : v === 'auto' ? 'default' : v)

/** The keys of a kept style the canvas owns (rewritten from the current look). */
const VERTEX_KEYS = ['shape', 'perimeter', 'fillColor', 'strokeColor', 'strokeWidth', 'dashed', 'dashPattern', 'opacity', 'rounded', 'arcSize', 'absoluteArcSize', 'direction', 'flipH', 'flipV', 'size',
  'rotation', 'fontSize', 'fontColor', 'fontStyle', 'align', 'verticalAlign', 'labelPosition', 'verticalLabelPosition', 'whiteSpace', 'html', 'labelBackgroundColor', 'points', 'locked',
  'startSize', 'horizontal', 'swimlaneFillColor', 'image', 'noLabel', 'container']
const EDGE_KEYS = ['edgeStyle', 'curved', 'rounded', 'strokeColor', 'strokeWidth', 'dashed', 'dashPattern', 'opacity', 'startArrow', 'endArrow', 'startFill', 'endFill',
  'exitX', 'exitY', 'exitDx', 'exitDy', 'exitPerimeter', 'entryX', 'entryY', 'entryDx', 'entryDy', 'entryPerimeter', 'fontSize', 'fontColor', 'labelBackgroundColor', 'elbow', 'html', 'locked', 'orthogonalLoop']

const SHAPE_STYLE: Record<string, [string, string][]> = {
  rect: [], ellipse: [['shape', 'ellipse'], ['perimeter', 'ellipsePerimeter']], diamond: [['shape', 'rhombus'], ['perimeter', 'rhombusPerimeter']],
  triangle: [['shape', 'triangle'], ['perimeter', 'trianglePerimeter']], hexagon: [['shape', 'hexagon'], ['perimeter', 'hexagonPerimeter2']],
  parallelogram: [['shape', 'parallelogram'], ['perimeter', 'parallelogramPerimeter']], trapezoid: [['shape', 'trapezoid'], ['perimeter', 'trapezoidPerimeter']],
  cylinder: [['shape', 'cylinder3'], ['boundedLbl', '1'], ['backgroundOutline', '1']], cloud: [['shape', 'cloud']], document: [['shape', 'document'], ['boundedLbl', '1']],
  process: [['shape', 'process'], ['backgroundOutline', '1']], card: [['shape', 'card']], note: [['shape', 'note'], ['backgroundOutline', '1']],
  callout: [['shape', 'callout'], ['perimeter', 'calloutPerimeter']], cross: [['shape', 'cross']], step: [['shape', 'step'], ['perimeter', 'stepPerimeter']],
  arrow: [['shape', 'singleArrow']], double_arrow: [['shape', 'doubleArrow']], line: [['shape', 'line']], text: [],
}
const DIRECTION: Record<string, string> = { s: 'south', w: 'west', n: 'north' }

/** The kept style minus the keys the canvas owns, plus `own`. */
function styleFrom(kept: unknown, owned: string[], own: [string, string | number | undefined][], keepShape = false): string {
  const style = parseStyle(typeof kept === 'string' ? kept : '')
  const keys = new Map<string, string>()
  for (const [k, v] of style.keys) if (!owned.includes(k) || (keepShape && k === 'shape')) keys.set(k, v)
  for (const [k, v] of own) if (v !== undefined && !(keepShape && k === 'shape' && keys.has('shape'))) keys.set(k, String(v))
  return styleString(keys)
}

function textKeys(look: Pick<ShapeLook, 'fontSize' | 'fontColor' | 'bold' | 'italic' | 'underline' | 'align' | 'valign' | 'labelPosition' | 'wrap' | 'labelFill'>): [string, string | number | undefined][] {
  const bits = (look.bold ? 1 : 0) | (look.italic ? 2 : 0) | (look.underline ? 4 : 0)
  const out: [string, string | number | undefined][] = [['html', 1], ['fontSize', fmt(look.fontSize)], ['fontColor', color(look.fontColor || 'auto')], ['fontStyle', bits || undefined]]
  if (look.labelPosition === 'top' || look.labelPosition === 'bottom') out.push(['verticalLabelPosition', look.labelPosition], ['labelPosition', 'center'])
  else if (look.labelPosition === 'left' || look.labelPosition === 'right') out.push(['labelPosition', look.labelPosition], ['verticalLabelPosition', 'middle'])
  out.push(['align', look.align], ['verticalAlign', look.valign], ['whiteSpace', look.wrap ? 'wrap' : undefined], ['labelBackgroundColor', look.labelFill ? color(look.labelFill) : undefined])
  return out
}

function lookKeys(look: ShapeLook, rotation: number): [string, string | number | undefined][] {
  const out: [string, string | number | undefined][] = [...(SHAPE_STYLE[look.kind] ?? [])]
  out.push(['fillColor', color(look.fill)], ['strokeColor', color(look.stroke)], ['strokeWidth', fmt(look.strokeWidth)])
  if (look.dash !== 'solid') out.push(['dashed', 1], ['dashPattern', look.dash === 'dotted' ? '1 2' : undefined])
  if (look.opacity < 1) out.push(['opacity', Math.round(look.opacity * 100)])
  if (look.radius > 0) out.push(['rounded', 1], ['absoluteArcSize', 1], ['arcSize', fmt(look.radius * 2)])
  if (look.direction !== 'e') out.push(['direction', DIRECTION[look.direction]])
  if (look.flipH) out.push(['flipH', 1])
  if (look.flipV) out.push(['flipV', 1])
  if (look.size !== null) out.push(['size', fmt(look.size)])
  if (rotation) out.push(['rotation', fmt(rotation)])
  return out
}

/** An SVG body as a standalone SVG document with its colours resolved (draw.io shows it as a picture). */
function svgDocument(look: ShapeLook): string {
  const svg = look.svg!
  const vars = `--aiws-shape-fill:${look.fill === 'auto' ? '#ffffff' : look.fill};--aiws-shape-stroke:${look.stroke === 'auto' ? '#000000' : look.stroke};--aiws-shape-stroke-width:${look.strokeWidth}`
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${escapeAttr(svg.viewBox)}" preserveAspectRatio="${svg.aspect === 'fixed' ? 'xMidYMid meet' : 'none'}" style="${vars}">${svg.body}</svg>`
}

const b64 = (text: string) => { const bytes = new TextEncoder().encode(text); let s = ''; for (const b of bytes) s += String.fromCharCode(b); return btoa(s) }
/** draw.io writes a data URI in a style without `;base64` (a `;` ends a style value). */
const styleDataUri = (uri: string) => uri.replace(/^data:([^;,]+);base64,/, 'data:$1,')

/** A label as draw.io HTML: escaped, line breaks as `<br>`. */
const html = (text: string) => text.split('\n').map((line) => line.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')).join('<br>')

const CAP_OUT: Record<string, [string, number]> = {
  none: ['none', 1], arrow: ['open', 1], triangle: ['classic', 1], triangle_outline: ['block', 0], diamond: ['diamond', 1], diamond_outline: ['diamond', 0],
  circle: ['oval', 1], circle_outline: ['oval', 0], bar: ['dash', 1],
  cardinality_one: ['ERone', 0], cardinality_many: ['ERmany', 0], cardinality_one_or_many: ['ERoneToMany', 0], cardinality_exactly_one: ['ERmandOne', 0],
  cardinality_zero_or_one: ['ERzeroToOne', 0], cardinality_zero_or_many: ['ERzeroToMany', 0],
}

export function exportDiagram(nodes: ExportNode[], page: ExportPage): ExportResult {
  const skipped = new Map<string, number>()
  const skip = (what: string) => skipped.set(what, (skipped.get(what) ?? 0) + 1)
  const byId = new Map(nodes.map((n) => [n.id, n]))
  // absolute rects (groups and Blocks; lines are placed from their ends)
  const abs = new Map<string, Rect>()
  const absOf = (id: string): Rect => {
    const known = abs.get(id)
    if (known) return known
    const n = byId.get(id)!
    const o = n.parent && byId.has(n.parent) ? absOf(n.parent) : { x: 0, y: 0, w: 0, h: 0 }
    const r = { x: o.x + n.placement.x, y: o.y + n.placement.y, w: n.placement.w, h: n.placement.h }
    abs.set(id, r)
    return r
  }
  const children = new Map<string | null, ExportNode[]>()
  for (const n of nodes) { const list = children.get(n.parent) ?? []; list.push(n); children.set(n.parent, list) }
  /** A group's first child covering it: the group exports as that container. */
  const backingOf = (g: ExportNode): ExportNode | undefined => {
    const first = (children.get(g.id) ?? [])[0]
    if (!first || first.kind !== 'cell' || (first.view !== 'frame' && first.view !== 'shape')) return undefined
    const p = first.placement
    return Math.abs(p.x) <= 1 && Math.abs(p.y) <= 1 && Math.abs(p.w - g.placement.w) <= 1 && Math.abs(p.h - g.placement.h) <= 1 ? first : undefined
  }
  // draw.io ids: the kept ones when unique, else the canvas ids
  const ids = new Map<string, string>()
  const used = new Set(['0', '1'])
  for (const n of nodes) {
    const kept = (n.config?.drawio as { id?: unknown } | undefined)?.id
    const want = typeof kept === 'string' && kept && !used.has(kept) ? kept : n.id
    ids.set(n.id, want)
    used.add(want)
  }
  for (const n of nodes) if (n.kind === 'group') { const b = backingOf(n); const kept = (b?.config?.drawio as { id?: unknown } | undefined)?.id; if (b && typeof kept === 'string' && ids.get(b.id) === kept) { ids.set(n.id, kept); ids.set(b.id, `${kept}~${b.id}`) } }
  const forms = new Map<string, { shape: Shape; anchors: readonly AnchorDef[] }>()
  const formOf = (id: string) => {
    let f = forms.get(id)
    if (!f) {
      const n = byId.get(id)
      const backing = n?.kind === 'group' ? backingOf(n) : undefined
      const shaped = n?.view === 'shape' ? n : backing?.view === 'shape' ? backing : undefined
      if (shaped) { const shape = shapeOutline(readLook(shaped.config)); f = { shape, anchors: shapeAnchors(shaped.config, shape) } } else f = { shape: 'rect', anchors: RECT_ANCHORS }
      forms.set(id, f)
    }
    return f
  }
  const cells: XmlElement[] = [el('mxCell', { id: '0' }), el('mxCell', { id: '1', parent: '0' })]
  const wrap = (n: ExportNode, cell: XmlElement, label: string): XmlElement => {
    const c = n.config ?? {}
    const data = c.data && typeof c.data === 'object' ? c.data as Record<string, unknown> : {}
    if (typeof c.link !== 'string' && typeof c.tooltip !== 'string' && Object.keys(data).length === 0) { cell.attrs = { id: cell.attrs.id, value: label, ...cell.attrs }; return cell }
    const attrs: Record<string, string> = { label }
    for (const [k, v] of Object.entries(data)) if (/^[A-Za-z_][\w.-]*$/.test(k) && !['id', 'label', 'link', 'tooltip', 'placeholders'].includes(k)) attrs[k] = String(v)
    if (typeof c.link === 'string') attrs.link = c.link
    if (typeof c.tooltip === 'string') attrs.tooltip = c.tooltip
    const id = cell.attrs.id
    delete cell.attrs.id
    return el('UserObject', { ...attrs, id }, [cell])
  }
  const geometry = (r: { x: number; y: number; w: number; h: number }) => el('mxGeometry', { x: fmt(r.x), y: fmt(r.y), width: fmt(r.w), height: fmt(r.h), as: 'geometry' })
  const parentId = (n: ExportNode) => (n.parent && byId.has(n.parent) ? ids.get(n.parent)! : '1')

  const vertex = (n: ExportNode, as: ExportNode = n): XmlElement => {
    const kept = (as.config?.drawio as { style?: unknown } | undefined)?.style
    const rotation = n.placement.rotation ?? 0
    let style: string
    let label = ''
    if (as.view === 'shape') {
      const look = readLook(as.config, as.title)
      label = look.text ? html(look.text) : ''
      const own = [...lookKeys(look, rotation), ...textKeys(look)]
      const anchors = (cleanAnchors(as.config?.anchors) ?? []).filter((a) => !/^(n|nne|ne|ene|e|ese|se|sse|s|ssw|sw|wsw|w|wnw|nw|nnw)$/.test(a.id))
      if (anchors.length) own.push(['points', JSON.stringify(anchors.map((a) => (a.dx || a.dy ? [a.x, a.y, 0, a.dx ?? 0, a.dy ?? 0] : [a.x, a.y])))])
      const keptShape = parseStyle(typeof kept === 'string' ? kept : '').keys.get('shape') ?? ''
      if (look.kind === 'svg') {
        // the library or inline stencil the import came from draws it again; anything else becomes a picture
        if (keptShape.startsWith('mxgraph.') || keptShape.startsWith('stencil(') || BUILTIN.has(keptShape)) style = styleFrom(kept, VERTEX_KEYS, own, true)
        else if (look.svg) style = styleFrom(kept, VERTEX_KEYS, [...own, ['shape', 'image'], ['imageAspect', look.svg.aspect === 'fixed' ? 1 : 0], ['image', `data:image/svg+xml,${b64(svgDocument(look))}`]])
        else style = styleFrom(kept, VERTEX_KEYS, own)
      } else style = styleFrom(kept, VERTEX_KEYS, own)
      if (look.kind === 'text') style = `text;${style}`
    } else if (as.view === 'frame') {
      const c = as.config ?? {}
      const header = typeof c.header === 'number' ? c.header : null
      label = as.title ? html(as.title) : ''
      const own: [string, string | number | undefined][] = [['html', 1], ['whiteSpace', 'wrap'], ['startSize', header ?? 0],
        ['horizontal', c.orientation === 'vertical' ? 0 : undefined], ['strokeColor', color(typeof c.color === 'string' ? c.color : '#4f8df7')],
        ['fillColor', color(typeof c.header_fill === 'string' ? c.header_fill : typeof c.color === 'string' ? c.color : '#4f8df7')],
        ['swimlaneFillColor', typeof c.fill === 'string' ? color(c.fill) : undefined], ['fontStyle', c.bold === true ? 1 : 0],
        ['fontSize', typeof c.font_size === 'number' ? fmt(c.font_size) : undefined], ['fontColor', typeof c.font_color === 'string' ? c.font_color : undefined],
        ['strokeWidth', typeof c.stroke_width === 'number' ? fmt(c.stroke_width) : header === null ? 2 : 1], ['dashed', c.dash === 'dashed' || c.dash === 'dotted' ? 1 : undefined],
        ['rounded', typeof c.radius === 'number' || header === null ? 1 : undefined], ['arcSize', typeof c.radius === 'number' ? fmt(c.radius * 2) : header === null ? 8 : undefined]]
      style = `swimlane;${styleFrom(kept, VERTEX_KEYS, own)}`
    } else if (as.view === 'asset' && as.image) {
      style = styleFrom(kept, VERTEX_KEYS, [['shape', 'image'], ['imageAspect', 1], ['aspect', 'fixed'], ['verticalLabelPosition', 'bottom'], ['verticalAlign', 'top'], ['html', 1], ['image', styleDataUri(as.image)], ['rotation', rotation ? fmt(rotation) : undefined]])
    } else {
      skip(as.view ?? 'group')
      label = html(as.text ?? as.title ?? '')
      style = `rounded=1;whiteSpace=wrap;html=1;align=left;verticalAlign=top;spacingLeft=6;dashed=1;fillColor=${as.view === 'note' ? '#fff2cc' : 'default'};strokeColor=default;`
    }
    if (n.locked || as.locked) style += 'locked=1;'
    if (n !== as) style += 'container=1;collapsible=0;'
    const cell = el('mxCell', { id: ids.get(n.id), style, vertex: '1', parent: parentId(n) }, [geometry(n.placement)])
    return wrap(as, cell, label)
  }

  const edge = (n: ExportNode): XmlElement => {
    const c = n.config ?? {}
    const l = n.line ?? {}
    const binding = (v: unknown): Binding | null => {
      const b = v as { entity_id?: unknown; anchor?: { id?: unknown } } | null | undefined
      return b && typeof b.entity_id === 'string' && typeof b.anchor?.id === 'string' && byId.has(b.entity_id) ? { entity_id: b.entity_id, anchor: { kind: 'named', id: b.anchor.id } } : null
    }
    const start = binding(l.start)
    const end = binding(l.end)
    const route: Route = l.route === 'elbow' || l.route === 'curve' ? l.route : 'straight'
    const controls = Array.isArray(l.controls) ? (l.controls as Control[]).filter((x) => x && typeof x.u === 'number' && typeof x.v === 'number') : []
    const flip = (l.flip && typeof l.flip === 'object' ? l.flip : {}) as { h?: boolean; v?: boolean }
    const box = absOf(n.id)
    const stored: [Point, Point] = [{ x: flip.h ? box.x + box.w : box.x, y: flip.v ? box.y + box.h : box.y }, { x: flip.h ? box.x : box.x + box.w, y: flip.v ? box.y : box.y + box.h }]
    const lookup = (id: string): TargetGeom | { broken: 'missing' } => {
      if (!byId.has(id)) return { broken: 'missing' }
      const f = formOf(id)
      return { rect: absOf(id), rotation: byId.get(id)!.placement.rotation ?? 0, shape: f.shape, anchors: f.anchors }
    }
    const data: ConnectorData = { start, end, flip, route, controls, label: { t: 0.5, offset: 0 } }
    const g = routeConnector(data, stored, lookup, { pad: 0 })
    const own: [string, string | number | undefined][] = [['html', 1]]
    if (route === 'elbow') own.push(['edgeStyle', 'orthogonalEdgeStyle'], ['orthogonalLoop', 1])
    else if (route === 'curve') own.push(['curved', 1], ['edgeStyle', undefined])
    else own.push(['edgeStyle', 'none'])
    own.push(['rounded', c.rounded === false ? 0 : 1])
    const constraint = (b: Binding | null, prefix: 'exit' | 'entry') => {
      if (!b) return
      const def = findAnchor(formOf(b.entity_id).anchors ?? defaultAnchors('rect'), b.anchor.id)
      if (!def) return
      own.push([`${prefix}X`, fmt(def.x)], [`${prefix}Y`, fmt(def.y)], [`${prefix}Dx`, fmt(def.dx ?? 0)], [`${prefix}Dy`, fmt(def.dy ?? 0)], [`${prefix}Perimeter`, 0])
    }
    constraint(start, 'exit')
    constraint(end, 'entry')
    const cap = (v: unknown, fallback: string): [string, number] => CAP_OUT[typeof v === 'string' ? v : fallback] ?? ['open', 1]
    const [sa, sf] = cap(c.start_cap, 'none')
    const [ea, ef] = cap(c.end_cap, 'arrow')
    own.push(['startArrow', sa], ['startFill', sf], ['endArrow', ea], ['endFill', ef])
    own.push(['strokeColor', typeof c.stroke === 'string' ? c.stroke : 'default'], ['strokeWidth', typeof c.width === 'number' ? fmt(c.width) : 2])
    if (c.dash === 'dashed' || c.dash === 'dotted') own.push(['dashed', 1], ['dashPattern', c.dash === 'dotted' ? '1 2' : undefined])
    if (typeof c.opacity === 'number' && c.opacity < 1) own.push(['opacity', Math.round(c.opacity * 100)])
    if (typeof c.elbow === 'string') own.push(['elbow', c.elbow])
    own.push(['fontSize', c.label_size === 's' ? 11 : c.label_size === 'l' ? 18 : 14])
    if (typeof c.label_color === 'string') own.push(['fontColor', c.label_color])
    if (typeof c.label_fill === 'string') own.push(['labelBackgroundColor', c.label_fill])
    let style = styleFrom((c.drawio as { style?: unknown } | undefined)?.style, EDGE_KEYS, own)
    if (n.locked) style += 'locked=1;'
    const origin = n.parent && byId.has(n.parent) ? absOf(n.parent) : { x: 0, y: 0 }
    const rel = (p: Point) => el('mxPoint', { x: fmt(p.x - origin.x), y: fmt(p.y - origin.y) })
    // the bends draw.io routes through: an elbow's corners, the points a straight line or a curve passes
    const bends = route === 'elbow' && controls.length === 0 ? [] : route === 'elbow' ? g.pts.slice(1, -1) : controls.map((x) => compose(x, g.start.point, g.end.point))
    const lp = l.label as { t?: number; offset?: number } | undefined
    const geo = el('mxGeometry', { ...(lp && typeof lp.t === 'number' && lp.t !== 0.5 ? { x: fmt(lp.t * 2 - 1) } : {}), ...(lp?.offset ? { y: fmt(lp.offset) } : {}), relative: '1', as: 'geometry' }, [
      ...(start ? [] : [el('mxPoint', { x: fmt(g.start.point.x - origin.x), y: fmt(g.start.point.y - origin.y), as: 'sourcePoint' })]),
      ...(end ? [] : [el('mxPoint', { x: fmt(g.end.point.x - origin.x), y: fmt(g.end.point.y - origin.y), as: 'targetPoint' })]),
      ...(bends.length ? [el('Array', { as: 'points' }, bends.map(rel))] : []),
    ])
    const cell = el('mxCell', { id: ids.get(n.id), style, edge: '1', parent: parentId(n), ...(start ? { source: ids.get(start.entity_id) } : {}), ...(end ? { target: ids.get(end.entity_id) } : {}) }, [geo])
    return wrap(n, cell, n.title ? html(n.title) : '')
  }

  const walk = (parent: string | null) => {
    for (const n of children.get(parent) ?? []) {
      if (n.kind === 'group') {
        const backing = backingOf(n)
        if (backing) cells.push(vertex(n, backing))
        else cells.push(el('mxCell', { id: ids.get(n.id), value: '', style: `group;${n.locked ? 'locked=1;' : ''}`, vertex: '1', connectable: '0', parent: parentId(n) }, [geometry(n.placement)]))
        walk(n.id)
        continue
      }
      if (parent !== null && byId.get(parent)?.kind === 'group' && backingOf(byId.get(parent)!) === n) continue
      cells.push(n.view === 'connector' ? edge(n) : vertex(n))
    }
  }
  walk(null)
  const model = el('mxGraphModel', {
    dx: '1000', dy: '700', grid: '1', gridSize: '10', guides: '1', tooltips: '1', connect: '1', arrows: '1', fold: '1', page: page.pageSize ? '1' : '0', pageScale: '1',
    pageWidth: fmt(page.pageSize?.w ?? 850), pageHeight: fmt(page.pageSize?.h ?? 1100), ...(page.background ? { background: page.background } : {}), math: '0', shadow: '0',
  }, [el('root', {}, cells)])
  const file = el('mxfile', { host: 'BuckyOS AI Workspace', type: 'device' }, [el('diagram', { id: 'page-1', name: page.name || 'Page-1' }, [model])])
  return { xml: toXml(file, '', '  ') + '\n', skipped }
}

/** draw.io shapes the importer draws from built-in SVG bodies: written back under their own name. */
const BUILTIN = new Set(['actor', 'umlActor', 'tape', 'internalStorage', 'manualInput', 'dataStorage', 'delay', 'display', 'offPageConnector', 'sumEllipse', 'orEllipse', 'cube', 'folder', 'component', 'umlBoundary', 'umlEntity', 'umlControl', 'lollipop', 'waypoint', 'corner', 'tee', 'plus'])
