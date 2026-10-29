/* Text (PDF 导入方案 §4.1): `getTextContent` items paired with the `showText` runs that drew them (colour, alpha,
 * rendering mode, font), joined into lines, the lines into paragraphs, one `shape: text` per paragraph.
 *
 *   lines        same block, same direction, baselines within 0.2 × size (0.5 for smaller super/subscripts), and
 *                the gap to the line's end under 1.6 × size; a gap over 0.15 × size reads as a space
 *   paragraphs   tagged PDFs: one per structure block (P, H1…H6, LI, TD, Caption…), split where its lines are not
 *                one column; untagged: lines of one size and colour, pitch 0.9–1.75 × size and steady, aligned
 *                left (a first-line indent allowed), centred or right
 *   box          text lines joined with `\n`, not reflowed (`wrap: false`); the box is placed so the browser's line
 *                boxes (padding 4 × 6, `line_height` = the pitch) put each baseline where the PDF has it, with 6 %
 *                width to spare for a substitute font
 *   unreadable   U+FFFD, private-use and control characters: a page with too many is drawn whole (§4.1 第 6 条) */

import type { PdfFont, PdfPage, PdfStructNode } from './data.ts'
import { apply, mul, scaleOf, type Mat, type TextRun } from './interpret.ts'
import type { Point, Rect } from '../import-drawio/geometry.ts'

/** Block-level structure roles: the text inside one of them is one paragraph. */
const BLOCK_ROLES = new Set(['P', 'H', 'H1', 'H2', 'H3', 'H4', 'H5', 'H6', 'LI', 'TD', 'TH', 'Caption', 'Title', 'BlockQuote', 'Note', 'TOCI', 'Formula', 'Index', 'BibEntry', 'Code'])
/** Roles that are only containers of blocks (a `Code` inside a `P` is inline: the `P` wins, being further out). */
const INLINE_INSIDE = new Set(['Code', 'Formula'])

export interface StructInfo {
  /** Marked content id → its block (an id unique on the page) and role. */
  blocks: Map<string, { id: string; role: string }>
  /** Marked content number → the `Alt` of the figure it is in. */
  figureAlt: Map<number, string>
}

const mcNumber = (id: string) => { const m = /_mc(\d+)$/.exec(id); return m ? Number(m[1]) : null }

export function readStruct(root: PdfStructNode | null): StructInfo {
  const info: StructInfo = { blocks: new Map(), figureAlt: new Map() }
  if (!root) return info
  let seq = 0
  const walk = (node: PdfStructNode, block: { id: string; role: string } | null, alt: string | null, depth: number) => {
    if (depth > 64) return
    const role = node.role ?? ''
    let here = block
    // the outermost block wins, except an inline role inside one
    if (BLOCK_ROLES.has(role) && (!block || !INLINE_INSIDE.has(role))) here = block && !INLINE_INSIDE.has(block.role) ? block : { id: `b${seq++}`, role }
    const figureAlt = role === 'Figure' ? node.alt ?? alt ?? null : alt
    for (const child of node.children ?? []) {
      if ('type' in child) {
        if (child.type !== 'content') continue
        if (here) info.blocks.set(child.id, here)
        const n = mcNumber(child.id)
        if (figureAlt && n !== null) info.figureAlt.set(n, figureAlt)
      } else walk(child, here, figureAlt, depth + 1)
    }
  }
  walk(root, null, null, 0)
  return info
}

export interface TextStyle { color: string; alpha: number; bold: boolean; italic: boolean; family: 'sans' | 'serif' | 'mono' }

interface Piece {
  str: string
  order: number
  origin: Point
  dir: Point
  /** World length along the baseline and the font size. */
  len: number
  size: number
  angle: number
  block: { id: string; role: string } | null
  style: TextStyle
  runs: number[]
}

export interface TextLine { pieces: Piece[]; text: string; x0: number; x1: number; baseline: number; size: number; style: TextStyle; angle: number; origin: Point; dir: Point; len: number; block: Piece['block'] }

export interface Paragraph {
  lines: TextLine[]
  text: string
  /** World box (before rotation) and rotation about its centre. */
  rect: Rect
  rotation: number
  fontSize: number
  lineHeight: number | null
  align: 'left' | 'center' | 'right'
  style: TextStyle
  role: string | null
  /** Operator index of the first run drawing it (stacking order), Infinity when none was found. */
  order: number
  runs: number[]
  mixed: boolean
}

export interface TextResult {
  paragraphs: Paragraph[]
  /** `showText` operators drawn by the objects (filtered out of the raster). */
  converted: Set<number>
  /** Characters that cannot be read vs. all (non-space) characters. */
  unreadable: number
  total: number
  invisible: number
  vertical: number
}

// deno-lint-ignore no-control-regex
const BAD = /[\uFFFD\uE000-\uF8FF\u0000-\u0008\u000E-\u001F]/g
const CJK = /[⺀-鿿가-힯豈-﫿＀-￯]/
const isSpace = (s: string) => /^\s*$/.test(s)

function familyOf(font: PdfFont | undefined): TextStyle['family'] {
  if (!font) return 'sans'
  if (font.mono || /courier|consol|mono|menlo/i.test(font.name)) return 'mono'
  if (font.serif || /times|georgia|garamond|serif|song|ming|simsun|nsimsun|fangsong|kai|mincho|cambria|palatino|book/i.test(font.name)) return /sans/i.test(font.name) ? 'sans' : 'serif'
  return 'sans'
}

/** The runs that drew an item: on its baseline, overlapping it along it. The first is the one overlapping most
 * (its colour and rendering mode are the item's); a run drawing several items belongs to each. */
function pairRuns(pieces: { origin: Point; dir: Point; len: number; size: number }[], runs: TextRun[]): TextRun[][] {
  const buckets = new Map<number, TextRun[]>()
  const B = 6
  for (const r of runs) { const k = Math.floor(r.start.y / B); const list = buckets.get(k) ?? []; list.push(r); buckets.set(k, list) }
  return pieces.map((p) => {
    const tol = Math.max(0.35 * p.size, 0.5)
    const hits: { run: TextRun; score: number }[] = []
    const consider = (r: TextRun) => {
      const sx = r.start.x - p.origin.x, sy = r.start.y - p.origin.y
      const ex = r.end.x - p.origin.x, ey = r.end.y - p.origin.y
      const perp = Math.abs(-p.dir.y * sx + p.dir.x * sy)
      if (perp > tol) return
      const a = p.dir.x * sx + p.dir.y * sy, b = p.dir.x * ex + p.dir.y * ey
      const lo = Math.min(a, b), hi = Math.max(a, b)
      const overlap = Math.min(hi, p.len) - Math.max(lo, 0)
      // an empty item or run (a space, a zero-width glyph) meets what starts at its spot
      if (overlap > 0.01 || (Math.abs(lo) < Math.max(0.2 * p.size, 0.5) && (p.len < 0.01 || hi - lo < 0.01))) hits.push({ run: r, score: overlap - 0.01 * perp })
    }
    if (Math.abs(p.dir.y) < 0.02) {
      const k = Math.floor(p.origin.y / B), span = Math.ceil(tol / B) + 1
      for (let j = k - span; j <= k + span; j++) for (const r of buckets.get(j) ?? []) consider(r)
    } else for (const r of runs) consider(r)
    return hits.sort((x, y) => y.score - x.score).map((h) => h.run)
  })
}

const majority = <T>(items: { value: T; weight: number }[], key: (v: T) => string): T | undefined => {
  const sums = new Map<string, { value: T; w: number }>()
  for (const { value, weight } of items) { const k = key(value); const e = sums.get(k) ?? { value, w: 0 }; e.w += weight; sums.set(k, e) }
  return [...sums.values()].sort((a, b) => b.w - a.w)[0]?.value
}
const styleKey = (s: TextStyle) => `${s.color}|${s.alpha}|${s.bold}|${s.italic}|${s.family}`

function makeLine(pieces: Piece[]): TextLine {
  pieces.sort((a, b) => a.origin.x * a.dir.x + a.origin.y * a.dir.y - (b.origin.x * b.dir.x + b.origin.y * b.dir.y))
  // spaces at either end are gaps between blocks, not text
  while (pieces.length > 1 && isSpace(pieces[0].str)) pieces.shift()
  while (pieces.length > 1 && isSpace(pieces[pieces.length - 1].str)) pieces.pop()
  const first = pieces[0]
  const weight = (p: Piece) => Math.max(1, p.str.trim().length)
  const size = majority(pieces.filter((p) => !isSpace(p.str)).map((p) => ({ value: p.size, weight: weight(p) })), (v) => v.toFixed(1)) ?? first.size
  let text = ''
  let end = -Infinity
  const along = (p: Point) => p.x * first.dir.x + p.y * first.dir.y
  for (const p of pieces) {
    const start = along(p.origin)
    if (text && start - end > 0.15 * size && !/\s$/.test(text) && !/^\s/.test(p.str)) text += ' '
    text += isSpace(p.str) ? ' ' : p.str
    if (!isSpace(p.str)) end = Math.max(end, start + p.len)
  }
  const main = pieces.filter((p) => !isSpace(p.str) && Math.abs(p.size - size) < 0.2 * size)
  const ref = main[0] ?? first
  const x0 = Math.min(...pieces.map((p) => along(p.origin)))
  const style = majority(pieces.filter((p) => !isSpace(p.str)).map((p) => ({ value: p.style, weight: weight(p) })), styleKey) ?? first.style
  // the baseline: the main pieces' perpendicular coordinate
  const perp = (p: Point) => -first.dir.y * p.x + first.dir.x * p.y
  const baseline = main.length ? main.reduce((s, p) => s + perp(p.origin), 0) / main.length : perp(first.origin)
  const origin = { x: first.dir.x * x0 - first.dir.y * baseline, y: first.dir.y * x0 + first.dir.x * baseline }
  return { pieces, text: text.trim().replace(/ {2,}/g, ' '), x0, x1: end, baseline, size, style, angle: ref.angle, origin, dir: first.dir, len: end - x0, block: first.block }
}

const PAD_X = 6
const PAD_Y = 4
const DEFAULT_LH = 1.25
const SLACK = 0.06

/** Text lines → one paragraph object (box, alignment, line height) in world units. */
function makeParagraph(lines: TextLine[]): Paragraph {
  const first = lines[0]
  const fs = first.size
  let lineHeight: number | null = null
  if (lines.length > 1) {
    const pitch = (lines.at(-1)!.baseline - first.baseline) / (lines.length - 1)
    const lh = Math.min(3, Math.max(0.8, pitch / fs))
    lineHeight = Math.abs(lh - DEFAULT_LH) < 0.015 ? null : Math.round(lh * 100) / 100
  }
  const lh = lineHeight ?? DEFAULT_LH
  const tol = 0.6 * fs
  const left = Math.min(...lines.map((l) => l.x0))
  const right = Math.max(...lines.map((l) => l.x1))
  const body = lines.length > 1 ? lines.slice(1) : lines
  const bodyLeft = Math.min(...body.map((l) => l.x0))
  let align: Paragraph['align'] = 'left'
  if (lines.length > 1) {
    const lefts = body.every((l) => Math.abs(l.x0 - bodyLeft) < tol) && first.x0 >= bodyLeft - tol
    const centres = lines.every((l) => Math.abs((l.x0 + l.x1) / 2 - (lines[0].x0 + lines[0].x1) / 2) < tol)
    const rights = lines.every((l) => Math.abs(l.x1 - right) < tol)
    align = lefts ? 'left' : centres ? 'center' : rights ? 'right' : 'left'
  }
  const texts = lines.map((l, i) => {
    // a first-line indent as em / en spaces (they keep their width in any font)
    if (i === 0 && align === 'left' && lines.length > 1 && l.x0 - bodyLeft > 0.4 * fs) {
      const em = (l.x0 - bodyLeft) / fs
      return ' '.repeat(Math.floor(em)) + (em % 1 >= 0.35 ? ' ' : '') + l.text
    }
    return l.text
  })
  const text = texts.join('\n')
  const cjk = CJK.test(text)
  const ascent = cjk ? 0.88 : 0.905, descent = cjk ? 0.12 : 0.212
  const above = (lh - (ascent + descent)) / 2 + ascent
  const startX = align === 'left' ? bodyLeft : left
  const textW = right - startX
  const boxW = textW * (1 + SLACK) + 2 * PAD_X
  const boxH = lines.length * lh * fs + 2 * PAD_Y
  // in the line's own frame: u along the baseline from the paragraph's left, v down from the first baseline
  const u0 = align === 'right' ? right - boxW + PAD_X : align === 'center' ? (left + right) / 2 - boxW / 2 : startX - PAD_X
  const v0 = -(PAD_Y + above * fs)
  const cu = u0 + boxW / 2, cv = v0 + boxH / 2
  const d = first.dir
  const centre = { x: d.x * cu - d.y * (first.baseline + cv), y: d.y * cu + d.x * (first.baseline + cv) }
  const runs = [...new Set(lines.flatMap((l) => l.pieces.flatMap((p) => p.runs)))]
  const styles = lines.flatMap((l) => l.pieces.filter((p) => !isSpace(p.str)).map((p) => ({ value: p.style, weight: p.str.trim().length })))
  const style = majority(styles, styleKey) ?? first.style
  return {
    lines, text, rect: { x: centre.x - boxW / 2, y: centre.y - boxH / 2, w: boxW, h: boxH }, rotation: first.angle, fontSize: fs, lineHeight, align, style,
    role: first.block?.role ?? null, order: runs.length ? Math.min(...runs) : Infinity, runs,
    mixed: new Set(styles.map((s) => styleKey(s.value))).size > 1 || lines.some((l) => Math.abs(l.size - fs) > 0.15 * fs),
  }
}

/** Untagged text: lines that read as one paragraph (one size and colour, a steady pitch, one alignment). */
function geometricParagraphs(lines: TextLine[]): TextLine[][] {
  const sorted = [...lines].sort((a, b) => a.baseline - b.baseline || a.x0 - b.x0)
  const open: { lines: TextLine[]; pitch: number | null; left: number }[] = []
  const out: TextLine[][] = []
  for (const line of sorted) {
    const fs = line.size
    let best: (typeof open)[number] | null = null
    let bestGap = Infinity
    for (const p of open) {
      const last = p.lines.at(-1)!
      if (Math.abs(last.size - fs) > 0.1 * fs || last.style.color !== line.style.color || Math.abs(last.angle) > 0.5 || Math.abs(line.angle) > 0.5) continue
      const pitch = line.baseline - last.baseline
      if (pitch < 0.9 * fs || pitch > 1.75 * fs) continue
      if (p.pitch !== null && Math.abs(pitch - p.pitch) > 0.15 * fs) continue
      if (Math.min(line.x1, last.x1) - Math.max(line.x0, last.x0) <= 0) continue
      const tol = 0.6 * fs
      const indentOk = p.lines.length === 1 && line.x0 < last.x0 && last.x0 - line.x0 < 4 * fs
      const aligned = Math.abs(line.x0 - p.left) < tol || indentOk || Math.abs((line.x0 + line.x1) / 2 - (last.x0 + last.x1) / 2) < tol || Math.abs(line.x1 - last.x1) < tol
      if (!aligned) continue
      if (pitch < bestGap) { bestGap = pitch; best = p }
    }
    if (best) {
      const last = best.lines.at(-1)!
      if (best.lines.length === 1) best.left = Math.min(best.left, line.x0)
      best.pitch = best.pitch ?? line.baseline - last.baseline
      best.lines.push(line)
    } else {
      const p = { lines: [line], pitch: null, left: line.x0 }
      open.push(p)
      out.push(p.lines)
    }
    // paragraphs far above can no longer grow
    for (let i = open.length - 1; i >= 0; i--) if (line.baseline - open[i].lines.at(-1)!.baseline > 2 * open[i].lines.at(-1)!.size) open.splice(i, 1)
  }
  return out
}

/** A tagged block's lines, split where they stop being one column (another size, a big gap, no overlap). */
function blockParagraphs(lines: TextLine[]): TextLine[][] {
  const sorted = [...lines].sort((a, b) => a.baseline - b.baseline || a.x0 - b.x0)
  const out: TextLine[][] = []
  let cur: TextLine[] = []
  for (const line of sorted) {
    const last = cur.at(-1)
    const fs = line.size
    const apart = last && (Math.abs(last.size - fs) > 0.2 * fs || line.baseline - last.baseline > 2.2 * Math.max(fs, last.size) || line.baseline - last.baseline < 0.5 * fs
      || Math.min(line.x1, last.x1) - Math.max(line.x0, last.x0) < -fs || Math.abs(line.angle) > 0.5)
    if (last && apart) { out.push(cur); cur = [] }
    cur.push(line)
  }
  if (cur.length) out.push(cur)
  return out
}

export interface TextOptions { unit: 'paragraph' | 'line' }

/** Group a page's text into paragraphs (world coordinates through `world`). */
export function groupText(page: PdfPage, world: Mat, runs: TextRun[], struct: StructInfo, options: TextOptions): TextResult {
  const result: TextResult = { paragraphs: [], converted: new Set(), unreadable: 0, total: 0, invisible: 0, vertical: 0 }
  const s = scaleOf(world)
  const raw: { item: PdfPage['text'][number]; origin: Point; dir: Point; len: number; size: number; angle: number; order: number }[] = []
  for (const [order, item] of page.text.entries()) {
    if (!item.str) continue
    const m = mul(world, item.transform)
    const len0 = Math.hypot(m[0], m[1])
    if (len0 === 0) continue
    const dir = { x: m[0] / len0, y: m[1] / len0 }
    raw.push({ item, origin: apply(m, 0, 0), dir, len: item.width * s, size: Math.hypot(m[2], m[3]), angle: Math.round((Math.atan2(m[1], m[0]) * 180) / Math.PI * 100) / 100, order })
  }
  const paired = pairRuns(raw, runs.filter((r) => r.complex === null || r.complex === 'text_clip'))
  const pieces: Piece[] = []
  for (const [k, r] of raw.entries()) {
    const font = page.fonts[r.item.fontName]
    const covering = paired[k]
    const run = covering[0] ?? null
    const chars = r.item.str.replace(/\s/g, '')
    const bad = (chars.match(BAD) ?? []).length
    result.total += chars.length
    result.unreadable += bad
    if (font?.vertical) { result.vertical += 1; continue }
    if (run && (run.mode & 3) === 3) { if (chars) result.invisible += 1; continue }
    if (run && (run.complex !== null || (run.fill && run.fill.kind !== 'color'))) continue
    if (chars && bad === chars.length) continue
    const color = run?.fill?.kind === 'color' ? run.fill.color : '#000000'
    pieces.push({
      str: r.item.str.replace(BAD, ''), order: r.order, origin: r.origin, dir: r.dir, len: r.len, size: r.size, angle: r.angle,
      block: r.item.mc ? struct.blocks.get(r.item.mc) ?? null : null,
      style: { color: /^#[0-9a-f]{6}$/.test(color) ? color : '#000000', alpha: run?.alpha ?? 1, bold: font?.bold ?? false, italic: font?.italic ?? false, family: familyOf(font) },
      runs: covering.map((c) => c.index),
    })
  }
  // lines: consecutive pieces in reading order, then neighbours on one baseline merged
  const lines: TextLine[] = []
  let cur: Piece[] = []
  let curEnd = 0
  const fits = (p: Piece, line: Piece[], end: number) => {
    const ref = line.find((q) => !isSpace(q.str)) ?? line[0]
    if (p.block?.id !== ref.block?.id || Math.abs(p.angle - ref.angle) > 0.5) return false
    const size = Math.max(ref.size, p.size)
    const perp = Math.abs(-ref.dir.y * (p.origin.x - ref.origin.x) + ref.dir.x * (p.origin.y - ref.origin.y))
    const smaller = p.size < 0.8 * ref.size || ref.size < 0.8 * p.size
    if (perp > (smaller ? 0.5 : 0.2) * size) return false
    if (isSpace(p.str) || end === -Infinity) return true
    const start = p.origin.x * ref.dir.x + p.origin.y * ref.dir.y
    const gap = start - end
    return gap > -0.5 * size && gap < 1.6 * size
  }
  for (const p of pieces) {
    // a space (pdf.js writes wide gaps as one) never bridges a gap: the gap is measured from the last text
    if (cur.length && !fits(p, cur, curEnd)) { lines.push(makeLine(cur)); cur = [] }
    const dir = (cur[0] ?? p).dir
    const end = p.origin.x * dir.x + p.origin.y * dir.y + p.len
    if (!isSpace(p.str)) curEnd = cur.some((q) => !isSpace(q.str)) ? Math.max(curEnd, end) : end
    else if (!cur.length) curEnd = -Infinity
    cur.push(p)
  }
  if (cur.length) lines.push(makeLine(cur))
  const merged: TextLine[] = []
  const byBaseline = [...lines].sort((a, b) => (a.block?.id ?? '').localeCompare(b.block?.id ?? '') || a.angle - b.angle || a.baseline - b.baseline || a.x0 - b.x0)
  for (const line of byBaseline) {
    const last = merged.at(-1)
    if (last && last.block?.id === line.block?.id && Math.abs(last.angle - line.angle) <= 0.5 && Math.abs(last.baseline - line.baseline) < 0.2 * Math.max(last.size, line.size)
      && line.x0 - last.x1 > -0.5 * line.size && line.x0 - last.x1 < 1.6 * Math.max(last.size, line.size)) {
      merged[merged.length - 1] = makeLine([...last.pieces, ...line.pieces])
    } else merged.push(line)
  }
  const visible = merged.filter((l) => l.text.trim() !== '')
  let groups: TextLine[][]
  if (options.unit === 'line') groups = visible.map((l) => [l])
  else {
    const tagged = new Map<string, TextLine[]>()
    const loose: TextLine[] = []
    for (const l of visible) {
      if (l.block && Math.abs(l.angle) <= 0.5) { const list = tagged.get(l.block.id) ?? []; list.push(l); tagged.set(l.block.id, list) }
      else if (Math.abs(l.angle) > 0.5) tagged.set(`r${tagged.size}`, [l])
      else loose.push(l)
    }
    groups = [...[...tagged.values()].flatMap(blockParagraphs), ...geometricParagraphs(loose)]
  }
  for (const g of groups) {
    const p = makeParagraph(g)
    result.paragraphs.push(p)
    for (const r of p.runs) result.converted.add(r)
  }
  result.paragraphs.sort((a, b) => a.order - b.order)
  return result
}
