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
  /** The viewer may ask every participant to delete the message (`redact`). */
  redact: boolean
  /** The redact is a recall of an own group message inside the session's recall window. */
  recall: boolean
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
    /** Cancels the viewer's own reaction with `key` (a `redact` of the reaction message). */
    unreact: (message: MessageObject, key: string) => Promise<void>
  }
  /** Opens the target picker that posts a copy of the message to another session. */
  forward?: (message: MessageObject) => void
  /** Removes the message from the viewer's own view only (other participants keep it). */
  remove?: (message: MessageObject) => Promise<void>
  /** Pins the message to the session panel, or unpins it when it is the pinned one. */
  pin?: { isPinned: (message: MessageObject) => boolean; toggle: (message: MessageObject) => Promise<void> }
  /** Read receipt of an own group message, when the session shows receipts. */
  readReceipt?: (message: MessageObject) => ReadReceipt | null
}

export const ConversationMessageActionsContext = createContext<ConversationMessageActions>({})
