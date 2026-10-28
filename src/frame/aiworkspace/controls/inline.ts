/* Inline Markdown of a node's title (脑图方案 §8: the outline keeps bold, links…): the title stores the Markdown
 * source; measuring and drawing use its runs. Only the inline subset an outline line carries: **bold** / __bold__,
 * *italic* / _italic_, `code`, ~~strike~~, [text](url), backslash escapes. Anything else is plain text. */

export interface RunStyle { bold?: boolean; italic?: boolean; code?: boolean; strike?: boolean; href?: string }
export interface Run extends RunStyle { text: string }

const ESCAPABLE = '\\`*_{}[]()#+-.!~|>'

/** The runs of `source` (adjacent runs of one style are merged). */
export function parseInline(source: string): Run[] {
  const out: Run[] = []
  const push = (text: string, style: RunStyle) => {
    if (!text) return
    const last = out[out.length - 1]
    if (last && same(last, style)) last.text += text
    else out.push({ text, ...style })
  }
  const walk = (s: string, style: RunStyle) => {
    let plain = ''
    const flush = () => { push(plain, style); plain = '' }
    for (let i = 0; i < s.length;) {
      const c = s[i]
      if (c === '\\' && i + 1 < s.length && ESCAPABLE.includes(s[i + 1])) { plain += s[i + 1]; i += 2; continue }
      if (c === '`') {
        const end = s.indexOf('`', i + 1)
        if (end > i + 1) { flush(); push(s.slice(i + 1, end), { ...style, code: true }); i = end + 1; continue }
      }
      if (c === '[') {
        const close = matching(s, i, '[', ']')
        if (close > 0 && s[close + 1] === '(') {
          const paren = matching(s, close + 1, '(', ')')
          if (paren > 0) {
            flush()
            walk(s.slice(i + 1, close), { ...style, href: s.slice(close + 2, paren).trim().split(/\s+/)[0] })
            i = paren + 1
            continue
          }
        }
      }
      const pair = s.startsWith('**', i) || s.startsWith('__', i) ? s.slice(i, i + 2) : s.startsWith('~~', i) ? '~~' : (c === '*' || c === '_') ? c : null
      if (pair) {
        const end = s.indexOf(pair, i + pair.length)
        // an opener needs text right after it, and `_` inside a word is not emphasis
        const opens = end > i + pair.length && s[i + pair.length] !== ' ' && s[end - 1] !== ' '
          && !(pair[0] === '_' && i > 0 && /\w/.test(s[i - 1]))
        if (opens) {
          flush()
          const inner = s.slice(i + pair.length, end)
          const next = pair === '~~' ? { ...style, strike: true } : pair.length === 2 ? { ...style, bold: true } : { ...style, italic: true }
          walk(inner, next)
          i = end + pair.length
          continue
        }
      }
      plain += c
      i += 1
    }
    flush()
  }
  walk(source, {})
  return out
}

function matching(s: string, open: number, a: string, b: string): number {
  let depth = 0
  for (let i = open; i < s.length; i++) {
    if (s[i] === '\\') { i += 1; continue }
    if (s[i] === a) depth += 1
    else if (s[i] === b) { depth -= 1; if (depth === 0) return i }
  }
  return -1
}

function same(a: RunStyle, b: RunStyle): boolean {
  return !!a.bold === !!b.bold && !!a.italic === !!b.italic && !!a.code === !!b.code && !!a.strike === !!b.strike && a.href === b.href
}

/** The text a reader sees (no markers). */
export function plainText(source: string): string {
  return parseInline(source).map((r) => r.text).join('')
}

/** Runs back to Markdown (what `parseInline` reads). */
export function formatInline(runs: Run[]): string {
  return runs.map((r) => {
    let t = escape(r.text, !!r.code)
    if (r.code) t = '`' + t + '`'
    if (r.strike) t = `~~${t}~~`
    if (r.italic) t = `*${t}*`
    if (r.bold) t = `**${t}**`
    if (r.href) t = `[${t}](${r.href})`
    return t
  }).join('')
}

function escape(text: string, code: boolean): string {
  return code ? text : text.replace(/[\\`*_[\]~]/g, (c) => '\\' + c)
}
