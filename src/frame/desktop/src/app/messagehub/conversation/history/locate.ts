import { getMessageStableId, type MessageObject } from '../../protocol/msgobj'
import { messageObjId } from './relations'
import type { ConversationMessageReader } from './types'

const PAGE_SIZE = 128

/** Finds a timeline row by the ObjId of its anchor message (or by its stable row id). */
export async function findMessage(reader: ConversationMessageReader, id: string): Promise<{ message: MessageObject; index: number } | null> {
  for (let start = 0; start < reader.totalCount; start += PAGE_SIZE) {
    const messages = await reader.readRange(start, PAGE_SIZE)
    const offset = messages.findIndex((message, index) => messageObjId(message) === id || getMessageStableId(message, start + index) === id)
    if (offset >= 0) return { message: messages[offset], index: start + offset }
    if (messages.length === 0) break
  }
  return null
}
