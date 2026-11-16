import { MessageSquare } from 'lucide-react'
import { useState } from 'react'
import { useI18n } from '../../../i18n/provider'
import type { BrowserGroup } from '../api/types'
import { HandBackSheet, HandToAgentSheet } from '../conversation/HandoverSheets'
import { useAgentBrowser, useAgentBrowserStore } from '../store/context'

type Translate = ReturnType<typeof useI18n>['t']

/** Who operates, in one line. */
function operatorText(t: Translate, group: BrowserGroup, name: string): string {
  if (group.controller === 'agent') return t('agentBrowser.operator.agent', '{{name}} is operating', { name })
  if (group.user_hold && group.handover) return t('agentBrowser.operator.asked', '{{name}} asks you to take over', { name })
  if (group.user_hold) return t('agentBrowser.operator.held', 'You have taken over')
  return t('agentBrowser.operator.user', 'You are operating')
}

/** What the Agent is doing or waiting for, with the note it left. */
function agentText(t: Translate, group: BrowserGroup, name: string): string {
  if (group.user_hold && group.handover) {
    const reason = group.handover.reason
    const why = reason === 'other' ? '' : t(`agentBrowser.handover.${reason}`, reason)
    return [why, group.handover.note ?? group.note ?? ''].filter(Boolean).join(' · ') || t('agentBrowser.handover.other', 'It needs you to do this step')
  }
  if (group.controller === 'agent') return t('agentBrowser.operator.agentHint', 'The page is view only until you take over.')
  // A question of the Agent is answered in the conversation, not on the page (also while the user holds the page).
  const asking = group.agent_state === 'waiting_answer'
  if (group.user_hold && !asking) return t('agentBrowser.operator.heldHint', '{{name}} has stopped operating until you hand back.', { name })
  const state = t(`agentBrowser.state.${group.agent_state}`, group.agent_state, { name })
  const more = asking ? t('agentBrowser.operator.answerHint', 'answer in the conversation') : group.note
  return more ? `${state} · ${more}` : state
}

/** The way into the browser conversation while the page is on the stage: it comes up over the page. */
function ChatToggle() {
  const store = useAgentBrowserStore()
  const { t } = useI18n()
  const open = useAgentBrowser((snapshot) => snapshot.chatOpen)
  const unread = useAgentBrowser((snapshot) => (snapshot.session?.unreadCount ?? 0) > 0)
  return (
    <button type="button" className="ab-icon-button ab-chat-toggle" aria-expanded={open} aria-label={open ? t('agentBrowser.chat.close', 'Close the conversation') : t('agentBrowser.chat.open', 'Open the conversation')} data-testid="ab-chat-toggle" onClick={() => store.setChatOpen(!open)}>
      <MessageSquare size={18} aria-hidden />
      {unread && !open ? <span className="ab-chat-unread" role="img" aria-label={t('agentBrowser.chat.unread', 'New messages')} data-testid="ab-chat-unread" /> : null}
    </button>
  )
}

/**
 * The strip that always shows who operates and the Agent's state (D8), with the one handover action that applies.
 * Handing the page to the Agent or back wakes it with a message in the browser conversation (DG2, DG3), which the
 * owner sees first; without the conversations the page changes hands at once.
 */
export function HandoverBar() {
  const store = useAgentBrowserStore()
  const { t } = useI18n()
  const group = useAgentBrowser((snapshot) => snapshot.group)
  const name = useAgentBrowser((snapshot) => snapshot.agent?.name ?? '')
  const pending = useAgentBrowser((snapshot) => snapshot.pending)
  // Where the conversation is: `stage` in conversation mode, else pulled up over the page (`open`) or put away.
  const chat = useAgentBrowser((snapshot) => (snapshot.mode === 'chat' ? 'stage' : snapshot.chatOpen ? 'open' : 'closed'))
  const talks = useAgentBrowser((snapshot) => snapshot.conversations === 'ready')
  const [asking, setAsking] = useState<'hand_back' | 'hand_to_agent' | null>(null)
  if (!group) {
    return <div className="ab-handover" data-testid="ab-status" data-controller="unknown"><p className="ab-handover-sub">{t('agentBrowser.operator.unknown', 'Waiting for the browser’s state…')}</p></div>
  }
  const busy = pending !== null
  const tone = group.controller === 'agent' ? 'agent' : group.user_hold ? 'held' : group.agent_state === 'stuck' ? 'stuck' : 'user'
  const handBack = () => { if (talks) setAsking('hand_back'); else void store.handBack() }
  const handToAgent = () => { if (talks) setAsking('hand_to_agent'); else void store.handToAgent() }
  // The Agent's question is answered in the conversation: while that is put away, the one action is the way to it.
  const answering = group.controller !== 'agent' && !group.user_hold && group.agent_state === 'waiting_answer'
  return (
    <div className="ab-handover" data-testid="ab-status" data-controller={group.controller} data-agent-state={group.agent_state} data-user-hold={group.user_hold ? 'true' : 'false'} data-tone={tone}>
      <span className="ab-handover-dot" aria-hidden />
      <div className="ab-handover-text" role="status">
        <p className="ab-handover-main" data-testid="ab-status-operator">{operatorText(t, group, name)}</p>
        <p className="ab-handover-sub" data-testid="ab-status-agent">{agentText(t, group, name)}</p>
      </div>
      {chat === 'stage' || (answering && chat === 'closed') ? null : <ChatToggle />}
      {group.controller === 'agent' ? (
        <button type="button" className="ab-button ab-button-primary" disabled={busy} data-testid="ab-take-over" onClick={() => void store.takeOver()}>{t('agentBrowser.action.takeOver', 'Take over')}</button>
      ) : group.user_hold ? (
        <button type="button" className="ab-button ab-button-primary" disabled={busy} data-testid="ab-hand-back" onClick={handBack}>{t('agentBrowser.action.handBack', 'Hand back')}</button>
      ) : answering ? (
        chat === 'closed' ? <button type="button" className="ab-button ab-button-primary" data-testid="ab-answer" onClick={() => store.setChatOpen(true)}>{t('agentBrowser.action.answer', 'Answer')}</button> : null
      ) : (
        <button type="button" className="ab-button" disabled={busy} data-testid="ab-hand-to-agent" onClick={handToAgent}>{t('agentBrowser.action.handToAgent', 'Hand to Agent')}</button>
      )}
      {asking === 'hand_back' ? <HandBackSheet onClose={() => setAsking(null)} /> : asking === 'hand_to_agent' ? <HandToAgentSheet onClose={() => setAsking(null)} /> : null}
    </div>
  )
}
