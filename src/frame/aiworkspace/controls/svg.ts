/* A control drawn as static SVG (脑图 TODO M4: published pages, HTML pre-rendering, previews): all expanded (no
 * reader state), the same layout, edges and node look as the canvas. Text nodes draw their measured lines; other
 * Blocks a labelled placeholder the page may replace with its own rendering. No DOM: a string. */

import { escapeAttr, escapeText } from '../import-drawio/xml.ts'
import { isTextView } from './contract.ts'
import { branchColors, edgeStrokes } from './edges.ts'
import { LABEL_ICON_GAP, labelBackgroundImage, labelIcon, readLabelLook } from './label.ts'
import { NODE_PAD, typography, runFont, type NodeMeasurer } from './measure.ts'
import { placeControl, type ControlSource, type SolvedControl } from './solve.ts'

export interface SvgOptions {
  measurer: NodeMeasurer
  /** Node fill / text colours (the canvas theme's light values by default); written as CSS, so `var(--…)` works in a page. */
  theme?: { fill: string; rootFill: string; text: string; rootText: string; stroke: string }
  /** Pixels around the box. */
  margin?: number
}

const LIGHT = { fill: '#ffffff', rootFill: '#1f2937', text: '#1f2937', rootText: '#ffffff', stroke: '#d0d5dd' }

// SVG fill also accepts paint-server URLs. Document colour fields select plain colours.
const svgColor = (color: unknown) => typeof color === 'string' && /^(?:#[0-9a-f]{3,8}|[a-z]+)$/i.test(color) && color !== 'auto' ? color : undefined

/** `<svg>` of the solved control `solved` (and its nested controls). */
export function controlSvg(src: ControlSource, solved: SolvedControl, opts: SvgOptions): string {
  const theme = opts.theme ?? LIGHT
  const margin = opts.margin ?? 8
  const b = solved.layout.bounds
  const origin = { x: margin - b.x, y: margin - b.y }
  const placed = placeControl(solved, origin)
  const parts: string[] = []
  const borders = new Map<string, string>()
  for (const [id, c] of placed.controls) {
    const colors = branchColors(c.solved.layout.nodes, c.solved.config, (n) => src.get(n)?.branch_color)
    for (const [nodeId, color] of colors) borders.set(nodeId, color)
    for (const s of edgeStrokes(c.solved.layout.nodes, c.solved.config, colors, c.origin)) {
      parts.push(`<path d="${s.d}" style="fill:none;stroke:${escapeAttr(s.color)}" stroke-width="${s.width}" stroke-linecap="round"/>`)
    }
    if (id !== solved.id) parts.push(`<rect x="${c.box.x}" y="${c.box.y}" width="${c.box.w}" height="${c.box.h}" rx="10" style="fill:none;stroke:${theme.stroke}" stroke-dasharray="4 3"/>`)
  }
  for (const n of placed.nodes.values()) {
    if (placed.controls.has(n.id)) continue
    const e = src.get(n.id)
    const root = n.depth === 0
    if (!e || e.type_id !== 'buckyos.cell' || !isTextView(e.view_type)) {
      parts.push(`<rect x="${n.x}" y="${n.y}" width="${n.w}" height="${n.h}" rx="6" style="fill:${theme.fill};stroke:${theme.stroke}"/>`)
      parts.push(`<text x="${n.x + n.w / 2}" y="${n.y + n.h / 2}" text-anchor="middle" dominant-baseline="central" font-size="12" style="fill:${theme.text}">${escapeText(e?.title || `[${e?.view_type ?? e?.kind ?? 'block'}]`)}</text>`)
      continue
    }
    const look = readLabelLook(e.config)
    const radius = root ? 10 : 8
    const bounds = `x="${n.x}" y="${n.y}" width="${n.w}" height="${n.h}"`
    const fill = svgColor(look.background_color) ?? (root ? theme.rootFill : theme.fill)
    const ink = svgColor(look.font_color) ?? (root ? theme.rootText : theme.text)
    const stroke = n.depth === 1 ? borders.get(n.id) ?? theme.stroke : theme.stroke
    parts.push(`<rect ${bounds} rx="${radius}" fill="${escapeAttr(fill)}"/>`)
    const image = labelBackgroundImage(look.background_image)
    if (image) parts.push(`<image ${bounds} href="${escapeAttr(image)}" preserveAspectRatio="xMidYMid slice" style="clip-path:inset(0 round ${radius}px) fill-box"/>`)
    if (!root) parts.push(`<rect ${bounds} rx="${radius}" fill="none" stroke="${escapeAttr(stroke)}" stroke-width="${n.depth === 1 ? 2 : 1}"/>`)

    const t = typography(n.depth, look)
    const inner = n.w - NODE_PAD.x * 2
    const m = opts.measurer.measure(e.title, n.depth, inner, look)
    const icon = labelIcon(look)
    const iconWidth = icon ? t.size + LABEL_ICON_GAP : 0
    const width = Math.max(0, ...m.lines.map(line => line.width))
    const align = look.align === 'right' ? 1 : look.align === 'center' ? 0.5 : 0
    const left = n.x + NODE_PAD.x + (inner - width - iconWidth) * align
    const rightIcon = look.icon_position === 'right'
    const textLeft = left + (rightIcon ? 0 : iconWidth)
    const valign = look.valign === 'top' ? 0 : look.valign === 'bottom' ? 1 : 0.5
    const top = n.y + NODE_PAD.y + (n.h - 2 * NODE_PAD.y - m.lines.length * t.lineHeight) * valign
    if (icon) {
      const x = left + (rightIcon ? width + LABEL_ICON_GAP : 0) + t.size / 2
      const y = n.y + NODE_PAD.y + (n.h - 2 * NODE_PAD.y - t.size) * valign + t.size / 2
      parts.push(`<text x="${x}" y="${y}" dominant-baseline="central" text-anchor="middle" font-size="${t.size}" font-family="system-ui, sans-serif" fill="${escapeAttr(ink)}" aria-hidden="true">${escapeText(icon)}</text>`)
    }
    m.lines.forEach((line, i) => {
      const x = textLeft + (width - line.width) * align
      const y = top + t.lineHeight * (i + 0.5)
      const spans = line.runs.map((r) => {
        const font = runFont(r, n.depth, look)
        const deco = [look.strike || r.strike ? 'line-through' : '', r.href ? 'underline' : ''].filter(Boolean).join(' ')
        return `<tspan style="font:${escapeAttr(font)}"${deco ? ` text-decoration="${deco}"` : ''}>${escapeText(r.text)}</tspan>`
      }).join('')
      parts.push(`<text x="${x}" y="${y}" dominant-baseline="central" xml:space="preserve" fill="${escapeAttr(ink)}">${spans}</text>`)
    })
  }
  const w = Math.ceil(b.w + margin * 2)
  const h = Math.ceil(b.h + margin * 2)
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}">${parts.join('')}</svg>`
}
