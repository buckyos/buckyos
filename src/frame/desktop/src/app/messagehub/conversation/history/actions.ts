import { createContext } from 'react'
import type { DID, MessageObject } from '../../protocol/msgobj'
import type { ReadReceipt } from '../../types'

/** What a group notification card needs to show about its group. */
export interface GroupNoticeView {
  groupName: string
  /** Only for invitations. */
  state?: 'pending' | 'joined' | 'approval' | 'expired'
}

/** Which relation actions the viewer may take on one message (host rules: author, time window, capability). */
export interface MessageRelationCapabilities {
  reply: boolean
  react: boolean
  edit: boolean
  redact: boolean
}

/** Per-message actions offered by the conversation (absent when read only). */
export interface ConversationMessageActions {
  resend?: (message: MessageObject) => Promise<void>
  /** Display name of a DID referenced by Action Log events and notices. */
  displayName?: (did: DID) => string
  groupNotice?: (message: MessageObject) => GroupNoticeView | null
  joinGroup?: (message: MessageObject) => Promise<void>
  approveMember?: (message: MessageObject) => Promise<void>
  rejectMember?: (message: MessageObject) => Promise<void>
  acceptOwnerTransfer?: (message: MessageObject) => Promise<void>
  acceptSessionInvitation?: (message: MessageObject) => Promise<void>
  openEntity?: (did: DID) => void
  /** MsgObject v2 relations (`relates_to`): edit / recall / delete / react / reply. */
  relations?: {
    capabilities: (message: MessageObject) => MessageRelationCapabilities
    reply: (message: MessageObject) => void
    edit: (message: MessageObject) => void
    redact: (message: MessageObject) => Promise<void>
    react: (message: MessageObject, key: string) => Promise<void>
  }
  /** Read receipt of an own group message, when the session shows receipts. */
  readReceipt?: (message: MessageObject) => ReadReceipt | null
}

export const ConversationMessageActionsContext = createContext<ConversationMessageActions>({})
