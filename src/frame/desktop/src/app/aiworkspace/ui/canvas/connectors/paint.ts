/* Drawing a connector (连接线实现方案 §5.2, §9.4, §9.5): one absolutely positioned frame per line in the world
 * layer, holding an SVG (the path, the caps, the label gap) and the label as HTML. React renders it and a
 * running gesture repaints it in place (`paintConnector`), both from the same markup, so a preview and the
 * committed drawing cannot differ. Widths are world units with a 1-screen-px floor; caps scale with the width
 * and the path is shortened so a cap's tip lands on the end; low zoom drops the label and simplifies caps. */

import type { Point } from '../geometry'
import type { Rect } from '../render/camera'
import { END_GAP, endTangents, labelBox, pathData, trimmed, type ConnectorGeometry } from './geometry'
import { LABEL_FONT, type Cap, type ConnectorStyle } from './model'

export interface PaintOptions {
  zoom: number
  /** Low detail: no label, simple caps. */
  simplified: boolean
  /** Edit mode: broken ends get a hollow "!" marker. */
  showBroken: boolean
  /** Unique within the page (mask id). */
  id: string
  title: string | null
}

const f = (n: number) => Math.round(n * 100) / 100
const esc = (s: string) => s.replace(/[&<>"]/g, (c) => (c === '&' ? '&amp;' : c === '<' ? '&lt;' : c === '>' ? '&gt;' : '&quot;'))

export function strokeWidth(style: ConnectorStyle, zoom: number): number {
  return Math.max(style.width, 1 / Math.max(zoom, 0.01))
}

/** Cap length along the line, growing with the width. */
export function capSize(width: number): number {
  return 6 + width * 2.5
}

/** How far the line stops short of the tip so a closed cap is not crossed by it. */
function capInset(cap: Cap, size: number): number {
  switch (cap) {
    case 'triangle': case 'triangle_outline': case 'diamond': case 'diamond_outline': return size
    case 'circle': case 'circle_outline': return size * 0.64
    default: return 0
  }
}

/** A cap drawn at the origin pointing along +x (the tip at 0, 0). */
function capShape(cap: Cap, size: number, color: string, width: number): string {
  const L = size
  const line = (d: string) => `<path d="${d}" fill="none" stroke="${color}" stroke-width="${f(width)}" stroke-linecap="round" stroke-linejoin="round"/>`
  const closed = (d: string, filled: boolean) => `<path d="${d}" fill="${filled ? color : 'var(--cp-bg)'}" stroke="${color}" stroke-width="${f(Math.max(width * 0.75, 0.5))}" stroke-linejoin="round"/>`
  const ring = (cx: number, r: number, filled: boolean) => `<circle cx="${f(cx)}" cy="0" r="${f(r)}" fill="${filled ? color : 'var(--cp-bg)'}" stroke="${color}" stroke-width="${f(Math.max(width * 0.75, 0.5))}"/>`
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

export function labelFont(style: ConnectorStyle): number {
  return LABEL_FONT[style.labelSize]
}

/** The drawn label box (world), or null without a label or in low detail. */
export function paintedLabel(g: ConnectorGeometry, style: ConnectorStyle, options: Pick<PaintOptions, 'simplified' | 'title'>): Rect | null {
  return options.title && !options.simplified ? labelBox(options.title, labelFont(style), g.label.point) : null
}

/** The frame's box: the path with room for the width and caps, and the label. */
export function frameBox(g: ConnectorGeometry, style: ConnectorStyle, options: Pick<PaintOptions, 'zoom' | 'simplified' | 'title'>): Rect {
  const w = strokeWidth(style, options.zoom)
  const pad = Math.max(capSize(w) * 0.6, w) + 6 / Math.max(options.zoom, 0.01)
  let x1 = Infinity, y1 = Infinity, x2 = -Infinity, y2 = -Infinity
  for (const p of g.flat) { x1 = Math.min(x1, p.x); y1 = Math.min(y1, p.y); x2 = Math.max(x2, p.x); y2 = Math.max(y2, p.y) }
  x1 -= pad; y1 -= pad; x2 += pad; y2 += pad
  const label = paintedLabel(g, style, options)
  if (label) { x1 = Math.min(x1, label.x); y1 = Math.min(y1, label.y); x2 = Math.max(x2, label.x + label.w); y2 = Math.max(y2, label.y + label.h) }
  return { x: Math.floor(x1), y: Math.floor(y1), w: Math.ceil(x2 - Math.floor(x1)), h: Math.ceil(y2 - Math.floor(y1)) }
}

/** The SVG content (world coordinates) of one connector. */
export function connectorMarkup(g: ConnectorGeometry, style: ConnectorStyle, options: PaintOptions): string {
  const w = strokeWidth(style, options.zoom)
  const L = capSize(w)
  const startCap: Cap = options.simplified && style.startCap !== 'none' ? 'triangle' : style.startCap
  const endCap: Cap = options.simplified && style.endCap !== 'none' ? 'triangle' : style.endCap
  const gapS = g.start.state === 'bound' ? END_GAP : 0
  const gapE = g.end.state === 'bound' ? END_GAP : 0
  const atTips = trimmed(g.segs, gapS, gapE)
  const [tanS, tanE] = endTangents(atTips)
  const body = trimmed(atTips, capInset(startCap, L), capInset(endCap, L))
  const radius = g.route === 'elbow' && style.rounded ? 4 + w * 2 : 0
  const dash = style.dash === 'dashed' ? ` stroke-dasharray="${f(w * 4)} ${f(w * 3)}"` : style.dash === 'dotted' ? ` stroke-dasharray="0.01 ${f(w * 2.2)}"` : ''
  const label = paintedLabel(g, style, options)
  const gap = label && !style.labelFill
  const maskId = `aiws-cm-${options.id}`
  let out = ''
  if (gap) {
    const b = frameBox(g, style, options)
    out += `<mask id="${esc(maskId)}" maskUnits="userSpaceOnUse" x="${b.x}" y="${b.y}" width="${b.w}" height="${b.h}"><rect x="${b.x}" y="${b.y}" width="${b.w}" height="${b.h}" fill="white"/><rect x="${f(label.x)}" y="${f(label.y)}" width="${f(label.w)}" height="${f(label.h)}" rx="3" fill="black"/></mask>`
  }
  out += `<g opacity="${style.opacity}">`
  out += `<path class="aiws-connector-path" d="${pathData(body, radius)}" fill="none" stroke="${style.stroke}" stroke-width="${f(w)}" stroke-linejoin="round" stroke-linecap="${style.dash === 'dotted' ? 'round' : 'butt'}"${dash}${gap ? ` mask="url(#${esc(maskId)})"` : ''}/>`
  const cap = (kind: Cap, tip: Point, dir: Point) => {
    const shape = capShape(kind, L, style.stroke, w)
    return shape ? `<g transform="translate(${f(tip.x)} ${f(tip.y)}) rotate(${f((Math.atan2(dir.y, dir.x) * 180) / Math.PI)})">${shape}</g>` : ''
  }
  if (atTips.length) {
    out += cap(startCap, atTips[0].a, { x: -tanS.x, y: -tanS.y })
    out += cap(endCap, atTips[atTips.length - 1].b, tanE)
  }
  out += '</g>'
  if (options.showBroken) {
    const r = 5 / Math.max(options.zoom, 0.01)
    for (const end of [g.start, g.end]) {
      if (end.state !== 'broken') continue
      out += `<g class="aiws-connector-broken"><circle cx="${f(end.point.x)}" cy="${f(end.point.y)}" r="${f(r)}" fill="var(--cp-bg)" stroke="var(--cp-muted)" stroke-width="${f(1.5 / options.zoom)}"/><text x="${f(end.point.x)}" y="${f(end.point.y)}" font-size="${f(r * 1.5)}" text-anchor="middle" dominant-baseline="central" fill="var(--cp-muted)">!</text></g>`
    }
  }
  return out
}

/** Repaint a connector frame in place (a gesture preview, or the restore after one). */
export function paintConnector(frame: HTMLElement, g: ConnectorGeometry, style: ConnectorStyle, options: PaintOptions) {
  const box = frameBox(g, style, options)
  frame.style.left = `${box.x}px`
  frame.style.top = `${box.y}px`
  frame.style.width = `${box.w}px`
  frame.style.height = `${box.h}px`
  const svg = frame.querySelector<SVGSVGElement>(':scope > svg')
  if (svg) {
    svg.setAttribute('viewBox', `${box.x} ${box.y} ${box.w} ${box.h}`)
    svg.innerHTML = connectorMarkup(g, style, options)
  }
  const label = frame.querySelector<HTMLElement>(':scope > .aiws-connector-label')
  if (label) { label.style.left = `${f(g.label.point.x - box.x)}px`; label.style.top = `${f(g.label.point.y - box.y)}px` }
}
