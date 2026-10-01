import type { MsgMentions } from '../../protocol/msgobj'

export interface MentionCandidate {
  did: string
  name: string
}

export const MENTION_ALL = '@all'

/** At most this many suggestions are offered while typing. */
const MENTION_SUGGESTION_LIMIT = 8
/** A longer word after `@` is not a name being looked up. */
const MENTION_QUERY_MAX = 32

/** Structured mentions for the text about to be sent: only names still present in it count (`mentions` is never `{}`). */
export function collectMentions(text: string, picked: readonly MentionCandidate[], mentionAll: boolean): MsgMentions | undefined {
  const dids = [...new Set(picked.filter(item => text.includes(`@${item.name}`)).map(item => item.did))]
  const all = mentionAll && text.includes(MENTION_ALL)
  if (!all && dids.length === 0) return undefined
  return { ...(dids.length ? { dids } : {}), ...(all ? { all: true } : {}) }
}

/** The `@word` the caret is in: `[start, end)` covers the `@` and the whole word, `query` is the part before the caret. */
export interface MentionQuery {
  start: number
  end: number
  query: string
}

/**
 * The mention being typed at `caret`: an `@` at the start of the text or after
 * whitespace or an opening bracket, followed by non-space characters. An `@`
 * inside a word (`mail@example.com`) starts no mention.
 */
export function mentionQueryAt(text: string, caret: number): MentionQuery | null {
  const before = text.slice(0, caret)
  const start = before.lastIndexOf('@')
  if (start < 0) return null
  const query = before.slice(start + 1)
  if (query.length > MENTION_QUERY_MAX || /\s/.test(query)) return null
  if (start > 0 && !/[\s([{"'“‘（【「《]/.test(before[start - 1])) return null
  const rest = text.slice(caret).match(/^\S*/)?.[0] ?? ''
  return { start, end: caret + rest.length, query }
}

/**
 * Candidates matching `query`, best first: names starting with it, then names
 * with a word starting with it, then names containing it. `@all` leads while
 * it matches.
 */
export function matchMentionCandidates(candidates: readonly MentionCandidate[], query: string, canMentionAll: boolean): Array<MentionCandidate | 'all'> {
  const needle = query.toLocaleLowerCase()
  const rank = (name: string) => {
    const haystack = name.toLocaleLowerCase()
    if (haystack.startsWith(needle)) return 0
    if (haystack.split(/[\s._-]+/).some(word => word.startsWith(needle))) return 1
    return haystack.includes(needle) ? 2 : -1
  }
  const matched = candidates
    .map((candidate, index) => ({ candidate, index, rank: rank(candidate.name) }))
    .filter(item => item.rank >= 0)
    .sort((a, b) => a.rank - b.rank || a.index - b.index)
    .map(item => item.candidate)
  const all = canMentionAll && 'all'.startsWith(needle) ? ['all' as const] : []
  return [...all, ...matched].slice(0, MENTION_SUGGESTION_LIMIT)
}

/** The text after replacing the typed `@word` with the picked mention, and the caret after it (and after one space). */
export function applyMention(text: string, query: MentionQuery, picked: MentionCandidate | 'all'): { text: string; caret: number } {
  const label = picked === 'all' ? MENTION_ALL : `@${picked.name}`
  const after = text.slice(query.end)
  const spaced = /^\s/.test(after) ? label : `${label} `
  return { text: `${text.slice(0, query.start)}${spaced}${after}`, caret: query.start + label.length + 1 }
}
