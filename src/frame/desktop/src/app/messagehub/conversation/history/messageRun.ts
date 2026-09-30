import { isActionMessage } from '../../sessionModel'
import type { ConversationListItem } from './types'

const MESSAGE_RUN_GAP_MS = 5 * 60_000

export function continuesMessageRun(previous: ConversationListItem | undefined, item: ConversationListItem | undefined): boolean {
  return previous?.kind === 'message'
    && item?.kind === 'message'
    && previous.data.from === item.data.from
    && !isActionMessage(previous.data)
    && !isActionMessage(item.data)
    && Math.abs(item.data.created_at_ms - previous.data.created_at_ms) < MESSAGE_RUN_GAP_MS
}
