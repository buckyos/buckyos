/* APP7: the browser conversation — the system conversation component on MessageHub's data, the way
 * MessageHub shows a conversation with one of the owner's own Agents: a question of the Agent (`ask_user`) is
 * answered in the composer, and while the Agent handles the message in front the Turn can be stopped from here. */

import { useEffect, useState } from 'react'
import { LoadingOrb } from '../../../components/LoadingOrb'
import { messageObjId } from '../../../components/conversation/history/relations'
import type { MessageObject } from '../../../components/conversation/model/msgobj'
import { SessionConversation } from '../../../components/conversation/SessionConversation'
import { useI18n } from '../../../i18n/provider'
import { desktopUIStore } from '../../../models/DesktopUIDataModel'
import type { ConversationContext } from '../extensions'
import { useAgentBrowser, useAgentBrowserStore } from '../store/context'
import { ensureBrowserSession } from './session'
import './conversation.css'

/** Q7: a message's details are MessageHub's; the conversation opens there on that message. */
function openInMessageHub({ agent, context }: ConversationContext, sessionId: string, message: MessageObject) {
  const messageId = messageObjId(message)
  if (!messageId) return
  desktopUIStore.openAppWindow('messagehub', { launch: { requestId: crypto.randomUUID(), payload: { kind: 'messagehub', entityId: agent.agentDid, sessionId, messageId, context } } })
}

function Status({ testId, title, detail, loading, action }: { testId: string; title: string; detail?: string; loading?: boolean; action?: { label: string; onClick: () => void } }) {
  return (
    <div className="ab-chat-status" role={loading ? 'status' : undefined} data-testid={testId}>
      {loading ? <LoadingOrb size={32} /> : null}
      <p className={loading ? undefined : 'ab-chat-status-title'}>{title}</p>
      {detail ? <p>{detail}</p> : null}
      {action ? <button type="button" className="ab-button" data-testid={`${testId}-retry`} onClick={action.onClick}>{action.label}</button> : null}
    </div>
  )
}

/**
 * The browser conversation (protocol §1), looked up (or started, and bound) when it is first shown. The one bound
 * before is not shown until then: it may be an older conversation that was not started here.
 */
export function ThingConversation() {
  const store = useAgentBrowserStore()
  const { t } = useI18n()
  const conversations = useAgentBrowser((snapshot) => snapshot.conversations)
  const agent = useAgentBrowser((snapshot) => snapshot.agent)
  const session = useAgentBrowser((snapshot) => snapshot.session)
  const [found, setFound] = useState<string | null>(null)
  const [attempt, setAttempt] = useState(0)
  const [failure, setFailure] = useState(-1)
  const shown = session && session.id === found ? session : null
  const finding = conversations === 'ready' && !shown
  useEffect(() => {
    const conversation = finding ? store.conversationContext() : null
    if (!conversation) return
    let alive = true
    ensureBrowserSession(conversation).then((next) => { if (alive) setFound(next.id) }, () => { if (alive) setFailure(attempt) })
    return () => { alive = false }
  }, [finding, store, attempt])

  const name = agent?.name ?? ''
  const conversation = store.conversationContext()
  if (conversations === 'unavailable') {
    return <Status testId="ab-conversation-unavailable" title={t('agentBrowser.chat.unavailable', 'Your conversations cannot be reached right now.')} detail={t('agentBrowser.chat.unavailableHint', 'You can keep browsing. Try again to talk to {{name}}.', { name })} action={{ label: t('common.retry'), onClick: () => store.retryConversations() }} />
  }
  if (shown && conversation) {
    return (
      <SessionConversation
        key={shown.id}
        sessionId={shown.id}
        testId="ab-conversation"
        placeholder={t('agentBrowser.chat.input', 'Message {{name}}…', { name })}
        emptyLabel={t('agentBrowser.chat.empty', 'Tell {{name}} what you want done. It works in this browser and asks here when it needs something from you.', { name })}
        readOnlyLabel={t('agentBrowser.chat.readOnly', 'Nothing can be sent in this conversation.')}
        onOpenMessage={(message) => openInMessageHub(conversation, shown.id, message)}
      />
    )
  }
  if (finding && failure === attempt) {
    return <Status testId="ab-conversation-failed" title={t('agentBrowser.chat.bindFailed', 'The conversation with {{name}} could not be opened.', { name })} action={{ label: t('common.retry'), onClick: () => setAttempt((value) => value + 1) }} />
  }
  return <Status testId="ab-conversation-loading" loading title={t('agentBrowser.chat.loading', 'Opening the conversation…')} />
}
