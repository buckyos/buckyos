/* DrawingML text (Office 导入 TODO §4.2–§4.3): `a:txBody` paragraphs and runs with their properties resolved down the
 * inheritance chain — the presentation's default text style, the master's title / body / other style, the master's
 * and the layout's placeholder list styles, the shape's own list style, the paragraph's `a:pPr` and the run's
 * `a:rPr`. A level's properties (`a:lvlNpPr`) hold alignment, indents, line spacing, bullets and the default run
 * (`a:defRPr`): size, bold, italic, underline, strike, colour, Latin / East Asian face.
 *
 * The canvas `shape` Block shows one text style per shape, so `shapeText` flattens a body into one string (bullets
 * and numbers as prefixes, levels as indents) and the style most of its characters have, and says when runs
 * differ (§12: the majority, not the first run — a bold lead-in would otherwise make a whole list bold). */

import type { XmlElement } from '../import-drawio/xml.ts'
import { attr, kid, kids, local } from './package.ts'
import { colorIn, themeFont, type ColorContext, type Rgba, type Theme } from './theme.ts'
import { fraction, int, onOff } from './units.ts'

export interface RunStyle {
  /** Points. */
  size?: number
  bold?: boolean
  italic?: boolean
  underline?: boolean
  strike?: boolean
  /** The colour's fill element (resolved late: the colour map and the shape's style colour apply). */
  fill?: XmlElement
  latin?: string
  ea?: string
  caps?: boolean
}

export interface ParaStyle {
  align?: string
  /** Line spacing: a multiple of single spacing, or points. */
  lineSpacing?: { pct: number } | { pt: number }
  bullet?: { kind: 'none' } | { kind: 'char'; char: string } | { kind: 'auto'; scheme: string; start: number }
  /** The bullet's face (`a:buFont`): Wingdings / Symbol characters are mapped to Unicode. */
  bulletFont?: string
  bulletColor?: XmlElement
  run: RunStyle
}

/** Paragraph styles by level (0–8). */
export type LevelStyles = (ParaStyle | undefined)[]

export function readRunStyle(rPr: XmlElement | undefined): RunStyle {
  const out: RunStyle = {}
  if (!rPr) return out
  const sz = attr(rPr, 'sz')
  if (sz !== undefined) out.size = int(sz) / 100
  if (attr(rPr, 'b') !== undefined) out.bold = onOff(attr(rPr, 'b'), true)
  if (attr(rPr, 'i') !== undefined) out.italic = onOff(attr(rPr, 'i'), true)
  const u = attr(rPr, 'u')
  if (u !== undefined) out.underline = u !== 'none'
  const strike = attr(rPr, 'strike')
  if (strike !== undefined) out.strike = strike !== 'noStrike'
  const cap = attr(rPr, 'cap')
  if (cap !== undefined) out.caps = cap === 'all'
  const fill = kid(rPr, 'solidFill') ?? kid(rPr, 'gradFill') ?? kid(rPr, 'noFill')
  if (fill) out.fill = fill
  const latin = attr(kid(rPr, 'latin'), 'typeface')
  if (latin) out.latin = latin
  const ea = attr(kid(rPr, 'ea'), 'typeface')
  if (ea) out.ea = ea
  return out
}

export function readParaStyle(pPr: XmlElement | undefined): ParaStyle {
  const out: ParaStyle = { run: readRunStyle(kid(pPr, 'defRPr')) }
  if (!pPr) return out
  const algn = attr(pPr, 'algn')
  if (algn) out.align = algn
  const ln = kid(pPr, 'lnSpc')
  const pct = kid(ln, 'spcPct'), pts = kid(ln, 'spcPts')
  if (pct) out.lineSpacing = { pct: fraction(attr(pct, 'val'), 1) }
  else if (pts) out.lineSpacing = { pt: int(attr(pts, 'val')) / 100 }
  if (kid(pPr, 'buNone')) out.bullet = { kind: 'none' }
  else if (kid(pPr, 'buChar')) out.bullet = { kind: 'char', char: attr(kid(pPr, 'buChar'), 'char') ?? '•' }
  else if (kid(pPr, 'buAutoNum')) out.bullet = { kind: 'auto', scheme: attr(kid(pPr, 'buAutoNum'), 'type') ?? 'arabicPeriod', start: int(attr(kid(pPr, 'buAutoNum'), 'startAt'), 1) }
  const buClr = kid(pPr, 'buClr')
  if (buClr) out.bulletColor = buClr
  const buFont = attr(kid(pPr, 'buFont'), 'typeface')
  if (buFont) out.bulletFont = buFont
  return out
}

/** Bullet characters of the symbol fonts PowerPoint uses, as the Unicode characters they draw. */
const SYMBOL_BULLETS: Record<string, Record<string, string>> = {
  wingdings: { Ø: '➢', '§': '▪', ü: '✓', l: '●', n: '■', q: '❑', v: '❖', 'è': '➔', 'à': '➢', 'Ü': '✓', 'o': '□', 'p': '◻', 'ª': '■', 'Ÿ': '•', 'ð': '➪', 'w': '◆', 'u': '◆', 't': '◆' },
  symbol: { '·': '•', 'Þ': '⇒', 'Ö': '√', '®': '→', '¨': '♦', '¾': '—' },
}
function bulletChar(char: string, font: string | undefined): string {
  const table = font ? SYMBOL_BULLETS[font.toLowerCase().replace(/\s+\d+$/, '')] : undefined
  const code = char.codePointAt(0) ?? 0
  // symbol fonts are often written in the private use area (U+F0xx)
  const plain = code >= 0xf000 && code <= 0xf0ff ? String.fromCharCode(code - 0xf000) : char
  return table?.[plain] ?? (table ? '•' : char)
}

/** `a:lstStyle`, a master's `p:titleStyle` / `p:bodyStyle`, or `defaultTextStyle`. */
export function readLevels(el: XmlElement | undefined): LevelStyles {
  const out: LevelStyles = []
  if (!el) return out
  const def = kid(el, 'defPPr')
  for (let k = 0; k < 9; k++) {
    const lvl = kid(el, `lvl${k + 1}pPr`)
    if (lvl || def) out[k] = lvl ? merge(def ? readParaStyle(def) : undefined, readParaStyle(lvl)) : readParaStyle(def)
  }
  return out
}

function mergeRun(a: RunStyle | undefined, b: RunStyle | undefined): RunStyle {
  return { ...(a ?? {}), ...(b ?? {}) }
}

export function merge(a: ParaStyle | undefined, b: ParaStyle | undefined): ParaStyle {
  if (!a) return b ?? { run: {} }
  if (!b) return a
  return { ...a, ...b, run: mergeRun(a.run, b.run) }
}

export interface TextRun { text: string; style: RunStyle }
export interface TextPara { level: number; style: ParaStyle; runs: TextRun[] }

/** The paragraphs of a text body with their styles merged down `chain` (farthest first). */
export function readParagraphs(txBody: XmlElement | undefined, chain: LevelStyles[]): TextPara[] {
  if (!txBody) return []
  const own = readLevels(kid(txBody, 'lstStyle'))
  const levels = [...chain, own]
  const out: TextPara[] = []
  for (const p of kids(txBody, 'p')) {
    const pPr = kid(p, 'pPr')
    const level = Math.min(8, Math.max(0, int(attr(pPr, 'lvl'))))
    let style: ParaStyle = { run: {} }
    for (const l of levels) style = merge(style, l[level])
    style = merge(style, readParaStyle(pPr))
    const runs: TextRun[] = []
    for (const c of p.children) {
      if (typeof c === 'string') continue
      const name = local(c.name)
      if (name === 'r' || name === 'fld') {
        const t = kid(c, 't')
        const value = t ? t.children.filter((x): x is string => typeof x === 'string').join('') : ''
        runs.push({ text: value, style: mergeRun(style.run, readRunStyle(kid(c, 'rPr'))) })
      } else if (name === 'br') runs.push({ text: '\n', style: mergeRun(style.run, readRunStyle(kid(c, 'rPr'))) })
    }
    // an empty paragraph still has the size of its end mark
    if (!runs.length) style = { ...style, run: mergeRun(style.run, readRunStyle(kid(p, 'endParaRPr'))) }
    out.push({ level, style, runs })
  }
  return out
}

const ROMAN = [['m', 1000], ['cm', 900], ['d', 500], ['cd', 400], ['c', 100], ['xc', 90], ['l', 50], ['xl', 40], ['x', 10], ['ix', 9], ['v', 5], ['iv', 4], ['i', 1]] as const
function roman(n: number): string {
  let out = ''
  let v = n
  for (const [s, k] of ROMAN) while (v >= k) { out += s; v -= k }
  return out
}
function alpha(n: number): string {
  let out = ''
  let v = n
  while (v > 0) { v -= 1; out = String.fromCharCode(97 + (v % 26)) + out; v = Math.floor(v / 26) }
  return out
}

/** An automatic number in one of DrawingML's schemes (`arabicPeriod`, `alphaLcParenR`, `romanUcPeriod` …). */
export function autoNumber(scheme: string, n: number): string {
  const m = /^(arabic|alphaLc|alphaUc|romanLc|romanUc|circleNum|ea1Chs|ea1Jpn|hindi|thai)(.*)$/.exec(scheme)
  const kind = m?.[1] ?? 'arabic'
  const tail = m?.[2] ?? 'Period'
  let num = kind === 'alphaLc' ? alpha(n) : kind === 'alphaUc' ? alpha(n).toUpperCase() : kind === 'romanLc' ? roman(n) : kind === 'romanUc' ? roman(n).toUpperCase() : String(n)
  if (kind === 'circleNum' && n >= 1 && n <= 20) num = String.fromCodePoint(0x2460 + n - 1)
  if (kind === 'ea1Chs' && n >= 1 && n <= 10) num = '一二三四五六七八九十'[n - 1] // i18n-exempt: the digits of Office's ea1Chs numbering scheme
  if (tail.includes('ParenBoth')) return `(${num})`
  if (tail.includes('ParenR')) return `${num})`
  if (tail.includes('Period') || tail.startsWith('Db')) return `${num}.`
  if (tail.includes('Minus')) return `- ${num} -`
  return num
}

export interface ShapeText {
  text: string
  /** The style most of the text has (the whole shape shows in it). */
  style: RunStyle
  align: string | undefined
  lineSpacing: ParaStyle['lineSpacing']
  /** Runs of more than one look. */
  mixed: boolean
  empty: boolean
}

const styleKey = (s: RunStyle) => [s.size, s.bold, s.italic, s.fill ? JSON.stringify(s.fill) : ''].join('|')

/** A text body as one string and one style (bullets as prefixes, two spaces per level). */
export function shapeText(paras: TextPara[]): ShapeText {
  const lines: string[] = []
  const counters: number[] = []
  let first: TextRun | null = null
  const weight = new Map<string, { run: TextRun; chars: number }>()
  let align: string | undefined
  let lineSpacing: ParaStyle['lineSpacing']
  let mixed = false
  let firstPara: TextPara | null = null
  for (const p of paras) {
    const value = p.runs.map((r) => (r.style.caps ? r.text.toUpperCase() : r.text)).join('')
    const has = value.trim() !== ''
    counters.length = p.level + 1
    let prefix = ''
    if (has && p.style.bullet?.kind === 'char') prefix = `${bulletChar(p.style.bullet.char, p.style.bulletFont)} `
    else if (has && p.style.bullet?.kind === 'auto') {
      counters[p.level] = (counters[p.level] ?? p.style.bullet.start - 1) + 1
      prefix = `${autoNumber(p.style.bullet.scheme, counters[p.level])} `
    }
    lines.push(has ? `${'  '.repeat(p.level)}${prefix}${value}` : value)
    for (const r of p.runs) {
      if (!r.text.trim()) continue
      if (!first) { first = r; firstPara = p } else if (styleKey(first.style) !== styleKey(r.style)) mixed = true
      const key = styleKey(r.style)
      const w = weight.get(key) ?? { run: r, chars: 0 }
      w.chars += r.text.trim().length
      weight.set(key, w)
    }
  }
  if (firstPara) { align = firstPara.style.align; lineSpacing = firstPara.style.lineSpacing }
  else if (paras[0]) { align = paras[0].style.align; lineSpacing = paras[0].style.lineSpacing }
  // trailing empty paragraphs show nothing
  while (lines.length && !lines[lines.length - 1].trim()) lines.pop()
  const text = lines.join('\n')
  // the look of most characters (a bold lead-in or a small footnote line does not decide the shape's style)
  const main = [...weight.values()].sort((a, b) => b.chars - a.chars)[0]?.run ?? first
  return { text, style: main?.style ?? paras[0]?.style.run ?? {}, align, lineSpacing, mixed, empty: text.trim() === '' }
}

/** The colour of a run style's fill (`a:solidFill`), or null (no fill element / no fill). */
export function runColor(style: RunStyle, ctx: ColorContext): Rgba | null {
  if (!style.fill || local(style.fill.name) === 'noFill') return null
  if (local(style.fill.name) === 'gradFill') return colorIn(kid(kid(style.fill, 'gsLst'), 'gs'), ctx)
  return colorIn(style.fill, ctx)
}

/** The face a run shows in: East Asian text in its `ea` face, else Latin (theme references resolved). */
export function runFace(style: RunStyle, text: string, theme: Theme): string {
  const cjk = /[　-鿿가-힯＀-￯]/.test(text)
  const face = (cjk && style.ea) || style.latin || (cjk ? '+mn-ea' : '+mn-lt')
  return themeFont(face, theme)
}
