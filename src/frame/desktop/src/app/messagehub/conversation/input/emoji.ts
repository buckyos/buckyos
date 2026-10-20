export type EmojiGroupId = 'recent' | 'people' | 'nature' | 'food' | 'activity' | 'travel' | 'objects' | 'symbols' | 'flags'

export interface EmojiItem {
  emoji: string
  name: string
}

export interface EmojiGroup {
  id: EmojiGroupId
  items: EmojiItem[]
}

let groups: Promise<EmojiGroup[]> | null = null

/** The panel's emoji by category, loaded with the first panel that opens. */
export function loadEmojiGroups(): Promise<EmojiGroup[]> {
  groups ??= import('./emojiData').then(({ EMOJI_GROUP_DATA }) => EMOJI_GROUP_DATA.map(([id, lines]) => ({
    id: id as EmojiGroupId,
    items: lines.split('\n').map(line => {
      const space = line.indexOf(' ')
      return { emoji: line.slice(0, space), name: line.slice(space + 1) }
    }),
  })))
  groups.catch(() => { groups = null })
  return groups
}

const SEARCH_LIMIT = 160

/** Emoji whose English name has a word starting with each word of the query, best matches first. */
export function searchEmoji(all: readonly EmojiGroup[], query: string): EmojiItem[] {
  const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean)
  if (words.length === 0) return []
  const scored: Array<{ item: EmojiItem; score: number; order: number }> = []
  let order = 0
  for (const group of all) {
    for (const item of group.items) {
      order++
      const nameWords = item.name.split(/[\s:,-]+/)
      if (!words.every(word => nameWords.some(part => part.startsWith(word)))) continue
      scored.push({ item, score: item.name === words.join(' ') ? 0 : item.name.startsWith(words[0]) ? 1 : 2, order })
    }
  }
  return scored.sort((a, b) => a.score - b.score || a.order - b.order).slice(0, SEARCH_LIMIT).map(entry => entry.item)
}

const RECENT_KEY = 'buckyos.messagehub.emoji.v1'
const RECENT_LIMIT = 24
export const DEFAULT_RECENT_EMOJI = ['👍', '❤️', '😂', '😮', '😢', '🙏', '🔥', '🎉', '👏', '😁', '🤔', '😍', '🥰', '😭', '😅', '👌']

export function recentEmoji(): string[] {
  try {
    const value: unknown = JSON.parse(window.localStorage.getItem(RECENT_KEY) ?? 'null')
    const recent = value && typeof value === 'object' ? (value as { recent?: unknown }).recent : undefined
    if (Array.isArray(recent)) return recent.filter((item): item is string => typeof item === 'string' && item.length > 0 && item.length <= 32).slice(0, RECENT_LIMIT)
  } catch {
    // Storage unavailable: the defaults stand in for the history.
  }
  return [...DEFAULT_RECENT_EMOJI]
}

export function rememberEmoji(emoji: string): string[] {
  const next = [emoji, ...recentEmoji().filter(item => item !== emoji)].slice(0, RECENT_LIMIT)
  try {
    window.localStorage.setItem(RECENT_KEY, JSON.stringify({ recent: next }))
  } catch {
    // Not remembered on this device; the panel still shows it until reload.
  }
  return next
}

const KEYCAP = /^[#*0-9]️?⃣$/u
const FLAG = /^\p{Regional_Indicator}{2}$/u
const PICTOGRAPHIC = /\p{Extended_Pictographic}/u
const PRESENTED = /\p{Emoji_Presentation}|️/u
const WORD = /[\p{L}\p{N}]/u

function isEmojiGrapheme(segment: string): boolean {
  if (KEYCAP.test(segment) || FLAG.test(segment)) return true
  return PICTOGRAPHIC.test(segment) && PRESENTED.test(segment) && !WORD.test(segment.replace(/[⃣️]/gu, ''))
}

let segmenter: Intl.Segmenter | null | undefined

/**
 * How many emoji a text consists of (whitespace between them ignored), when it
 * is nothing but 1–`max` emoji; 0 otherwise. Emoji written in text style
 * (`©` without U+FE0F) do not count.
 */
export function emojiOnlyCount(text: string, max = 3): number {
  const trimmed = text.trim()
  if (!trimmed || trimmed.length > 16 * max) return 0
  if (segmenter === undefined) segmenter = typeof Intl.Segmenter === 'function' ? new Intl.Segmenter(undefined, { granularity: 'grapheme' }) : null
  if (!segmenter) return 0
  let count = 0
  for (const { segment } of segmenter.segment(trimmed)) {
    if (!segment.trim()) continue
    if (!isEmojiGrapheme(segment) || ++count > max) return 0
  }
  return count
}
