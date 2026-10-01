import type { MsgMentions } from '../../protocol/msgobj'

export interface MentionCandidate {
  did: string
  name: string
}

export const MENTION_ALL = '@all'

/** Structured mentions for the text about to be sent: only names still present in it count (`mentions` is never `{}`). */
export function collectMentions(text: string, picked: readonly MentionCandidate[], mentionAll: boolean): MsgMentions | undefined {
  const dids = [...new Set(picked.filter(item => text.includes(`@${item.name}`)).map(item => item.did))]
  const all = mentionAll && text.includes(MENTION_ALL)
  if (!all && dids.length === 0) return undefined
  return { ...(dids.length ? { dids } : {}), ...(all ? { all: true } : {}) }
}
