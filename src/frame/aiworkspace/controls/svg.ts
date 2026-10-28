/* A control drawn as static SVG (脑图 TODO M4: published pages, HTML pre-rendering, previews): all expanded (no
 * reader state), the same layout, edges and node look as the canvas. Text nodes draw their measured lines; other
 * Blocks a labelled placeholder the page may replace with its own rendering. No DOM: a string. */

import { escapeAttr, escapeText } from '../import-drawio/xml.ts'
import { isTextView } from './contract.ts'
import { branchColors, edgeStrokes } from './edges.ts'
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

/** `<svg>` of the solved control `solved` (and its nested controls). */
export function controlSvg(src: ControlSource, solved: SolvedControl, opts: SvgOptions): string {
  const theme = opts.theme ?? LIGHT
  const margin = opts.margin ?? 8
  const b = solved.layout.bounds
  const origin = { x: margin - b.x, y: margin - b.y }
  const placed = placeControl(solved, origin)
  const parts: string[] = []
  for (const [id, c] of placed.controls) {
    const colors = branchColors(c.solved.layout.nodes, c.solved.config, (n) => src.get(n)?.branch_color)
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
    parts.push(`<rect x="${n.x}" y="${n.y}" width="${n.w}" height="${n.h}" rx="${root ? 10 : 8}" style="fill:${root ? theme.rootFill : theme.fill};stroke:${root ? 'none' : theme.stroke}"/>`)
    const t = typography(n.depth)
    const m = opts.measurer.measure(e.title, n.depth, n.w - NODE_PAD.x * 2)
    m.lines.forEach((line, i) => {
      const y = n.y + NODE_PAD.y + t.lineHeight * i + t.lineHeight / 2
      const spans = line.runs.map((r) => {
        const font = runFont(r, n.depth)
        const deco = [r.strike ? 'line-through' : '', r.href ? 'underline' : ''].filter(Boolean).join(' ')
        return `<tspan style="font:${escapeAttr(font)}"${deco ? ` text-decoration="${deco}"` : ''}>${escapeText(r.text)}</tspan>`
      }).join('')
      parts.push(`<text x="${n.x + NODE_PAD.x}" y="${y}" dominant-baseline="central" style="fill:${root ? theme.rootText : theme.text}">${spans}</text>`)
    })
  }
  const w = Math.ceil(b.w + margin * 2)
  const h = Math.ceil(b.h + margin * 2)
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}">${parts.join('')}</svg>`
}
