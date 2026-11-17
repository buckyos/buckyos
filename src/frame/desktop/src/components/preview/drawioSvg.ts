import { bytesToBase64 } from '@drawio/bytes'
import { RECT_ANCHORS } from '@drawio/anchors'
import { endTangents, pathData, routeConnector, trimmed, type ConnectorGeometry, type TargetGeom } from '@drawio/connector'
import { convertPage, ImportReport, readDrawio, type ConvertOptions, type DrawioFile, type DrawioPage, type PagePlan, type PlanItem } from '@drawio/mod'
import { rotatedBounds, type Point, type Rect } from '@drawio/geometry'
import { dashArray, labelBox, readLook, shapeAnchors, shapeOutline, shapePaths, type FontFamily, type ShapeLook } from '@drawio/shapes'
import { decodeStencilStyle, stencilToSvg } from '@drawio/stencil'
import { sanitizeSvgBody } from '@drawio/svg'
import { libraryShape } from '../../app/aiworkspace/ui/canvas/stencilLibrary'

export type { DrawioFile, DrawioPage } from '@drawio/mod'

const AUTO_FILL = '#ffffff'
const AUTO_STROKE = '#000000'
const AUTO_INK = '#000000'
const PAGE = '#ffffff'
const MARGIN = 16

const FONT_STACK: Record<FontFamily, string> = {
  sans: 'Helvetica, Arial, "PingFang SC", "Microsoft YaHei", "Noto Sans CJK SC", "Noto Sans", sans-serif',
  serif: '"Times New Roman", Times, "Songti SC", SimSun, "Noto Serif CJK SC", serif',
  mono: 'ui-monospace, SFMono-Regular, Menlo, Consolas, "Liberation Mono", monospace',
}

const APPROXIMATED = new Set(['approximated', 'unknown_shape', 'library_shape', 'stencil_failed', 'image_missing', 'svg_too_large'])

export interface RenderedDiagramPage {
  svg: string
  width: number
  height: number
  approximated: boolean
}

export interface DiagramRenderOptions {
  imageText?: (url: string | null) => string
  aspect?: number
}

export function openDiagram(bytes: Uint8Array): Promise<DrawioFile> {
  return readDrawio({ bytes })
}

export async function renderDiagramPage(page: DrawioPage, options: DiagramRenderOptions = {}): Promise<RenderedDiagramPage> {
  const report = new ImportReport()
  const convert: ConvertOptions = {
    stencil: (name) => libraryShape(name),
    inlineStencil: async (value) => {
      const shape = await decodeStencilStyle(value)
      if (!shape) return null
      const s = stencilToSvg(shape)
      return { title: s.title, svg: s.svg, anchors: s.anchors, unsupported: s.unsupported }
    },
    imageText: options.imageText ?? ((url) => (url ? `Image: ${url}` : 'Image')),
  }
  const plan = await convertPage(page, report, convert)
  const approximated = report.list().some((note) => APPROXIMATED.has(note.kind))
  return { ...drawPlan(plan, options.aspect), approximated }
}

const f = (n: number) => String(Math.round(n * 100) / 100)
const esc = (s: string) =>
  s
    // eslint-disable-next-line no-control-regex
    .replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f\ufffe\uffff]/g, '')
    .replace(/[&<>"']/g, (c) => (c === '&' ? '&amp;' : c === '<' ? '&lt;' : c === '>' ? '&gt;' : c === '"' ? '&quot;' : '&#39;'))
const HEX = /^#[0-9a-f]{6}$/i

function paint(value: string, auto: string): string {
  if (value === 'auto') return auto
  if (value === 'none') return 'none'
  return HEX.test(value) ? value : auto
}

class Bounds {
  x1 = Infinity
  y1 = Infinity
  x2 = -Infinity
  y2 = -Infinity
  add(r: Rect) {
    if (![r.x, r.y, r.w, r.h].every(Number.isFinite)) return
    this.x1 = Math.min(this.x1, r.x)
    this.y1 = Math.min(this.y1, r.y)
    this.x2 = Math.max(this.x2, r.x + r.w)
    this.y2 = Math.max(this.y2, r.y + r.h)
  }
  rect(): Rect {
    if (!Number.isFinite(this.x1)) return { x: 0, y: 0, w: 1, h: 1 }
    return { x: this.x1, y: this.y1, w: Math.max(1, this.x2 - this.x1), h: Math.max(1, this.y2 - this.y1) }
  }
}

interface Ctx {
  page: string
  ids: number
  bounds: Bounds
}

const rotate = (item: PlanItem, body: string) => {
  if (!item.rotation) return body
  const cx = item.rect.x + item.rect.w / 2
  const cy = item.rect.y + item.rect.h / 2
  return `<g transform="rotate(${f(item.rotation)} ${f(cx)} ${f(cy)})">${body}</g>`
}

interface TextBox {
  x: number
  y: number
  w: number
  h: number
  text: string
  fontSize: number
  color: string
  align: 'left' | 'center' | 'right'
  valign: 'top' | 'middle' | 'bottom'
  wrap: boolean
  bold?: boolean
  italic?: boolean
  underline?: boolean
  family?: FontFamily | null
  lineHeight?: number | null
  background?: string | null
  padding?: number
}

function textMarkup(box: TextBox): string {
  if (!box.text) return ''
  const padX = Math.max(box.w, 120)
  const padY = Math.max(box.h, 60)
  const justify = box.valign === 'top' ? 'flex-start' : box.valign === 'bottom' ? 'flex-end' : 'center'
  const items = box.align === 'left' ? 'flex-start' : box.align === 'right' ? 'flex-end' : 'center'
  const outer = `padding:${f(padY)}px ${f(padX)}px;margin:0;box-sizing:content-box;width:${f(box.w)}px;height:${f(box.h)}px`
  const inner = [
    `width:${f(box.w)}px`, `height:${f(box.h)}px`, 'box-sizing:border-box', `padding:${box.padding ?? 2}px`,
    'display:flex', 'flex-direction:column', `justify-content:${justify}`, `align-items:${items}`,
  ].join(';')
  const span = [
    'display:block', `max-width:${box.wrap ? '100%' : 'none'}`, `white-space:${box.wrap ? 'pre-wrap' : 'pre'}`, 'overflow-wrap:anywhere',
    `text-align:${box.align}`, `font-size:${f(box.fontSize)}px`, `line-height:${box.lineHeight ?? 1.2}`, `color:${box.color}`,
    `font-family:${FONT_STACK[box.family ?? 'sans']}`,
    ...(box.bold ? ['font-weight:700'] : []), ...(box.italic ? ['font-style:italic'] : []), ...(box.underline ? ['text-decoration:underline'] : []),
    ...(box.background ? [`background:${box.background}`, 'padding:0 2px'] : []),
  ].join(';')
  return (
    `<foreignObject x="${f(box.x - padX)}" y="${f(box.y - padY)}" width="${f(box.w + padX * 2)}" height="${f(box.h + padY * 2)}" style="overflow:visible">` +
    `<div xmlns="http://www.w3.org/1999/xhtml" style="${esc(outer)}"><div style="${esc(inner)}"><span style="${esc(span)}">${esc(box.text)}</span></div></div>` +
    '</foreignObject>'
  )
}

function shapeColors(look: ShapeLook) {
  return { fill: paint(look.fill, AUTO_FILL), stroke: paint(look.stroke, AUTO_STROKE) }
}

function shapeArt(look: ShapeLook, rect: Rect): string {
  const { fill, stroke } = shapeColors(look)
  const w = Math.max(rect.w, 1)
  const h = Math.max(rect.h, 1)
  if (look.kind === 'svg') {
    if (!look.svg) return `<rect x="${f(rect.x)}" y="${f(rect.y)}" width="${f(w)}" height="${f(h)}" fill="${fill}" stroke="${stroke}"/>`
    const vars = `--aiws-shape-fill:${fill};--aiws-shape-stroke:${stroke};--aiws-shape-stroke-width:${f(look.strokeWidth)};--aiws-shape-font:${AUTO_INK};color:${AUTO_INK}`
    const aspect = look.svg.aspect === 'fixed' ? 'xMidYMid meet' : 'none'
    return `<svg x="${f(rect.x)}" y="${f(rect.y)}" width="${f(w)}" height="${f(h)}" viewBox="${esc(look.svg.viewBox)}" preserveAspectRatio="${aspect}" overflow="visible" style="${vars}">${sanitizeSvgBody(look.svg.body)}</svg>`
  }
  if (look.kind === 'text' && look.fill === 'none' && look.stroke === 'none') return ''
  const paths = shapePaths(look.kind === 'text' ? { ...look, kind: 'rect' } : look, w, h)
  const width = look.stroke === 'none' ? 0 : look.strokeWidth
  const dash = dashArray(look.dash, width)
  const strokeAttrs = width > 0 ? ` stroke="${stroke}" stroke-width="${f(width)}" stroke-linejoin="round"${dash ? ` stroke-dasharray="${dash}"` : ''}` : ' stroke="none"'
  let out = `<g transform="translate(${f(rect.x)} ${f(rect.y)})">`
  out += `<path d="${paths.body}" fill="${paths.open ? 'none' : fill}"${strokeAttrs}/>`
  for (const d of paths.details) out += `<path d="${d}" fill="none"${strokeAttrs}/>`
  return `${out}</g>`
}

function shapeMarkup(ctx: Ctx, item: PlanItem, config: Record<string, unknown>, title?: string): string {
  const look = readLook(config, title ?? item.title)
  const rect = item.rect
  let body = shapeArt(look, rect)
  if (look.text) {
    const box = labelBox(look, Math.max(rect.w, 1), Math.max(rect.h, 1))
    const ink = look.fontColor && look.fontColor !== 'auto' ? look.fontColor : AUTO_INK
    body += textMarkup({
      x: rect.x + box.x, y: rect.y + box.y, w: box.w, h: box.h, text: look.text, fontSize: look.fontSize, color: ink,
      align: look.align, valign: look.valign, wrap: look.wrap, bold: look.bold, italic: look.italic, underline: look.underline,
      family: look.fontFamily, lineHeight: look.lineHeight, background: look.labelFill ? paint(look.labelFill, AUTO_FILL) : null,
    })
    if (look.labelPosition !== 'inside') ctx.bounds.add({ x: rect.x + box.x, y: rect.y + box.y, w: box.w, h: box.h })
  }
  ctx.bounds.add(rotatedBounds(rect, item.rotation))
  const group = look.opacity < 1 ? `<g opacity="${f(look.opacity)}">${body}</g>` : body
  return rotate(item, group)
}

function frameMarkup(ctx: Ctx, item: PlanItem): string {
  const c = item.config as Record<string, unknown>
  const { x, y, w, h } = item.rect
  const num = (v: unknown, fallback: number) => (typeof v === 'number' && Number.isFinite(v) ? v : fallback)
  const color = typeof c.color === 'string' ? paint(c.color, AUTO_STROKE) : AUTO_STROKE
  const headerFill = typeof c.header_fill === 'string' ? paint(c.header_fill, AUTO_FILL) : color
  const bodyFill = typeof c.fill === 'string' ? paint(c.fill, AUTO_FILL) : 'none'
  const header = Math.max(0, Math.min(num(c.header, 23), c.orientation === 'vertical' ? w : h))
  const vertical = c.orientation === 'vertical'
  const width = Math.max(0, num(c.stroke_width, 1))
  const radius = Math.max(0, num(c.radius, 0))
  const dash = dashArray(c.dash === 'dashed' || c.dash === 'dotted' ? c.dash : 'solid', width)
  const clip = `cp-dio-${ctx.ids++}`
  let out = `<clipPath id="${clip}"><rect x="${f(x)}" y="${f(y)}" width="${f(w)}" height="${f(h)}" rx="${f(radius)}"/></clipPath>`
  out += `<g clip-path="url(#${clip})">`
  if (bodyFill !== 'none') out += `<rect x="${f(x)}" y="${f(y)}" width="${f(w)}" height="${f(h)}" fill="${bodyFill}"/>`
  if (header > 0 && headerFill !== 'none') {
    out += vertical
      ? `<rect x="${f(x)}" y="${f(y)}" width="${f(header)}" height="${f(h)}" fill="${headerFill}"/>`
      : `<rect x="${f(x)}" y="${f(y)}" width="${f(w)}" height="${f(header)}" fill="${headerFill}"/>`
  }
  out += '</g>'
  if (width > 0 && color !== 'none') {
    const strokeAttrs = `fill="none" stroke="${color}" stroke-width="${f(width)}"${dash ? ` stroke-dasharray="${dash}"` : ''}`
    out += `<rect x="${f(x)}" y="${f(y)}" width="${f(w)}" height="${f(h)}" rx="${f(radius)}" ${strokeAttrs}/>`
    if (header > 0) {
      out += vertical
        ? `<path d="M${f(x + header)} ${f(y)}L${f(x + header)} ${f(y + h)}" ${strokeAttrs}/>`
        : `<path d="M${f(x)} ${f(y + header)}L${f(x + w)} ${f(y + header)}" ${strokeAttrs}/>`
    }
  }
  const title = item.title ?? ''
  if (title && header > 0) {
    const text: Omit<TextBox, 'x' | 'y' | 'w' | 'h'> = {
      text: title, fontSize: num(c.font_size, 12), color: typeof c.font_color === 'string' && HEX.test(c.font_color) ? c.font_color : AUTO_INK,
      align: 'center', valign: 'middle', wrap: false, bold: c.bold === true, italic: c.italic === true, underline: c.underline === true,
    }
    out += vertical
      ? `<g transform="translate(${f(x)} ${f(y + h)}) rotate(-90)">${textMarkup({ ...text, x: 0, y: 0, w: h, h: header })}</g>`
      : textMarkup({ ...text, x, y, w, h: header })
  }
  ctx.bounds.add(item.rect)
  return out
}

function assetMarkup(ctx: Ctx, item: PlanItem): string {
  const image = item.image
  if (!image?.bytes || !/^image\//.test(image.mediaType)) {
    return item.fallback ? shapeMarkup(ctx, item, item.fallback.config as Record<string, unknown>, item.fallback.title) : ''
  }
  const { x, y, w, h } = item.rect
  const href = `data:${image.mediaType};base64,${bytesToBase64(image.bytes)}`
  ctx.bounds.add(rotatedBounds(item.rect, item.rotation))
  return rotate(item, `<image x="${f(x)}" y="${f(y)}" width="${f(Math.max(w, 1))}" height="${f(Math.max(h, 1))}" href="${esc(href)}" preserveAspectRatio="xMidYMid meet"/>`)
}

type Cap = 'none' | 'arrow' | 'triangle' | 'triangle_outline' | 'diamond' | 'diamond_outline' | 'circle' | 'circle_outline' | 'bar'
  | 'cardinality_one' | 'cardinality_many' | 'cardinality_one_or_many' | 'cardinality_exactly_one' | 'cardinality_zero_or_one' | 'cardinality_zero_or_many'
const CAPS = new Set<string>(['none', 'arrow', 'triangle', 'triangle_outline', 'diamond', 'diamond_outline', 'circle', 'circle_outline', 'bar',
  'cardinality_one', 'cardinality_many', 'cardinality_one_or_many', 'cardinality_exactly_one', 'cardinality_zero_or_one', 'cardinality_zero_or_many'])
const capOf = (v: unknown, fallback: Cap): Cap => (typeof v !== 'string' ? fallback : CAPS.has(v) ? (v as Cap) : 'arrow')
const LABEL_FONT = { s: 12, m: 14, l: 18 } as const

const capSize = (width: number) => 6 + width * 2.5

function capInset(cap: Cap, size: number): number {
  switch (cap) {
    case 'triangle': case 'triangle_outline': case 'diamond': case 'diamond_outline': return size
    case 'circle': case 'circle_outline': return size * 0.64
    default: return 0
  }
}

function capShape(cap: Cap, L: number, color: string, width: number, page: string): string {
  const line = (d: string) => `<path d="${d}" fill="none" stroke="${color}" stroke-width="${f(width)}" stroke-linecap="round" stroke-linejoin="round"/>`
  const closed = (d: string, filled: boolean) => `<path d="${d}" fill="${filled ? color : page}" stroke="${color}" stroke-width="${f(Math.max(width * 0.75, 0.5))}" stroke-linejoin="round"/>`
  const ring = (cx: number, r: number, filled: boolean) => `<circle cx="${f(cx)}" cy="0" r="${f(r)}" fill="${filled ? color : page}" stroke="${color}" stroke-width="${f(Math.max(width * 0.75, 0.5))}"/>`
  const bar = (x: number) => line(`M${f(x)} ${f(-L * 0.5)}L${f(x)} ${f(L * 0.5)}`)
  const crow = line(`M0 ${f(-L * 0.55)}L${f(-L)} 0L0 ${f(L * 0.55)}`)
  const triangle = `M0 0L${f(-L)} ${f(-L * 0.5)}L${f(-L)} ${f(L * 0.5)}Z`
  const diamond = `M0 0L${f(-L / 2)} ${f(-L * 0.38)}L${f(-L)} 0L${f(-L / 2)} ${f(L * 0.38)}Z`
  switch (cap) {
    case 'none': return ''
    case 'arrow': return line(`M${f(-L)} ${f(-L * 0.5)}L0 0L${f(-L)} ${f(L * 0.5)}`)
    case 'triangle': return closed(triangle, true)
    case 'triangle_outline': return closed(triangle, false)
    case 'diamond': return closed(diamond, true)
    case 'diamond_outline': return closed(diamond, false)
    case 'circle': return ring(-L * 0.32, L * 0.32, true)
    case 'circle_outline': return ring(-L * 0.32, L * 0.32, false)
    case 'bar': return bar(0)
    case 'cardinality_one': return bar(-L * 0.6)
    case 'cardinality_exactly_one': return bar(-L * 0.45) + bar(-L * 0.85)
    case 'cardinality_many': return crow
    case 'cardinality_one_or_many': return crow + bar(-L * 1.2)
    case 'cardinality_zero_or_one': return bar(-L * 0.5) + ring(-L * 1.15, L * 0.3, false)
    case 'cardinality_zero_or_many': return crow + ring(-L * 1.4, L * 0.3, false)
  }
}

function connectorMarkup(ctx: Ctx, item: PlanItem, lookup: (key: string) => TargetGeom | { broken: 'missing' }): string {
  const line = item.line
  if (!line) return ''
  const c = item.config as Record<string, unknown>
  const width = typeof c.width === 'number' && Number.isFinite(c.width) ? Math.min(24, Math.max(0.5, c.width)) : 1
  const stroke = typeof c.stroke === 'string' && HEX.test(c.stroke) ? c.stroke : AUTO_STROKE
  const opacity = typeof c.opacity === 'number' && Number.isFinite(c.opacity) ? Math.min(1, Math.max(0.1, c.opacity)) : 1
  const startCap = capOf(c.start_cap, 'none')
  const endCap = capOf(c.end_cap, 'arrow')
  const labelSize = c.label_size === 's' || c.label_size === 'l' ? c.label_size : 'm'
  const font = LABEL_FONT[labelSize]
  const title = item.title ?? ''
  const binding = (end: typeof line.start) => (end ? { entity_id: end.key, anchor: { kind: 'named' as const, id: end.anchor } } : null)
  let g: ConnectorGeometry
  try {
    g = routeConnector(
      { start: binding(line.start), end: binding(line.end), flip: {}, route: line.route, controls: line.controls, label: line.label ?? { t: 0.5, offset: 0 } },
      [line.startPoint, line.endPoint],
      lookup,
      { title: title || null, font, pad: 0 },
    )
  } catch {
    return ''
  }
  if (g.segs.length === 0) return ''
  const L = capSize(width)
  const [tanS, tanE] = endTangents(g.segs)
  const body = trimmed(g.segs, capInset(startCap, L), capInset(endCap, L))
  const radius = g.route === 'elbow' && c.rounded !== false ? 4 + width * 2 : 0
  const dash = c.dash === 'dashed' ? ` stroke-dasharray="${f(width * 4)} ${f(width * 3)}"` : c.dash === 'dotted' ? ` stroke-dasharray="0.01 ${f(width * 2.2)}"` : ''
  let out = `<g opacity="${f(opacity)}">`
  out += `<path d="${pathData(body, radius)}" fill="none" stroke="${stroke}" stroke-width="${f(width)}" stroke-linejoin="round" stroke-linecap="${c.dash === 'dotted' ? 'round' : 'butt'}"${dash}/>`
  const cap = (kind: Cap, tip: Point, dir: Point) => {
    const shape = capShape(kind, L, stroke, width, ctx.page)
    return shape ? `<g transform="translate(${f(tip.x)} ${f(tip.y)}) rotate(${f((Math.atan2(dir.y, dir.x) * 180) / Math.PI)})">${shape}</g>` : ''
  }
  out += cap(startCap, g.segs[0].a, { x: -tanS.x, y: -tanS.y })
  out += cap(endCap, g.segs[g.segs.length - 1].b, tanE)
  out += '</g>'
  const pad = L + width
  for (const p of g.flat) ctx.bounds.add({ x: p.x - pad, y: p.y - pad, w: pad * 2, h: pad * 2 })
  if (title && g.label.box) {
    const box = g.label.box
    const fill = typeof c.label_fill === 'string' && HEX.test(c.label_fill) ? c.label_fill : ctx.page
    const ink = typeof c.label_color === 'string' && HEX.test(c.label_color) ? c.label_color : AUTO_INK
    out += textMarkup({ x: box.x, y: box.y, w: box.w, h: box.h, text: title, fontSize: font, color: ink, align: 'center', valign: 'middle', wrap: true, background: fill, padding: 0 })
    ctx.bounds.add(box)
  }
  return out
}

function targetLookup(items: PlanItem[]): (key: string) => TargetGeom | { broken: 'missing' } {
  const byKey = new Map(items.map((item) => [item.key, item]))
  const cache = new Map<string, TargetGeom>()
  return (key) => {
    const known = cache.get(key)
    if (known) return known
    const item = byKey.get(key)
    if (!item || item.type === 'connector') return { broken: 'missing' }
    const config = item.config as Record<string, unknown>
    const shape = item.type === 'shape' ? shapeOutline(readLook(config)) : 'rect'
    const anchors = item.type === 'shape' || Array.isArray(config.anchors) ? shapeAnchors(config, shape) : RECT_ANCHORS
    const geom: TargetGeom = { rect: item.rect, rotation: item.rotation, shape, anchors }
    cache.set(key, geom)
    return geom
  }
}

function drawPlan(plan: PagePlan, aspect?: number): { svg: string; width: number; height: number } {
  const page = plan.background ?? PAGE
  const ctx: Ctx = { page, ids: 0, bounds: new Bounds() }
  const lookup = targetLookup(plan.items)
  let body = ''
  for (const item of plan.items) {
    switch (item.type) {
      case 'shape': body += shapeMarkup(ctx, item, item.config as Record<string, unknown>); break
      case 'frame': body += frameMarkup(ctx, item); break
      case 'asset': body += assetMarkup(ctx, item); break
      case 'connector': body += connectorMarkup(ctx, item, lookup); break
      default: break
    }
  }
  const r = ctx.bounds.rect()
  let x = Math.floor(r.x - MARGIN)
  let y = Math.floor(r.y - MARGIN)
  let width = Math.ceil(r.x + r.w + MARGIN) - x
  let height = Math.ceil(r.y + r.h + MARGIN) - y
  if (aspect && Number.isFinite(aspect) && aspect > 0) {
    if (width / height < aspect) {
      const wide = Math.ceil(height * aspect)
      x -= Math.floor((wide - width) / 2)
      width = wide
    } else {
      const tall = Math.ceil(width / aspect)
      y -= Math.floor((tall - height) / 2)
      height = tall
    }
  }
  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="${width}" height="${height}" viewBox="${x} ${y} ${width} ${height}">` +
    `<rect x="${x}" y="${y}" width="${width}" height="${height}" fill="${page}"/>` +
    body +
    '</svg>'
  return { svg, width, height }
}
