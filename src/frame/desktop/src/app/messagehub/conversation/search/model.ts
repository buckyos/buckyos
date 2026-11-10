import { parseFriendNotice } from '../../friendNotice'
import { parseGroupNotice } from '../../groupModel'
import { getMessageStableId, isStatusMessageObject, messageNotify, type MessageObject } from '../../protocol/msgobj'
import { isActionMessage } from '../../sessionModel'
import { effectiveContent, messageRelations } from '../history/relations'

export interface SearchHit {
  /** Stable id of the row, as `ConversationHistoryPane.scrollToMessage` takes it. */
  id: string
  createdAt: number
}

const fold = (value: string) => value.normalize('NFKC').toLocaleLowerCase()

/** The words of a query; a row matches when it holds every one of them. */
export function searchTerms(query: string): string[] {
  return [...new Set(fold(query).split(/\s+/).filter(Boolean))]
}

/** What a row is found by: the text it shows (after edits), the names of its attachments, and the title of a notification card. */
export function searchableText(message: MessageObject): string {
  const content = effectiveContent(message)
  return fold([messageNotify(message)?.prompt.title ?? '', content.content ?? '', ...(content.refs ?? []).map(ref => ref.label ?? '')].join('\n'))
}

/** A card whose text is made from structured data (a friend request, a group notice): its plain-text body is not what the row shows. */
const isNoticeCard = (message: MessageObject) => Boolean(parseFriendNotice(message) || parseGroupNotice(message))

/** Rows that hold every word of the query, oldest first. Status rows, the action log, notice cards and redacted or unreadable messages are not searched. */
export function searchMessages(rows: readonly MessageObject[], query: string): SearchHit[] {
  const terms = searchTerms(query)
  if (terms.length === 0) return []
  const hits: SearchHit[] = []
  rows.forEach((message, index) => {
    if (isStatusMessageObject(message) || isActionMessage(message) || isNoticeCard(message) || message.ui_unavailable === true || messageRelations(message)?.redacted) return
    const text = searchableText(message)
    if (terms.every(term => text.includes(term))) hits.push({ id: getMessageStableId(message, index), createdAt: message.created_at_ms })
  })
  return hits
}
