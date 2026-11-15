/* APP7: the conversation of the current thing — the history and the composer of MessageHub on its store, the way
 * MessageHub shows a conversation with one of the owner's own Agents: a question of the Agent (`ask_user`) is
 * answered in the composer, and while the Agent handles the message in front the Turn can be stopped from here. */

import { useMediaQuery } from '@mui/material'
import { Lock } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react'
import { LoadingOrb } from '../../../components/LoadingOrb'
import { WindowDialogProvider } from '../../../desktop/windows/dialogs'
import { useI18n } from '../../../i18n/provider'
import '../../messagehub/messagehub.css'
import { useGenerationBreaks, useReaderWithBreaks } from '../../messagehub/agentpanel/breaks'
import { getAgentPanelSource } from '../../messagehub/agentpanel/source'
import { friendlyDidName } from '../../messagehub/api/projection'
import { ConversationMessageActionsContext, type ConversationMessageActions } from '../../messagehub/conversation/history/actions'
import { ConversationHistoryPane, type ConversationHistoryPaneHandle } from '../../messagehub/conversation/history/ConversationHistoryPane'
import { messageObjId, ownReactionId } from '../../messagehub/conversation/history/relations'
import { ConversationComposer, type ComposerAnswer, type ComposerRelation, type ConversationComposerSubmitPayload } from '../../messagehub/conversation/input/ConversationComposer'
import { foregroundState, latestTurnAnchor, turnInputMessage } from '../../messagehub/conversation/input/foregroundState'
import { conversationInputState } from '../../messagehub/conversation/input/inputState'
import { ConversationMediaScopeContext } from '../../messagehub/conversation/media/context'
import { MessageMediaHost } from '../../messagehub/conversation/media/MessageMediaHost'
import { useTaskSummary } from '../../messagehub/conversation/tasks'
import { TurnControls } from '../../messagehub/conversation/tasks/TurnControls'
import { taskWorklogTarget } from '../../messagehub/conversation/worklog/model'
import { useMessageHubRuntime, type MessageHubStore, type OutgoingPayload } from '../../messagehub/store'
import type { MessageHubContext, Session } from '../../messagehub/types'
import type { BrowserAgent } from '../api/backend'
import { DEFAULT_GROUP } from '../api/types'
import { useAgentBrowser, useAgentBrowserStore } from '../store/context'
import { currentThing } from '../store/store'
import { ensureDefaultSession } from './session'
import './conversation.css'

const toOutgoing = (payload: ConversationComposerSubmitPayload): OutgoingPayload => ({
  content: payload.content,
  signal: payload.signal,
  attachments: payload.attachments.map(({ file, relativePath, directory }) => ({ file, relativePath, directory })),
  ...(payload.relatesTo ? { relatesTo: payload.relatesTo } : {}),
  ...(payload.mentions ? { mentions: payload.mentions } : {}),
  ...(payload.machine ? { machine: payload.machine } : {}),
  ...(payload.voice ? { voice: payload.voice } : {}),
  ...(payload.localId ? { localId: payload.localId, submittedAt: payload.submittedAt } : {}),
})

function SessionConversation({ hub, context, agent, session }: { hub: MessageHubStore; context: MessageHubContext; agent: BrowserAgent; session: Session }) {
  const { t } = useI18n()
  const isDesktop = useMediaQuery('(min-width: 769px)')
  useSyncExternalStore(hub.subscribe, hub.getSnapshot)
  useMessageHubRuntime()
  const sessionId = session.id
  const ownerDid = context.ownerDid
  useEffect(() => hub.startSync(context, sessionId), [hub, context, sessionId])

  const ownAgent = hub.isOwnAgent(context, agent.agentDid)
  const reader = hub.reader(context, sessionId)
  const breaks = useGenerationBreaks(context.viewerDid, ownAgent ? sessionId : undefined)
  const historyReader = useReaderWithBreaks(reader, breaks, t('messagehub.agentPanel.generationBreak'), sessionId)
  const access = hub.access(context, session, false)
  const canSend = access.mode === 'read_write'
  const historyStatus = hub.historyStatus(context, sessionId)
  const runtime = hub.runtimeFor(context, sessionId)

  // The input follows the timeline: an open question locks the composer to answering it.
  const inputState = useMemo(() => conversationInputState(reader.timeline ?? [], ownerDid), [reader, ownerDid])
  const answerState = inputState.mode === 'answer' ? inputState : null
  const [relation, setRelation] = useState<ComposerRelation | null>(null)
  if (answerState && relation) setRelation(null)
  const answer = useMemo<ComposerAnswer | null>(() => (answerState ? { ...answerState, askerName: agent.name } : null), [answerState, agent.name])

  // The message in front: while the Agent handles it nothing new is sent; an open question is answered all the same.
  const turnAnchor = useMemo(() => (ownAgent ? latestTurnAnchor(reader.timeline ?? [], ownerDid) : null), [ownAgent, reader, ownerDid])
  const turnTask = useTaskSummary(turnAnchor?.taskId).snapshot
  const turnDetail = turnTask?.status === 'ready' ? turnTask.task : undefined
  const foreground = foregroundState(turnAnchor, turnDetail)
  const turnTarget = turnDetail ? taskWorklogTarget(turnDetail) : null
  const turnAgentDid = turnTarget?.agentDid
  const turnSessionId = turnTarget?.sessionId
  const sendBlocked = !answerState && foreground.mode !== 'idle'
  const stopAnswerTurn = useMemo(() => (ownAgent && turnAgentDid && turnSessionId ? () => getAgentPanelSource().stop(turnAgentDid, turnSessionId, 'turn') : undefined), [ownAgent, turnAgentDid, turnSessionId])

  const historyRef = useRef<ConversationHistoryPaneHandle>(null)
  const bodyRef = useRef<HTMLDivElement>(null)
  const [composerMax, setComposerMax] = useState<number | undefined>(undefined)
  useEffect(() => {
    const body = bodyRef.current
    if (!body) return
    const observer = new ResizeObserver(() => setComposerMax(Math.floor(body.clientHeight / 2)))
    observer.observe(body)
    return () => observer.disconnect()
  }, [])

  const send = useCallback(async (payload: ConversationComposerSubmitPayload) => {
    historyRef.current?.scrollToBottom()
    await hub.send(context, sessionId, toOutgoing(payload), undefined)
    historyRef.current?.scrollToBottom()
  }, [hub, context, sessionId])

  const actions = useMemo<ConversationMessageActions>(() => {
    const relate = (target: string | undefined, rel: 'redact' | 'reaction', key?: string) => (target && canSend
      ? hub.send(context, sessionId, { content: key ?? '', attachments: [], relatesTo: { rel, target, ...(key ? { key } : {}) } }, undefined)
      : Promise.reject(Error('permission_denied')))
    return {
      resend: canSend ? (message) => hub.resend(context, sessionId, message, undefined) : undefined,
      displayName: (did) => (did === ownerDid ? t('messagehub.you') : did === agent.agentDid ? agent.name : hub.findEntity(context, did)?.name ?? friendlyDidName(did, false)),
      relations: {
        capabilities: () => ({ reply: canSend && !answerState, react: canSend, edit: false, redact: canSend, recall: false }),
        reply: (message) => setRelation({ kind: 'reply', message }),
        edit: () => undefined,
        redact: (message) => relate(messageObjId(message), 'redact'),
        react: (message, key) => relate(messageObjId(message), 'reaction', key),
        unreact: (message, key) => relate(ownReactionId(message, ownerDid, key), 'redact'),
      },
      remove: (message) => hub.deleteMessage(context, sessionId, message),
      agentTurn: ownAgent ? {
        inputOf: (anchor, task) => {
          const keys = (task?.input as { input_keys?: unknown } | undefined)?.input_keys
          return canSend ? turnInputMessage(reader.timeline ?? [], anchor, ownerDid, Array.isArray(keys) ? keys.filter((key): key is string => typeof key === 'string') : []) : null
        },
        retract: (input) => relate(messageObjId(input), 'redact'),
      } : undefined,
      handleNotify: true,
    }
  }, [hub, context, sessionId, ownerDid, agent.agentDid, agent.name, canSend, answerState, ownAgent, reader, t, setRelation])

  const mediaScope = useMemo(() => ({ reader, hostContext: sessionId }), [reader, sessionId])
  const emptyLabel = historyStatus === 'loading' ? t('messagehub.loadingHistory') : historyStatus === 'error' ? t('messagehub.historyFailed') : t('agentBrowser.chat.empty', 'Tell {{name}} what you want done. It works in this browser and asks here when it needs something from you.', { name: agent.name })

  return (
    <WindowDialogProvider surface={isDesktop ? 'desktop' : 'mobile'} permissions={{ fullscreen: false }}>
      <MessageMediaHost>
        <div ref={bodyRef} className="ab-chat" data-testid="ab-conversation" data-session={sessionId}>
          <div className="ab-chat-history">
            <ConversationMediaScopeContext.Provider value={mediaScope}>
              <ConversationMessageActionsContext.Provider value={actions}>
                <ConversationHistoryPane
                  ref={historyRef}
                  reader={historyReader}
                  selfDid={ownerDid}
                  isGroup={false}
                  peerMarkdown
                  showActions={hub.preferences(context, sessionId).showActions}
                  emptyLabel={emptyLabel}
                  hasOlder={hub.hasOlder(context, sessionId)}
                  onLoadOlder={() => hub.loadOlder(context, sessionId)}
                  onVisibleMessages={(recordIds) => { void hub.markRead(context, sessionId, recordIds) }}
                />
              </ConversationMessageActionsContext.Provider>
            </ConversationMediaScopeContext.Provider>
          </div>
          {runtime.length > 0 ? (
            <div className="ab-chat-runtime" role="status" data-testid="ab-conversation-runtime">
              {runtime.map((state) => `${state.memberDid === ownerDid ? t('messagehub.you') : agent.name} · ${t(`messagehub.runtime.${state.status}`)}${state.statusLine ? ` · ${state.statusLine}` : ''}`).join(' · ')}
            </div>
          ) : null}
          {canSend && sendBlocked ? (
            <div className="flex shrink-0 flex-wrap items-center gap-2 border-t border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] px-3 py-1.5 text-[12px]" role="status" data-testid="composer-blocked" data-mode={foreground.mode}>
              <span className="min-w-0 flex-1 text-[color:var(--cp-muted)]">{t(`messagehub.turn.blocked.${foreground.mode}`)}</span>
              {foreground.anchor ? <ConversationMessageActionsContext.Provider value={actions}><TurnControls anchor={foreground.anchor.message} taskId={foreground.anchor.taskId} /></ConversationMessageActionsContext.Provider> : null}
            </div>
          ) : null}
          {canSend ? (
            <ConversationComposer
              initialDraft={hub.draft(context, sessionId)}
              initialAttachments={hub.attachments(context, sessionId)}
              onAttachmentsChange={(attachments) => hub.saveAttachments(context, sessionId, attachments)}
              onDraftChange={(value) => hub.saveDraft(context, sessionId, value)}
              placeholder={t('agentBrowser.chat.input', 'Message {{name}}…', { name: agent.name })}
              maxHeight={composerMax}
              onSendMessage={send}
              onQueueChange={(queued) => hub.stageOutgoing(context, sessionId, queued.map(toOutgoing))}
              relation={answer ? null : relation}
              onCancelRelation={() => setRelation(null)}
              answer={answer}
              sendBlocked={sendBlocked}
              onStopTurn={answerState ? stopAnswerTurn : undefined}
            />
          ) : (
            <div className="flex shrink-0 items-center justify-center gap-2 border-t border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] px-4 py-3 text-[13px] text-[color:var(--cp-muted)]" data-testid="composer-readonly">
              <Lock size={14} className="shrink-0" aria-hidden />
              <span>{access.readOnlyReason ? t(`messagehub.reason.${access.readOnlyReason}`) : t('agentBrowser.chat.readOnly', 'Nothing can be sent in this conversation.')}</span>
            </div>
          )}
        </div>
      </MessageMediaHost>
    </WindowDialogProvider>
  )
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
 * The conversation of the current thing. The default thing has none until it is first shown: then the owner's
 * default conversation with the Agent is bound to the default group (protocol §1).
 */
export function ThingConversation() {
  const store = useAgentBrowserStore()
  const { t } = useI18n()
  const conversations = useAgentBrowser((snapshot) => snapshot.conversations)
  const agent = useAgentBrowser((snapshot) => snapshot.agent)
  const groupId = useAgentBrowser((snapshot) => snapshot.groupId)
  const things = useAgentBrowser((snapshot) => snapshot.things)
  const session = currentThing({ things, groupId }).session
  const [attempt, setAttempt] = useState(0)
  const [failure, setFailure] = useState(-1)
  const binding = conversations === 'ready' && groupId === DEFAULT_GROUP && !session
  useEffect(() => {
    const thing = binding ? store.thingContext() : null
    if (!thing) return
    let alive = true
    ensureDefaultSession(thing).then(() => store.refreshStatus(), () => { if (alive) setFailure(attempt) })
    return () => { alive = false }
  }, [binding, store, attempt])

  const name = agent?.name ?? ''
  const thing = store.thingContext()
  if (conversations === 'unavailable') {
    return <Status testId="ab-conversation-unavailable" title={t('agentBrowser.chat.unavailable', 'Your conversations cannot be reached right now.')} detail={t('agentBrowser.chat.unavailableHint', 'You can keep browsing. Try again to talk to {{name}}.', { name })} action={{ label: t('common.retry'), onClick: () => store.retryConversations() }} />
  }
  if (session && thing) return <SessionConversation key={session.id} hub={thing.hub} context={thing.context} agent={thing.agent} session={session} />
  if (conversations === 'ready' && groupId !== DEFAULT_GROUP) {
    return <Status testId="ab-conversation-gone" title={t('agentBrowser.chat.gone', 'The conversation of this task was archived or deleted.')} detail={t('agentBrowser.chat.goneHint', 'Its tabs are still here. Choose another task to talk to {{name}}.', { name })} />
  }
  if (binding && failure === attempt) {
    return <Status testId="ab-conversation-failed" title={t('agentBrowser.chat.bindFailed', 'The conversation with {{name}} could not be opened.', { name })} action={{ label: t('common.retry'), onClick: () => setAttempt((value) => value + 1) }} />
  }
  return <Status testId="ab-conversation-loading" loading title={t('agentBrowser.chat.loading', 'Opening the conversation…')} />
}
