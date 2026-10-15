/* draw.io labels with `html=1` (draw.io 导入 TODO §3): HTML → plain text with line breaks, and the formatting that
 * covers the whole label as the shape's text keys. Formatting of a part of the label, lists and tables are not
 * kept (plain text, reported). No DOM: tags are scanned, entities decoded. */

import { decodeEntities } from './xml.ts'
import { normalizeColor } from './style.ts'

export interface LabelText {
  text: string
  bold: boolean
  italic: boolean
  underline: boolean
  color: string | null
  /** Some formatting covered only part of the label (dropped). */
  partial: boolean
  /** Lists, tables, images or other structure flattened to text. */
  complex: boolean
}

const BLOCK = new Set(['div', 'p', 'li', 'tr', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'ul', 'ol', 'table', 'blockquote', 'pre', 'hr'])
const COMPLEX = new Set(['ul', 'ol', 'table', 'img', 'pre', 'hr', 'blockquote'])
const VOID = new Set(['br', 'img', 'hr', 'input', 'meta', 'wbr'])

interface Run { text: string; bold: boolean; italic: boolean; underline: boolean; color: string | null }
interface Frame { tag: string; bold?: boolean; italic?: boolean; underline?: boolean; color?: string | null }

function styleOf(tag: string, attrs: string): Omit<Frame, 'tag'> {
  const out: Omit<Frame, 'tag'> = {}
  if (tag === 'b' || tag === 'strong' || /^h[1-6]$/.test(tag)) out.bold = true
  if (tag === 'i' || tag === 'em') out.italic = true
  if (tag === 'u') out.underline = true
  const color = /\scolor\s*=\s*["']?([^"'\s>]+)/i.exec(attrs)
  if (tag === 'font' && color) out.color = normalizeColor(decodeEntities(color[1]))
  const style = /\sstyle\s*=\s*(?:"([^"]*)"|'([^']*)')/i.exec(attrs)
  if (style) {
    const css = decodeEntities(style[1] ?? style[2] ?? '')
    for (const decl of css.split(';')) {
      const [k, ...rest] = decl.split(':')
      const key = k?.trim().toLowerCase()
      const value = rest.join(':').trim().toLowerCase()
      if (!key || !value) continue
      if (key === 'font-weight') out.bold = value === 'bold' || Number(value) >= 600
      else if (key === 'font-style') out.italic = value === 'italic' || value === 'oblique'
      else if (key === 'text-decoration' || key === 'text-decoration-line') out.underline = value.includes('underline')
      else if (key === 'color') out.color = normalizeColor(value)
    }
  }
  return out
}

/** Convert a draw.io HTML label. */
export function htmlLabel(html: string): LabelText {
  const runs: Run[] = []
  const stack: Frame[] = []
  let complex = false
  let text = ''
  const current = (): Run => {
    let bold = false, italic = false, underline = false
    let color: string | null = null
    for (const f of stack) {
      if (f.bold !== undefined) bold = f.bold
      if (f.italic !== undefined) italic = f.italic
      if (f.underline !== undefined) underline = f.underline
      if (f.color !== undefined) color = f.color
    }
    return { text: '', bold, italic, underline, color }
  }
  const newline = () => { if (text && !text.endsWith('\n')) text += '\n' }
  const push = (chunk: string) => {
    const t = decodeEntities(chunk).replace(/[ \t\r\n]+/g, ' ').replace(/ /g, ' ')
    if (!t) return
    const leading = text === '' || text.endsWith('\n')
    const piece = leading ? t.replace(/^ +/, '') : t
    if (!piece) return
    text += piece
    const run = current()
    run.text = piece
    runs.push(run)
  }
  const re = /<\/?([A-Za-z][A-Za-z0-9]*)([^>]*)>|<!--[\s\S]*?-->/g
  let at = 0
  for (let m = re.exec(html); m; m = re.exec(html)) {
    if (m.index > at) push(html.slice(at, m.index))
    at = re.lastIndex
    if (!m[1]) continue
    const tag = m[1].toLowerCase()
    const closing = m[0][1] === '/'
    if (COMPLEX.has(tag)) complex = true
    if (tag === 'br') { text += '\n'; continue }
    if (closing) {
      for (let i = stack.length - 1; i >= 0; i--) if (stack[i].tag === tag) { stack.splice(i); break }
      if (BLOCK.has(tag)) newline()
      continue
    }
    if (BLOCK.has(tag)) newline()
    if (tag === 'li') text += '• '
    if (!VOID.has(tag) && !m[2].trim().endsWith('/')) stack.push({ tag, ...styleOf(tag, m[2]) })
  }
  if (at < html.length) push(html.slice(at))
  const body = text.replace(/[ \t]+\n/g, '\n').replace(/\n+$/, '').replace(/^\n+/, '')
  const visible = runs.filter((r) => r.text.trim())
  const all = <K extends keyof Run>(k: K) => visible.length > 0 && visible.every((r) => r[k] === visible[0][k]) ? visible[0][k] : null
  const bold = all('bold') === true
  const italic = all('italic') === true
  const underline = all('underline') === true
  const color = all('color') as string | null
  const partial = visible.some((r) => r.bold !== bold || r.italic !== italic || r.underline !== underline || r.color !== color)
  return { text: body, bold, italic, underline, color: color === 'none' || color === 'default' ? null : color, partial, complex }
}

/** A plain (non-HTML) label: draw.io keeps its line breaks. */
export function plainLabel(value: string): LabelText {
  return { text: value.replace(/\r\n?/g, '\n'), bold: false, italic: false, underline: false, color: null, partial: false, complex: false }
}

/** Replace `%name%` placeholders with the cell's data (draw.io `placeholders=1`); `%%` is a literal percent. */
export function fillPlaceholders(label: string, data: Record<string, string>): string {
  return label.replace(/%([A-Za-z0-9_.-]*)%/g, (all, name: string) => (name === '' ? '%' : name in data ? data[name] : all))
}
