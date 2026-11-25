import { ChevronDown } from 'lucide-react'
import { useCallback } from 'react'
import { useMobileBackHandler } from '../../../desktop/windows/MobileNavContext'
import { useI18n } from '../../../i18n/provider'
import { useAgentBrowser, useAgentBrowserStore } from '../store/context'
import { ThingConversation } from './ThingConversation'

/**
 * The conversation pulled up over the stage (home and original page modes), closed by default: from the bottom on
 * a phone, at the side of a wide window. The page stays usable beside it; the phone's Back puts it away.
 */
export function ConversationSheet() {
  const store = useAgentBrowserStore()
  const { t } = useI18n()
  const open = useAgentBrowser((snapshot) => snapshot.chatOpen)
  const name = useAgentBrowser((snapshot) => snapshot.agent?.name ?? '')
  const close = useCallback(() => store.setChatOpen(false), [store])
  useMobileBackHandler(open ? close : null)
  if (!open) return null
  const title = t('agentBrowser.chat.with', 'Conversation with {{name}}', { name })
  return (
    <section className="ab-chat-sheet" aria-label={title} data-testid="ab-chat-sheet">
      <header className="ab-chat-sheet-head">
        <h2>{title}</h2>
        <button type="button" className="ab-icon-button" aria-label={t('agentBrowser.chat.close', 'Close the conversation')} data-testid="ab-chat-sheet-close" onClick={close}><ChevronDown size={18} aria-hidden /></button>
      </header>
      <div className="ab-chat-sheet-body"><ThingConversation /></div>
    </section>
  )
}
