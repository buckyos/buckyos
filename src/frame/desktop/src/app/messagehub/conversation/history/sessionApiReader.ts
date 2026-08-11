import {
  listSessionMessages,
  type SessionDeliveryOverall,
  type SessionMessageItem,
} from '../../datamodel/sessionApi'
import type {
  DID,
  MessageDeliveryStatus,
  MessageObject,
} from '../../protocol/msgobj'
import type { AppendableConversationMessageReader } from './types'

const SESSION_MESSAGE_PAGE_SIZE = 200

/**
 * Conversation reader backed by the msg-center `msg.list_session` projection.
 *
 * The factory pulls the full session timeline once (ascending, paged) and the
 * reader keeps it in memory; `append` returns a new reader with the same
 * readerKey so the history pane extends its projection incrementally.
 */
export class SessionApiConversationMessageReader
implements AppendableConversationMessageReader {
  readonly readerKey: string
  readonly totalCount: number
  private readonly sessionId: string
  private readonly messages: readonly MessageObject[]

  constructor(sessionId: string, messages: readonly MessageObject[]) {
    this.sessionId = sessionId
    this.messages = messages
    this.totalCount = messages.length
    this.readerKey = `msg-center:${sessionId}`
  }

  append(message: MessageObject) {
    return new SessionApiConversationMessageReader(
      this.sessionId,
      [...this.messages, message],
    )
  }

  async readRange(startIndex: number, count: number) {
    if (count <= 0 || startIndex >= this.totalCount) {
      return []
    }

    const safeStart = Math.max(0, startIndex)
    return this.messages.slice(safeStart, safeStart + count)
  }
}

function mapDeliveryOverall(
  overall: SessionDeliveryOverall | undefined,
): MessageDeliveryStatus | undefined {
  switch (overall) {
    case 'sending':
      return 'sending'
    case 'delivered':
      return 'delivered'
    case 'partial_failed':
    case 'failed':
      return 'failed'
    default:
      return undefined
  }
}

/**
 * Merge one `SessionMessageItem` into the UI `MessageObject` shape: the
 * protocol object stays as-is and the UI meta hints ride along as flattened
 * `ui_*` keys. Items without an attached message object are skipped.
 */
export function sessionItemToMessageObject(
  item: SessionMessageItem,
  sessionId: string,
): MessageObject | null {
  if (!item.msg) {
    console.warn(
      `Skipping session message ${item.record_id} of ${sessionId}: no message object attached.`,
    )
    return null
  }

  const message: MessageObject = {
    ...item.msg,
    ui_message_id: item.record_id,
    ui_session_id: sessionId,
  }

  if (item.direction === 'out') {
    const deliveryStatus = mapDeliveryOverall(item.delivery?.overall)
    if (deliveryStatus) {
      message.ui_delivery_status = deliveryStatus
    }
  }

  return message
}

/**
 * Build a reader for one session by paging through `msg.list_session`
 * (ascending, with message objects) until the cursor is exhausted.
 */
export async function createSessionApiReader(
  owner: DID,
  sessionId: string,
): Promise<AppendableConversationMessageReader> {
  const messages: MessageObject[] = []
  let cursorSortKey: number | undefined
  let cursorRecordId: string | undefined

  for (;;) {
    const page = await listSessionMessages({
      owner,
      session_id: sessionId,
      limit: SESSION_MESSAGE_PAGE_SIZE,
      cursor_sort_key: cursorSortKey,
      cursor_record_id: cursorRecordId,
      descending: false,
      with_object: true,
    })

    for (const item of page.items ?? []) {
      const message = sessionItemToMessageObject(item, sessionId)
      if (message) {
        messages.push(message)
      }
    }

    if (
      page.next_cursor_sort_key === undefined
      || page.next_cursor_record_id === undefined
    ) {
      break
    }

    cursorSortKey = page.next_cursor_sort_key
    cursorRecordId = page.next_cursor_record_id
  }

  return new SessionApiConversationMessageReader(sessionId, messages)
}
