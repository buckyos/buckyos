import { createContext } from 'react'
import type { DID, MessageObject } from '../../protocol/msgobj'

/** What a group notification card needs to show about its group. */
export interface GroupNoticeView {
  groupName: string
  /** Only for invitations. */
  state?: 'pending' | 'joined' | 'expired'
}

/** Per-message actions offered by the conversation (absent when read only). */
export interface ConversationMessageActions {
  resend?: (message: MessageObject) => Promise<void>
  /** Display name of a DID referenced by Action Log events and notices. */
  displayName?: (did: DID) => string
  groupNotice?: (message: MessageObject) => GroupNoticeView | null
  joinGroup?: (message: MessageObject) => Promise<void>
  openEntity?: (did: DID) => void
}

export const ConversationMessageActionsContext = createContext<ConversationMessageActions>({})
