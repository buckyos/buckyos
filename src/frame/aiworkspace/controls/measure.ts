/* The size of a text node (脑图方案 §5.4, R17): its title, wrapped at a fixed maximum width in the font of its depth.
 * The browser measures with canvas `measureText`, Deno (Agents, export) with a fixed-width approximation; the two
 * differ a little, which is accepted (the Desktop supplies its canvas `TextMetrics`). Results are cached per (font, text). The node Block draws the lines computed
 * here, so what is measured is what is shown. */

import { parseInline, type Run } from './inline.ts'

export interface TextMetrics {
  /** Advance width of `text` in the CSS font `font`. */
  width(text: string, font: string): number
}

export const NODE_FONT_FAMILY = 'system-ui, -apple-system, "Segoe UI", "PingFang SC", "Hiragino Sans GB", "Microsoft YaHei", sans-serif'
export const NODE_CODE_FAMILY = 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace'

/** Typography by depth (0 = a root): size and weight in px / CSS weight. */
const SIZES = [18, 15, 14]
const WEIGHTS = [700, 600, 400]
export const LINE_HEIGHT = 1.4
/** Widest a line of text may get before it wraps. */
export const MAX_TEXT_WIDTH = 260
export const NODE_PAD = { x: 14, y: 8 }
export const MIN_NODE_WIDTH = 48

export interface NodeTypography { size: number; weight: number; lineHeight: number }

export function typography(depth: number): NodeTypography {
  const i = Math.min(Math.max(depth, 0), SIZES.length - 1)
  return { size: SIZES[i], weight: WEIGHTS[i], lineHeight: Math.round(SIZES[i] * LINE_HEIGHT) }
}

/** The CSS font of a run at `depth`. */
export function runFont(run: Run, depth: number): string {
  const t = typography(depth)
  if (run.code) return `${Math.round(t.size * 0.9)}px ${NODE_CODE_FAMILY}`
  return `${run.italic ? 'italic ' : ''}${run.bold ? Math.max(t.weight, 700) : t.weight} ${t.size}px ${NODE_FONT_FAMILY}`
}

export interface MeasuredLine { runs: Run[]; width: number }
export interface MeasuredNode { w: number; h: number; lines: MeasuredLine[] }

const WIDE = /[ᄀ-ᅟ⺀-〾ぁ-㏿㐀-䶿一-鿿ꀀ-꓏가-힣豈-﫿︰-﹏＀-｠￠-￦]/

/** Fixed-width approximation (Deno, tests): wide characters one em, others about half. Deterministic. */
export const approxMetrics: TextMetrics = {
  width(text, font) {
    const size = Number(/(\d+(?:\.\d+)?)px/.exec(font)?.[1] ?? 14)
    const bold = /\b(6|7|8|9)00\b/.test(font)
    let w = 0
    for (const ch of text) w += WIDE.test(ch) ? size : ch === ' ' ? size * 0.28 : size * (bold ? 0.6 : 0.56)
    return w
  },
}

/** Break runs into tokens a line may end between: spaces, single wide characters, words. */
function tokens(runs: Run[]): Run[] {
  const out: Run[] = []
  for (const run of runs) {
    const re = /\n|\s+|[ᄀ-ᅟ⺀-〾ぁ-㏿㐀-䶿一-鿿ꀀ-꓏가-힣豈-﫿︰-﹏＀-｠￠-￦]|[^\sᄀ-ᅟ⺀-〾ぁ-㏿㐀-䶿一-鿿ꀀ-꓏가-힣豈-﫿︰-﹏＀-｠￠-￦]+/g
    for (const m of run.text.matchAll(re)) out.push({ ...run, text: m[0] })
  }
  return out
}

/** Greedy line filling at `max` (a word wider than a line is cut by characters). */
export function wrapRuns(runs: Run[], depth: number, metrics: TextMetrics, max = MAX_TEXT_WIDTH): MeasuredLine[] {
  const lines: MeasuredLine[] = []
  let line: MeasuredLine = { runs: [], width: 0 }
  const append = (tok: Run, w: number) => {
    const last = line.runs[line.runs.length - 1]
    if (last && sameStyle(last, tok)) last.text += tok.text
    else line.runs.push({ ...tok })
    line.width += w
  }
  const breakLine = () => {
    // trailing spaces do not count
    const last = line.runs[line.runs.length - 1]
    if (last && /\s$/.test(last.text)) {
      const trimmed = last.text.replace(/\s+$/, '')
      line.width -= metrics.width(last.text, runFont(last, depth)) - metrics.width(trimmed, runFont(last, depth))
      if (trimmed) last.text = trimmed
      else line.runs.pop()
    }
    lines.push(line)
    line = { runs: [], width: 0 }
  }
  for (const tok of tokens(runs)) {
    if (tok.text === '\n') { breakLine(); continue }
    const font = runFont(tok, depth)
    const space = /^\s+$/.test(tok.text)
    if (space && line.runs.length === 0) continue
    const w = metrics.width(tok.text, font)
    if (line.width + w <= max || line.runs.length === 0 && w <= max || space) { append(tok, w); continue }
    if (w <= max) { breakLine(); append(tok, w); continue }
    // a word wider than a line: by characters
    for (const ch of tok.text) {
      const cw = metrics.width(ch, font)
      if (line.width + cw > max && line.runs.length > 0) breakLine()
      append({ ...tok, text: ch }, cw)
    }
  }
  if (line.runs.length > 0 || lines.length === 0) breakLine()
  return lines
}

function sameStyle(a: Run, b: Run): boolean {
  return !!a.bold === !!b.bold && !!a.italic === !!b.italic && !!a.code === !!b.code && !!a.strike === !!b.strike && a.href === b.href
}

/** Measures node titles, with a cache per (depth, title). */
export class NodeMeasurer {
  private readonly cache = new Map<string, MeasuredNode>()
  readonly metrics: TextMetrics
  constructor(metrics: TextMetrics = approxMetrics) { this.metrics = metrics }

  /** `maxText`: the text width a pinned node width leaves (its height then follows the wrapped text). */
  measure(title: string | null | undefined, depth: number, maxText = MAX_TEXT_WIDTH): MeasuredNode {
    const text = title ?? ''
    const key = `${Math.min(depth, SIZES.length - 1)}\u0000${Math.round(maxText)}\u0000${text}`
    const hit = this.cache.get(key)
    if (hit) return hit
    if (this.cache.size > 20_000) this.cache.clear()
    const t = typography(depth)
    const lines = wrapRuns(parseInline(text), depth, this.metrics, maxText)
    const textW = Math.max(0, ...lines.map((l) => l.width))
    const out: MeasuredNode = {
      w: Math.ceil(Math.max(MIN_NODE_WIDTH, textW + NODE_PAD.x * 2)),
      h: Math.ceil(lines.length * t.lineHeight + NODE_PAD.y * 2),
      lines,
    }
    this.cache.set(key, out)
    return out
  }
}
