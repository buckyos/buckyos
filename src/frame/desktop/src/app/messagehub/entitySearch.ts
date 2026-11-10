/* ── Fuzzy entity search of the new-chat dialog: names by letters in order, DIDs by substring ── */

const fold = (value: string) => value.normalize('NFKC').toLocaleLowerCase()

/**
 * How well `text` answers one word: 0 is no match. The whole text ranks above a
 * prefix, a prefix above a match inside (earlier is better, a word start counts),
 * and those above letters that only appear in order within a short stretch.
 */
export function fuzzyScore(word: string, text: string): number {
  const query = fold(word).trim(), target = fold(text)
  if (!query) return 1
  if (!target) return 0
  if (target === query) return 1000
  if (target.startsWith(query)) return 800 - Math.min(200, target.length - query.length)
  const at = target.indexOf(query)
  if (at >= 0) return 600 - Math.min(200, at) + (/[\s._\-:/@]/.test(target[at - 1]) ? 100 : 0)
  const letters = [...query]
  if (letters.length < 2) return 0
  let cursor = 0, first = -1, last = -1
  for (const letter of letters) {
    const found = target.indexOf(letter, cursor)
    if (found < 0) return 0
    if (first < 0) first = found
    last = found
    cursor = found + letter.length
  }
  const spread = last - first + 1
  // Letters scattered over a long name are a coincidence, not a match.
  if (spread > letters.length * 2) return 0
  return Math.max(1, 300 - (spread - letters.length) * 20 - Math.min(100, first))
}

/** An entity matches when every word of the query is in its name (fuzzily) or in its DID; the score is the mean of the words'. */
export function entityMatchScore(query: string, entity: { name: string; id: string }): number {
  const words = fold(query).split(/\s+/).filter(Boolean)
  if (words.length === 0) return 1
  const id = fold(entity.id)
  let total = 0
  for (const word of words) {
    const inId = id.includes(word) ? (id === word ? 500 : 250) : 0
    const score = Math.max(fuzzyScore(word, entity.name), inId)
    if (score <= 0) return 0
    total += score
  }
  return total / words.length
}

/** The entities answering the query, best first; ties keep the given order (the list's: pinned, then recent). */
export function rankEntities<T extends { name: string; id: string }>(entities: readonly T[], query: string): T[] {
  return entities
    .map((entity, index) => ({ entity, index, score: entityMatchScore(query, entity) }))
    .filter(item => item.score > 0)
    .sort((a, b) => b.score - a.score || a.index - b.index)
    .map(item => item.entity)
}
