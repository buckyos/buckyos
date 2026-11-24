/** Marking a conversation read: every inbound record of it that is still unread. */
import type { DID } from 'buckyos'
import type { ListSessionRequest, MailboxRecord, RecipientState, SessionMessageItem, SessionMessagePage } from './wire.ts'

const PAGE_SIZE = 64
const PARALLEL_UPDATES = 8

export interface UnreadRecords {
  owner: DID
  sessionId: string
  /** The session's latest record when the caller read its list: what arrives later stays unread. */
  boundary?: Pick<MailboxRecord, 'sort_key' | 'record_id'>
}

/** Adds the unread inbound records of a session to `into`, page by page; `check` may throw to stop. */
export async function collectUnreadRecords(
  list: (request: ListSessionRequest) => Promise<SessionMessagePage>,
  { owner, sessionId, boundary }: UnreadRecords,
  into: SessionMessageItem[],
  check: () => void = () => undefined,
): Promise<void> {
  let cursor: { sortKey: number; recordId: string } | undefined
  do {
    const page = await list({ owner, session_id: sessionId, limit: PAGE_SIZE, descending: true, with_object: false, cursor_sort_key: cursor?.sortKey, cursor_record_id: cursor?.recordId })
    check()
    into.push(...(page.items ?? []).filter(item => item.direction === 'in' && item.recipient_state === 'UNREAD' && (!boundary || item.sort_key < boundary.sort_key || (item.sort_key === boundary.sort_key && item.record_id <= boundary.record_id))))
    cursor = page.next_cursor_sort_key !== undefined && page.next_cursor_record_id !== undefined ? { sortKey: page.next_cursor_sort_key, recordId: page.next_cursor_record_id } : undefined
  } while (cursor)
}

/** Marks records read a few at a time; `onBatch` gets each batch's outcome. */
export async function markRecordsRead(
  update: (recordId: string, state: RecipientState) => Promise<MailboxRecord>,
  records: readonly SessionMessageItem[],
  onBatch: (results: PromiseSettledResult<MailboxRecord>[]) => void,
): Promise<void> {
  for (let offset = 0; offset < records.length; offset += PARALLEL_UPDATES) {
    onBatch(await Promise.allSettled(records.slice(offset, offset + PARALLEL_UPDATES).map(item => update(item.record_id, 'READ'))))
  }
}
