import type {
  ConversationStatusType,
  MessageObject,
} from '../../protocol/msgobj'

export interface ConversationMessageReader {
  readonly readerKey: string
  readonly totalCount: number
  /**
   * The loaded messages behind the rows, unfolded and oldest first, when the
   * reader holds them in memory: state derived from the whole timeline (the
   * conversation input state) reads them here.
   */
  readonly timeline?: readonly MessageObject[]
  readRange(startIndex: number, count: number): Promise<readonly MessageObject[]>
}

export interface AppendableConversationMessageReader
extends ConversationMessageReader {
  append(message: MessageObject): AppendableConversationMessageReader
}

export interface ConversationStatusDescriptor {
  id: string
  label: string
  status: ConversationStatusType
  position?: 'head' | 'tail'
  createdAtMs?: number
}

export type ConversationListIndexEntry =
  | {
      kind: 'message'
      key: string
      messageIndex: number
    }
  | {
      kind: 'timestamp'
      key: string
      dateMs: number
      anchorMessageIndex: number
    }
  | {
      kind: 'status'
      key: string
      status: ConversationStatusType
      label: string
      anchorMessageIndex?: number
      createdAtMs?: number
    }

export type ConversationListItem =
  | {
      kind: 'message'
      key: string
      index: number
      messageIndex: number
      data: MessageObject
    }
  | {
      kind: 'timestamp'
      key: string
      index: number
      date: Date
    }
  | {
      kind: 'status'
      key: string
      index: number
      status: ConversationStatusType
      label: string
      createdAtMs?: number
    }

export interface ConversationProjection {
  readonly showActions: boolean
  readonly readerKey: string
  readonly messageCount: number
  readonly tailStatusCount: number
  readonly statusItemsSignature: string
  readonly lastMessage?: MessageObject
  readonly totalCount: number
  readonly entries: readonly ConversationListIndexEntry[]
}

export interface ConversationMaterializedWindow {
  startIndex: number
  endIndex: number
  items: readonly ConversationListItem[]
}
