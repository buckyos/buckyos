import { FileUp, Lock, Pin, Search, Ticket } from 'lucide-react'
import { useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { useI18n } from '../../i18n/provider'
import { AgentControlsContext } from './agentControls'
import { ConversationClockContext } from './clock'
import { ConversationMessageActionsContext } from './history/actions'
import { useGenerationBreaks, useReaderWithBreaks } from './history/breaks'
import { ConversationHistoryPane, type ConversationHistoryPaneHandle } from './history/ConversationHistoryPane'
import type { ConversationMessageReader } from './history/types'
import { isTransferWithFiles } from './input/attachmentDraft'
import { ConversationComposer, type ComposerAnswer, type ComposerRelation, type ConversationComposerHandle, type ConversationComposerSubmitPayload } from './input/ConversationComposer'
import { foregroundState, latestTurnAnchor, type ForegroundMode } from './input/foregroundState'
import { conversationInputState } from './input/inputState'
import { ConversationMediaScopeContext } from './media/context'
import { buildMessageActions, type ConversationHostActions } from './messageActions'
import { toOutgoing } from './model/outgoing'
import { ConversationSearchBar } from './search/ConversationSearchBar'
import { SessionPanel } from './SessionPanel'
import type { ConversationSource } from './source'
import { useSwipe } from './swipe'
import { useTaskSummary } from './tasks'
import { TurnControls } from './tasks/TurnControls'
import { taskWorklogTarget } from './worklog/model'
import './conversation.css'

const MIN_HISTORY_PANE_HEIGHT = 180

/** What a host header gets: the button that opens the search (focus goes back to it when the search closes). */
export interface ConversationHeaderControls {
  /** null when the conversation cannot be searched. */
  searchButton: ReactNode
}

export interface ConversationSurfaceProps extends ConversationHostActions {
  source: ConversationSource
  /** Optional parts, all on by default. */
  features?: { search?: boolean; drop?: boolean }
  /** Shown in an empty, loaded history. */
  emptyLabel?: string
  placeholder?: string
  /** Shown instead of the composer when nothing can be sent and the source gives no reason. */
  readOnlyLabel?: string
  /** Why nothing may be sent right now, in words (the conversation is being archived). */
  sendBlockedReason?: string
  /** ObjId of a message to bring into view (a link, or the message whose details are open). */
  focusMessageId?: string | null
  /** The host's header; without one a slim bar carries the runtime status and the search. */
  header?: (controls: ConversationHeaderControls) => ReactNode
  /** Under the header, above the history (the host's banners). */
  banner?: ReactNode
  /** Beside the history and the composer (MessageHub's session list). */
  leadingPane?: ReactNode
  /** Extra content of the session panel, shown when it is expanded. */
  panelDetails?: ReactNode
  /** Rows to show instead of the source's history (a host-side filter); the source's stay behind search and media. */
  historyReader?: ConversationMessageReader
  /** false: nothing under the history, not even the read-only note. */
  composer?: boolean
  /** Texts the host carries out as commands instead of sending them (`ConversationComposer`). */
  commands?: Record<string, () => void>
  /** A swipe to the left over the history (touch layouts). */
  onSwipeLeft?: () => void
  /** What the conversation in front is doing: the Agent's Turn (`foregroundState`), or `answer` while its question is open. */
  onTurnMode?: (mode: ForegroundMode | 'answer') => void
}

/**
 * The body of one conversation (§0 of `notepads/conversation-ui-component-todo.md`): the session panel, the
 * history, the runtime status, the blocked line with the Turn controls and the composer, with dropping files and
 * searching as options. It only knows its `source`; the host draws its own header around it and handles what the
 * host actions name.
 */
export function ConversationSurface({
  source,
  features,
  emptyLabel,
  placeholder,
  readOnlyLabel,
  sendBlockedReason,
  focusMessageId = null,
  header,
  banner,
  leadingPane = null,
  panelDetails,
  historyReader: filteredReader,
  composer = true,
  commands,
  onSwipeLeft,
  onTurnMode,
  onOpenMessage,
  onForward,
  onOpenEntity,
  onOpenProfile,
}: ConversationSurfaceProps) {
  const { t } = useI18n()
  const { sessionId, selfDid, group } = source
  const isGroup = !!group
  const canSend = source.access?.mode === 'read_write'
  const breaks = useGenerationBreaks(selfDid, source.agent && sessionId ? sessionId : undefined)
  const markedReader = useReaderWithBreaks(source.reader, breaks, t('messagehub.agentPanel.generationBreak'), sessionId || undefined)
  const historyReader = filteredReader ?? markedReader
  const bodyRef = useRef<HTMLDivElement>(null)
  const composerRef = useRef<ConversationComposerHandle>(null)
  const dragDepthRef = useRef(0)
  const historyPaneRef = useRef<ConversationHistoryPaneHandle>(null)
  const [isDropActive, setIsDropActive] = useState(false)
  const [composerMaxHeight, setComposerMaxHeight] = useState<number | undefined>(undefined)
  const mediaScope = useMemo(() => ({ reader: source.reader, hostContext: source.hostContext ?? (sessionId || undefined) }), [source.reader, source.hostContext, sessionId])
  const [relation, setRelation] = useState<ComposerRelation | null>(null)
  // The input state follows the timeline: an open question locks the composer to answering it and replaces a reply
  // or edit the user picked.
  const timeline = source.reader.timeline
  const inputState = useMemo(() => conversationInputState(timeline ?? [], selfDid, { group: isGroup }), [timeline, selfDid, isGroup])
  const answerState = inputState.mode === 'answer' ? inputState : null
  if (answerState && relation) setRelation(null)
  // The message in front: while the Agent handles it (or a restart interrupted it) nothing new is sent. An open
  // question is answered all the same.
  const ownAgent = !!source.agent
  const turnAnchor = useMemo(() => ownAgent ? latestTurnAnchor(timeline ?? [], selfDid) : null, [ownAgent, timeline, selfDid])
  const turnTask = useTaskSummary(turnAnchor?.taskId).snapshot
  const turnDetail = turnTask?.status === 'ready' ? turnTask.task : undefined
  const foreground = foregroundState(turnAnchor, turnDetail)
  const turnTarget = turnDetail ? taskWorklogTarget(turnDetail) : null
  const sendBlocked = !answerState && (foreground.mode !== 'idle' || !!sendBlockedReason)
  const turnMode = answerState ? 'answer' : foreground.mode
  useEffect(() => { onTurnMode?.(turnMode) }, [onTurnMode, turnMode])
  const outerClock = useContext(ConversationClockContext)
  const outerControls = useContext(AgentControlsContext)
  const controls = source.agentControls ?? outerControls
  const turnAgentDid = turnTarget?.agentDid
  const turnSessionId = turnTarget?.sessionId
  const stopAnswerTurn = useMemo(() => ownAgent && controls && turnAgentDid && turnSessionId ? () => controls.stop(turnAgentDid, turnSessionId, 'turn') : undefined, [ownAgent, controls, turnAgentDid, turnSessionId])
  const [panelInset, setPanelInset] = useState(0)
  const you = t('messagehub.you')
  const messageActions = useMemo(
    () => buildMessageActions(source, { onOpenMessage, onForward, onOpenEntity, onOpenProfile }, { canSend, answering: !!answerState, you, setRelation }),
    [source, onOpenMessage, onForward, onOpenEntity, onOpenProfile, canSend, answerState, you],
  )
  const displayName = messageActions.displayName ?? source.displayName
  const answer = useMemo<ComposerAnswer | null>(() => answerState ? { ...answerState, askerName: displayName(answerState.question.from) } : null, [answerState, displayName])
  const pinned = source.preferences.pinned
  const setPinned = source.preferences.setPinned

  const locate = source.locate
  useEffect(() => {
    if (!focusMessageId || !sessionId) return
    historyPaneRef.current?.scrollToMessage(focusMessageId)
    void locate?.(focusMessageId).catch(() => false)
  }, [focusMessageId, sessionId]) // eslint-disable-line react-hooks/exhaustive-deps

  const [searching, setSearching] = useState(false)
  const searchButtonRef = useRef<HTMLButtonElement>(null)
  const searchable = features?.search !== false && !!sessionId
  const openSearch = useMemo(() => searchable ? () => setSearching(true) : undefined, [searchable])
  const focusSearchHit = useCallback((messageId: string) => historyPaneRef.current?.scrollToMessage(messageId), [])
  const closeSearch = useCallback(() => {
    setSearching(false)
    requestAnimationFrame(() => searchButtonRef.current?.focus())
  }, [])
  const searchButton = (floating: boolean) => openSearch ? <button ref={searchButtonRef} type="button" onClick={openSearch} title={t('messagehub.search.open')} aria-label={t('messagehub.search.open')} className={floating ? 'mh-conversation-search' : 'flex min-h-11 min-w-11 items-center justify-center rounded-lg text-[color:var(--cp-muted)]'} style={floating ? { top: panelInset > 0 ? panelInset + 8 : 8 } : undefined} data-testid="conversation-search-open"><Search size={floating ? 16 : 18} /></button> : null

  // The composer may take up to half of the conversation body.
  useEffect(() => {
    const element = bodyRef.current
    if (!element) return
    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) setComposerMaxHeight(Math.floor(entry.contentRect.height / 2))
    })
    observer.observe(element)
    return () => observer.disconnect()
  }, [])

  const send = source.send
  const handleSendMessage = useCallback(async (payload: ConversationComposerSubmitPayload) => {
    if (!canSend) throw Error('permission_denied')
    historyPaneRef.current?.scrollToBottom()
    await send(toOutgoing(payload))
    historyPaneRef.current?.scrollToBottom()
  }, [send, canSend])
  const stageOutgoing = source.stageOutgoing
  const handleQueueChange = useMemo(() => stageOutgoing ? (queued: ConversationComposerSubmitPayload[]) => stageOutgoing(queued.map(toOutgoing)) : undefined, [stageOutgoing])

  const swipe = useSwipe({ onLeft: onSwipeLeft })

  const canDrop = features?.drop !== false && canSend && source.attachments !== false
  const handleDragEnter = (event: React.DragEvent<HTMLDivElement>) => {
    if (!canDrop || !isTransferWithFiles(event.dataTransfer)) return
    event.preventDefault()
    dragDepthRef.current += 1
    setIsDropActive(true)
  }
  const handleDragOver = (event: React.DragEvent<HTMLDivElement>) => {
    if (!canDrop || !isTransferWithFiles(event.dataTransfer)) return
    event.preventDefault()
    event.dataTransfer.dropEffect = 'copy'
    setIsDropActive(true)
  }
  const handleDragLeave = (event: React.DragEvent<HTMLDivElement>) => {
    if (!canDrop || !isTransferWithFiles(event.dataTransfer)) return
    event.preventDefault()
    dragDepthRef.current = Math.max(0, dragDepthRef.current - 1)
    if (dragDepthRef.current === 0) setIsDropActive(false)
  }
  const handleDrop = (event: React.DragEvent<HTMLDivElement>) => {
    if (!canDrop || !isTransferWithFiles(event.dataTransfer)) return
    event.preventDefault()
    dragDepthRef.current = 0
    setIsDropActive(false)
    void composerRef.current?.addTransferData(event.dataTransfer)
  }

  const runtime = source.runtime.map(state => `${displayName(state.memberDid)} · ${t(`messagehub.runtime.${state.status}`)}${state.statusLine ? ` · ${state.statusLine}` : ''}`).join(' · ')
  const readOnlyReason = source.access?.readOnlyReason || source.unavailableReason
  const historyEmpty = source.historyStatus === 'loading' ? t('messagehub.loadingHistory') : source.historyStatus === 'error' ? t('messagehub.historyFailed') : emptyLabel ?? t(canSend ? 'messagehub.startConversation' : 'messagehub.noMessages')

  const body = (
    <div
      className="relative flex flex-col h-full"
      onDragEnter={handleDragEnter}
      onDragLeave={handleDragLeave}
      onDragOver={handleDragOver}
      onDrop={handleDrop}
      style={{ background: 'var(--cp-message-canvas)' }}
    >
      {searching && searchable ? (
        <ConversationSearchBar reader={historyReader} hasOlder={source.hasOlder} onLoadOlder={source.loadOlder} onFocus={focusSearchHit} onClose={closeSearch} />
      ) : header ? header({ searchButton: searchButton(false) }) : null}
      {banner}
      <div className="relative flex min-h-0 flex-1">
        {leadingPane}

        <div ref={bodyRef} className="flex min-h-0 flex-1 flex-col">
          <div className="relative flex flex-1 min-h-0 flex-col" style={{ minHeight: MIN_HISTORY_PANE_HEIGHT }} {...swipe}>
            {source.panel ? (
              <SessionPanel key={`custom:${sessionId}`} icon={<Ticket size={15} />} title={source.panel.title} text={source.panel.text} fields={source.panel.fields} onCollapsedHeight={setPanelInset} testId="session-panel">{panelDetails}</SessionPanel>
            ) : pinned && sessionId ? (
              <SessionPanel key={`pin:${pinned.id}`} icon={<Pin size={15} />} title={t('messagehub.panel.pinnedBy', undefined, { name: displayName(pinned.senderDid) })} text={pinned.text} dismissLabel={t('messagehub.message.unpin')} onDismiss={source.canAct && setPinned ? () => { void setPinned(null).catch(() => undefined) } : undefined} onCollapsedHeight={setPanelInset} testId="session-panel">{panelDetails}</SessionPanel>
            ) : null}
            <ConversationHistoryPane
              ref={historyPaneRef}
              reader={historyReader}
              showActions={source.preferences.showActions}
              emptyLabel={historyEmpty}
              selfDid={selfDid}
              isGroup={isGroup}
              peerMarkdown={!!source.peerMarkdown}
              hasOlder={source.hasOlder}
              onLoadOlder={source.loadOlder}
              onVisibleMessages={source.markRead}
              topInset={panelInset > 0 ? panelInset + 12 : 0}
            />
            {header || searching ? null : searchButton(true)}
          </div>

          {!header && runtime ? <div className="mh-conversation-runtime" role="status" data-testid="session-runtime">{runtime}</div> : null}
          {composer && canSend && sendBlocked ? <div className="flex shrink-0 flex-wrap items-center gap-2 border-t border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] px-3 py-1.5 text-[12px]" role="status" data-testid="composer-blocked" data-mode={sendBlockedReason ? 'archiving' : foreground.mode}>
            <span className="min-w-0 flex-1 text-[color:var(--cp-muted)]">{sendBlockedReason ?? t(`messagehub.turn.blocked.${foreground.mode}`)}</span>
            {!sendBlockedReason && foreground.anchor ? <TurnControls anchor={foreground.anchor.message} taskId={foreground.anchor.taskId} /> : null}
          </div> : null}
          {!composer ? null : canSend ? <ConversationComposer
            initialDraft={source.draft?.text}
            initialAttachments={source.draft?.attachments}
            onAttachmentsChange={source.draft?.saveAttachments}
            onDraftChange={source.draft?.saveText}
            ref={composerRef}
            placeholder={placeholder ?? t('messagehub.inputPlaceholder', 'Message')}
            maxHeight={composerMaxHeight}
            onSendMessage={handleSendMessage}
            onQueueChange={handleQueueChange}
            relation={answer ? null : relation}
            onCancelRelation={() => setRelation(null)}
            answer={answer}
            mentionCandidates={group?.mentionCandidates}
            canMentionAll={group?.canMentionAll ?? false}
            allowFiles={source.attachments !== false}
            sendBlocked={sendBlocked}
            commands={commands}
            onStopTurn={answerState ? stopAnswerTurn : undefined}
            onReleaseAttachments={source.releaseAttachments}
          /> : <div className="flex shrink-0 items-center justify-center gap-2 border-t border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] px-4 py-3 text-[13px] text-[color:var(--cp-muted)]" data-testid="composer-readonly"><Lock size={14} className="shrink-0" aria-hidden /><span>{readOnlyReason ? t(`messagehub.reason.${readOnlyReason}`) : readOnlyLabel ?? t('messagehub.noSessions')}</span></div>}
        </div>
      </div>

      {isDropActive ? (
        <div className="pointer-events-none absolute inset-0 z-30 flex items-center justify-center p-5" style={{ background: 'color-mix(in srgb, var(--cp-accent) 12%, transparent)' }}>
          <div
            className="flex max-w-sm flex-col items-center gap-2 rounded-[28px] px-6 py-5 text-center"
            style={{
              background: 'color-mix(in srgb, var(--cp-surface) 94%, white)',
              border: '1px solid color-mix(in srgb, var(--cp-accent) 26%, var(--cp-border))',
              boxShadow: '0 20px 60px color-mix(in srgb, var(--cp-shadow) 18%, transparent)',
            }}
          >
            <div className="rounded-full p-3" style={{ background: 'color-mix(in srgb, var(--cp-accent) 16%, transparent)', color: 'var(--cp-accent)' }}>
              <FileUp size={20} />
            </div>
            <p className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{t('messagehub.dropFilesTitle', 'Drop files or folders to attach')}</p>
            <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('messagehub.dropFilesHint', 'Everything you drop here will be added to the current draft.')}</p>
          </div>
        </div>
      ) : null}
    </div>
  )

  return (
    <ConversationClockContext.Provider value={source.clock ?? outerClock}>
      <AgentControlsContext.Provider value={controls}>
        <ConversationMediaScopeContext.Provider value={mediaScope}>
          <ConversationMessageActionsContext.Provider value={messageActions}>
            {body}
          </ConversationMessageActionsContext.Provider>
        </ConversationMediaScopeContext.Provider>
      </AgentControlsContext.Provider>
    </ConversationClockContext.Provider>
  )
}
