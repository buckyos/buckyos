/* `context.quote` matching — the rules of the backend (core/src/anchor.rs): inside the recorded
 * target the best match wins (most agreement with the recorded prefix/suffix, then the earliest);
 * anywhere else only an unmistakable one counts. */

import type { AnnotationQuote } from '../api/types'

/** A quote searched outside its recorded target must be at least this many characters long. */
export const MIN_RELOCATE_CHARS = 4
/** Characters of context recorded on each side of a quote. */
export const QUOTE_SIDE_CHARS = 32
export const MAX_QUOTE_CHARS = 2000
export const BLOCK_SEPARATOR = '\n'
/** An inline atom (hard break, object link) in anchor text. */
export const ATOM_CHAR = '￼'

const chars = (text: string) => Array.from(text)

interface Occurrence { start: number; end: number; agree: number }

function occurrences(hay: string, quote: AnnotationQuote): Occurrence[] {
  const prefix = chars(quote.prefix ?? '')
  const suffix = chars(quote.suffix ?? '')
  const out: Occurrence[] = []
  for (let start = hay.indexOf(quote.exact); start >= 0; start = hay.indexOf(quote.exact, start + 1)) {
    const end = start + quote.exact.length
    const before = chars(hay.slice(Math.max(0, start - 4 * prefix.length), start))
    const after = chars(hay.slice(end, end + 4 * suffix.length))
    let agree = 0
    for (let i = 1; i <= prefix.length && i <= before.length && prefix[prefix.length - i] === before[before.length - i]; i++) agree++
    for (let i = 0; i < suffix.length && i < after.length && suffix[i] === after[i]; i++) agree++
    out.push({ start, end, agree })
  }
  return out
}

/** UTF-16 offsets of the best occurrence in `hay`. */
export function findBest(hay: string, quote: AnnotationQuote): [number, number] | null {
  let best: Occurrence | null = null
  for (const o of occurrences(hay, quote)) if (!best || o.agree > best.agree) best = o
  return best ? [best.start, best.end] : null
}

/** The only occurrence, or the only one whose recorded context agrees completely; never for short quotes. */
export function findUnique(hay: string, quote: AnnotationQuote): [number, number] | null {
  if (chars(quote.exact).length < MIN_RELOCATE_CHARS) return null
  const all = occurrences(hay, quote)
  if (all.length === 1) return [all[0].start, all[0].end]
  const full = chars(quote.prefix ?? '').length + chars(quote.suffix ?? '').length
  const agreeing = all.filter((o) => full > 0 && o.agree === full)
  return agreeing.length === 1 ? [agreeing[0].start, agreeing[0].end] : null
}

/** The last / first `n` characters (never splitting a surrogate pair). */
export const tail = (text: string, n: number) => (n > 0 ? chars(text).slice(-n).join('') : '')
export const head = (text: string, n: number) => chars(text).slice(0, n).join('')
