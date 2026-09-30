import { createContext } from 'react'
import type { MessageObject } from '../../protocol/msgobj'

/** Per-message actions offered by the conversation (absent when read only). */
export interface ConversationMessageActions {
  resend?: (message: MessageObject) => Promise<void>
}

export const ConversationMessageActionsContext = createContext<ConversationMessageActions>({})
