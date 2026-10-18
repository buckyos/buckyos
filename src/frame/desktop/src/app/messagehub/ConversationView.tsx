import {
  ArrowLeft,
  FileUp,
  Lock,
  Menu,
  MoreVertical,
  Pin,
  Ticket,
  SquarePen,
} from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { useI18n } from '../../i18n/provider'
import {
  ConversationHistoryPane,
  type ConversationHistoryPaneHandle,
} from './conversation/history/ConversationHistoryPane'
import { ConversationMessageActionsContext } from './conversation/history/actions'
import type { ConversationMessageReader } from './conversation/history/types'
import {
  ConversationComposer,
  type ComposerAnswer,
  type ComposerRelation,
  type ConversationComposerHandle,
  type ConversationComposerSubmitPayload,
} from './conversation/input/ConversationComposer'
import { conversationInputState } from './conversation/input/inputState'
import { messageObjId, messageSummaryText, ownReactionId } from './conversation/history/relations'
import { SessionPanel } from './conversation/SessionPanel'
import { isTransferWithFiles, type ComposerAttachmentInput } from './conversation/input/attachmentDraft'
import { ConversationMediaScopeContext } from './conversation/media/context'
import { notifySessionCategory, type DID, type MessageObject } from './protocol/msgobj'
import { useNotifyFilteredReader } from './notify'
import { NOTIFY_FILTERS, type NotifyFilter } from './notify/model'
import type { Entity, Session, SessionAccess, MessageHubContext } from './types'
import { useMessageHubRuntime, useMessageHubStore } from './store'
import { RequestBanner } from './RelationPanel'
import { friendlyDidName } from './api/projection'
import { parseGroupNotice, withinWindow } from './groupModel'
import { HubAvatar } from './HubAvatar'

interface ConversationViewProps {
  entity: Entity
  session: Session | null
  messageReader: ConversationMessageReader
  selfDid: DID
  onBack: () => void
  onOpenSessionSidebar: () => void
  onOpenDetails: () => void
  onSendMessage: (payload: ConversationComposerSubmitPayload) => void | Promise<void>
  onQueueChange?: (queued: ConversationComposerSubmitPayload[]) => void
  onResend?: (message: MessageObject) => Promise<void>
  context?: MessageHubContext
  access?: SessionAccess | null
  title?: string
  onOpenSessionDetails?: () => void
  onCreate?: () => void
  creationReason?: string
  defaultSessionError?: boolean
  onRetryDefaultSession?: () => void
  draft?: string
  draftAttachments?: ComposerAttachmentInput[]
  onAttachmentsChange?: (attachments: ComposerAttachmentInput[]) => Promise<void> | undefined
  onDraftChange?: (value: string) => Promise<void> | undefined
  showActions?: boolean
  sessionCount: number
  leadingPane?: ReactNode
  isSessionSidebarOpen?: boolean
  /** Unread messages of the entity's other sessions, shown on the session list button. */
  otherSessionsUnread?: number
  historyStatus?: 'idle' | 'loading' | 'ready' | 'error'
  hasOlder?: boolean
  onLoadOlder?: () => Promise<boolean>
  onVisibleMessages?: (recordIds: string[]) => void
  onOpenEntity?: (id: string) => void
  /** Opens the forward target picker for a message. */
  onForward?: (message: MessageObject) => void
  /** Extra content of the session panel, shown when it is expanded (host-defined). */
  sessionPanelDetails?: ReactNode
  /** Opens the details of a message. */
  onOpenMessage?: (message: MessageObject) => void
  /** ObjId of a message to bring into view (a deep link, or the message whose details are open). */
  focusMessageId?: string | null
}

const MIN_HISTORY_PANE_HEIGHT = 180

export function ConversationView({
  entity,
  session,
  messageReader,
  selfDid,
  onBack,
  onOpenSessionSidebar,
  onOpenDetails,
  onSendMessage,
  onQueueChange,
  onResend,
  context: contextProp, access, title = session?.title ?? '', onOpenSessionDetails, onCreate, creationReason, defaultSessionError, onRetryDefaultSession, draft, draftAttachments, onAttachmentsChange, onDraftChange, showActions = true,
  leadingPane = null,
  isSessionSidebarOpen = false,
  otherSessionsUnread = 0,
  historyStatus = 'ready',
  hasOlder = false,
  onLoadOlder,
  onVisibleMessages,
  onOpenEntity,
  onForward,
  sessionPanelDetails,
  onOpenMessage,
  focusMessageId = null,
}: ConversationViewProps) {
  const { t } = useI18n()
  const store = useMessageHubStore()
  useMessageHubRuntime()
  const context = contextProp ?? store.defaultContext()
  const runtime = session ? store.runtimeFor(context, session.id) : []
  const requestCount = session?.requestCount ?? 0
  const canSend = access === undefined || access?.mode === 'read_write'
  // A notification session (`sysnotify:*`) is not a conversation: its cards are filtered, never answered by typing.
  const notifyCategory = session ? notifySessionCategory(session.id) : undefined
  const [notifyFilter, setNotifyFilter] = useState<NotifyFilter>('all')
  const historyReader = useNotifyFilteredReader(messageReader, notifyCategory ? notifyFilter : 'all')
  const isGroup = entity.type === 'group'
  const subtitle = session ? [title, session.binding.kind === 'tunnel' ? session.binding.connectionName : ''].filter(Boolean).join(' · ') : t('messagehub.noSessions')
  const bodyRef = useRef<HTMLDivElement>(null)
  const composerRef = useRef<ConversationComposerHandle>(null)
  const dragDepthRef = useRef(0)
  const historyPaneRef = useRef<ConversationHistoryPaneHandle>(null)
  const [isDropActive, setIsDropActive] = useState(false)
  const [composerMaxHeight, setComposerMaxHeight] = useState<number | undefined>(undefined)
  const mediaScope = useMemo(() => ({ reader: messageReader, hostContext: session?.id }), [messageReader, session?.id])
  const snapshot = store.getSnapshot()
  const isOwner = context.mode === 'self' && context.ownerDid === context.viewerDid
  const [relation, setRelation] = useState<ComposerRelation | null>(null)
  const sessionId = session?.id
  // The input state follows the timeline: an open question locks the
  // composer to answering it and replaces a reply or edit the user picked.
  const inputState = useMemo(() => conversationInputState(messageReader.timeline ?? [], context.ownerDid, { group: isGroup }), [messageReader, context.ownerDid, isGroup])
  const answerState = inputState.mode === 'answer' ? inputState : null
  if (answerState && relation) setRelation(null)
  const pinned = session ? store.preferences(context, session.id).pinnedMessage ?? null : null
  const pinnedId = pinned?.id
  const [panelInset, setPanelInset] = useState(0)
  const group = isGroup ? store.group(context, entity.id) : null
  const groupMembers = group?.members
  const groupHosted = group?.hosted === true
  useEffect(() => { if (groupHosted && sessionId) void store.ensureGroupSessionMembers(context, entity.id, sessionId) }, [store, groupHosted, entity.id, sessionId, context.ownerDid, context.viewerDid, context.mode]) // eslint-disable-line react-hooks/exhaustive-deps
  // Mentions go to the session's participants (guests included); the group's
  // members stand in until those are known.
  const sessionMembers = isGroup && sessionId ? store.groupSessionMembers(context, entity.id, sessionId) : null
  const mentionCandidates = useMemo(() => {
    const dids = sessionMembers ? sessionMembers.items.filter(item => item.state === 'included').map(item => item.did) : (groupMembers ?? []).filter(member => member.state === 'active').map(member => member.did)
    return dids.filter(did => did !== context.ownerDid).map(did => ({ did, name: store.findEntity(context, did)?.name ?? friendlyDidName(did, false) }))
  }, [sessionMembers, groupMembers, store, context.ownerDid, context.viewerDid, context.mode]) // eslint-disable-line react-hooks/exhaustive-deps
  const messageActions = useMemo(() => {
    const notice = (message: MessageObject, operation: (notice: NonNullable<ReturnType<typeof parseGroupNotice>>) => Promise<void>) => {
      const parsed = parseGroupNotice(message)
      return parsed ? operation(parsed) : Promise.reject(Error('not-found'))
    }
    const sendRelation = (target: string | undefined, rel: 'redact' | 'reaction', key?: string) => {
      if (!target || !canSend) return Promise.reject(Error('permission_denied'))
      return Promise.resolve(onSendMessage({ attachments: [], content: key ?? '', relatesTo: { rel, target, ...(key ? { key } : {}) } }))
    }
    return {
      resend: canSend ? onResend : undefined,
      displayName: (did: DID) => did === context.ownerDid ? t('messagehub.you') : store.findEntity(context, did)?.name ?? friendlyDidName(did, false),
      groupNotice: (message: MessageObject) => {
        const parsed = parseGroupNotice(message)
        if (!parsed) return null
        return parsed.invitation ? store.groupInvitation(context, parsed.invitation) : { groupName: store.findEntity(context, parsed.groupDid)?.name ?? friendlyDidName(parsed.groupDid, false) }
      },
      joinGroup: isOwner ? (message: MessageObject) => notice(message, parsed => parsed.invitation ? store.acceptGroupInvitation(context, parsed.invitation) : Promise.reject(Error('not-found'))) : undefined,
      approveMember: isOwner ? (message: MessageObject) => notice(message, parsed => parsed.memberDid ? store.approveGroupMember(context, parsed.groupDid, parsed.memberDid) : Promise.reject(Error('not-found'))) : undefined,
      rejectMember: isOwner ? (message: MessageObject) => notice(message, parsed => parsed.memberDid ? store.rejectGroupMember(context, parsed.groupDid, parsed.memberDid) : Promise.reject(Error('not-found'))) : undefined,
      acceptOwnerTransfer: isOwner ? (message: MessageObject) => notice(message, parsed => parsed.transferId ? store.acceptGroupOwnerTransfer(context, parsed.groupDid, parsed.transferId) : Promise.reject(Error('transfer-mismatch'))) : undefined,
      acceptSessionInvitation: isOwner ? (message: MessageObject) => notice(message, parsed => parsed.sessionId ? store.acceptGroupSessionInvitation(context, parsed.groupDid, parsed.sessionId) : Promise.reject(Error('not-found'))) : undefined,
      openEntity: onOpenEntity,
      // Relations are plain messages with `relates_to`, so a direct session
      // offers reactions, replies and delete requests too; edits follow group rules.
      relations: isOwner && sessionId ? {
        capabilities: (message: MessageObject) => {
          const own = message.from === context.ownerDid, now = store.now()
          const edit = isGroup && canSend && own && withinWindow(group?.messageRules.editWindowMs, message.created_at_ms, now)
          // Outside a group a redact is a request the peer's client honours by
          // default; nothing enforces it, so either side may ask for any message.
          const redact = canSend && (!isGroup || (own ? withinWindow(group?.messageRules.recallWindowMs, message.created_at_ms, now) : group?.can.redactAny ?? false))
          // While a question is open nothing but its answer may be sent.
          return { reply: canSend && !answerState, react: canSend, edit: edit && !answerState, redact, recall: redact && isGroup && own }
        },
        reply: (message: MessageObject) => setRelation({ kind: 'reply', message }),
        edit: (message: MessageObject) => setRelation({ kind: 'edit', message }),
        redact: (message: MessageObject) => sendRelation(messageObjId(message), 'redact'),
        react: (message: MessageObject, key: string) => sendRelation(messageObjId(message), 'reaction', key),
        unreact: (message: MessageObject, key: string) => sendRelation(ownReactionId(message, context.ownerDid, key), 'redact'),
      } : undefined,
      openDetails: sessionId ? onOpenMessage : undefined,
      forward: isOwner && sessionId ? onForward : undefined,
      remove: isOwner && sessionId ? (message: MessageObject) => store.deleteMessage(context, sessionId, message) : undefined,
      pin: isOwner && sessionId ? {
        isPinned: (message: MessageObject) => !!pinnedId && messageObjId(message) === pinnedId,
        toggle: (message: MessageObject) => {
          const id = messageObjId(message)
          if (!id) return Promise.reject(Error('not-found'))
          return store.updatePreferences(context, sessionId, { pinnedMessage: id === pinnedId ? null : { id, text: messageSummaryText(message).slice(0, 2000), senderDid: message.from, createdAt: message.created_at_ms } })
        },
      } : undefined,
      readReceipt: sessionId ? (message: MessageObject) => store.readReceipt(context, sessionId, message) : undefined,
      handleNotify: isOwner,
      answerFriendRequests: isOwner,
    }
  }, [canSend, onResend, onSendMessage, store, snapshot, context.ownerDid, context.viewerDid, context.mode, isOwner, isGroup, sessionId, group, onOpenEntity, onForward, onOpenMessage, pinnedId, t, answerState]) // eslint-disable-line react-hooks/exhaustive-deps
  const askerName = answerState ? messageActions.displayName(answerState.question.from) : ''
  const answer = useMemo<ComposerAnswer | null>(() => answerState ? { ...answerState, askerName } : null, [answerState, askerName])

  useEffect(() => {
    if (!focusMessageId || !sessionId) return
    historyPaneRef.current?.scrollToMessage(focusMessageId)
    void store.locateMessage(context, sessionId, focusMessageId).catch(() => false)
  }, [focusMessageId, sessionId, store, context.ownerDid, context.viewerDid, context.mode]) // eslint-disable-line react-hooks/exhaustive-deps

  // Observe body height to compute composer max (50% of conversation body)
  useEffect(() => {
    const element = bodyRef.current
    if (!element) {
      return
    }

    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        setComposerMaxHeight(Math.floor(entry.contentRect.height / 2))
      }
    })

    observer.observe(element)

    return () => {
      observer.disconnect()
    }
  }, [])

  const handleSendMessage = useCallback(async (payload: ConversationComposerSubmitPayload) => {
    if (!canSend) throw Error('permission_denied')
    historyPaneRef.current?.scrollToBottom()
    await onSendMessage(payload)
    historyPaneRef.current?.scrollToBottom()
  }, [onSendMessage, canSend])

  const handleDragEnter = (event: React.DragEvent<HTMLDivElement>) => {
    if (!canSend || !isTransferWithFiles(event.dataTransfer)) {
      return
    }

    event.preventDefault()
    dragDepthRef.current += 1
    setIsDropActive(true)
  }

  const handleDragOver = (event: React.DragEvent<HTMLDivElement>) => {
    if (!canSend || !isTransferWithFiles(event.dataTransfer)) {
      return
    }

    event.preventDefault()
    event.dataTransfer.dropEffect = 'copy'
    setIsDropActive(true)
  }

  const handleDragLeave = (event: React.DragEvent<HTMLDivElement>) => {
    if (!canSend || !isTransferWithFiles(event.dataTransfer)) {
      return
    }

    event.preventDefault()
    dragDepthRef.current = Math.max(0, dragDepthRef.current - 1)

    if (dragDepthRef.current === 0) {
      setIsDropActive(false)
    }
  }

  const handleDrop = (event: React.DragEvent<HTMLDivElement>) => {
    if (!canSend || !isTransferWithFiles(event.dataTransfer)) {
      return
    }

    event.preventDefault()
    dragDepthRef.current = 0
    setIsDropActive(false)
    void composerRef.current?.addTransferData(event.dataTransfer)
  }


  return (
    <div
      className="relative flex flex-col h-full"
      onDragEnter={handleDragEnter}
      onDragLeave={handleDragLeave}
      onDragOver={handleDragOver}
      onDrop={handleDrop}
      style={{ background: 'var(--cp-message-canvas)' }}
    >
      <div
        className="flex items-center gap-1 px-2 py-1.5 flex-shrink-0 md:gap-2 md:px-3"
        style={{
          borderBottom: '1px solid var(--cp-border)',
          background: 'var(--cp-surface)',
        }}
      >
        <button
          onClick={onBack}
          className="-ml-2 flex min-h-11 min-w-11 items-center justify-center rounded-lg md:hidden"
          style={{ color: 'var(--cp-accent)' }}
          type="button"
          aria-label={t('messagehub.backToList')}
          title={t('messagehub.backToList')}
        >
          <ArrowLeft size={20} />
        </button>

        <button onClick={onOpenSessionSidebar} aria-label={t('messagehub.sessions')} title={otherSessionsUnread > 0 ? t('messagehub.otherSessionsUnread', '{{count}} unread in other sessions', { count: otherSessionsUnread }) : t('messagehub.sessions')} className="relative flex min-h-11 min-w-11 items-center justify-center" style={{ color: isSessionSidebarOpen ? 'var(--cp-accent)' : 'var(--cp-muted)' }} type="button"><Menu size={18} />{otherSessionsUnread > 0 && <span data-testid="other-sessions-unread" className="absolute right-0.5 top-1 flex h-4 min-w-4 items-center justify-center rounded-full bg-[color:var(--cp-accent)] px-1 text-[10px] font-semibold leading-none text-white">{otherSessionsUnread}</span>}</button>
        <div className="min-w-0 flex-1">
          {/* On touch layouts the whole title block is one entity target; session
              details stay on the ⋮ button. */}
          <button onClick={onOpenDetails} className="flex min-h-11 w-full min-w-0 flex-col justify-center text-left md:hidden" type="button" aria-label={`${t('messagehub.entityDetails')}: ${entity.name}`}>
            <span className="flex max-w-full items-center gap-1.5"><HubAvatar type={entity.type} name={entity.name} avatar={entity.avatar} size={20} systemNotify={entity.systemNotify} /><span className="truncate text-base font-semibold">{entity.name}</span></span>
            <span className="block max-w-full truncate text-xs text-[color:var(--cp-muted)]">{subtitle}</span>
          </button>
          <button onClick={onOpenDetails} className="hidden max-w-full items-center gap-1.5 text-left md:flex" type="button" aria-label={`${t('messagehub.entityDetails')}: ${entity.name}`} title={t('messagehub.entityDetails')}><HubAvatar type={entity.type} name={entity.name} avatar={entity.avatar} size={22} systemNotify={entity.systemNotify} /><span className="truncate text-[15px] font-semibold">{entity.name}</span></button>
          <button onClick={onOpenSessionDetails} disabled={!session} type="button" className="hidden max-w-full truncate text-xs text-[color:var(--cp-muted)] md:block" aria-label={t('messagehub.sessionDetails')} title={t('messagehub.sessionDetails')}>{subtitle}</button>
          <div role="status" data-testid="session-runtime" className="truncate text-xs text-[color:var(--cp-accent)]">{runtime.map(state => `${state.memberDid === context.ownerDid ? t('messagehub.you') : session?.members[state.memberDid]?.nickname || entity.name} · ${t(`messagehub.runtime.${state.status}`)}${state.statusLine ? ` · ${state.statusLine}` : ''}`).join(' · ')}</div>
        </div>
        {onCreate && <button type="button" onClick={onCreate} disabled={!!creationReason} title={creationReason ? t(`messagehub.reason.${creationReason}`) : t('messagehub.newSession')} aria-label={t('messagehub.newSession')} className="flex min-h-11 min-w-11 items-center justify-center rounded-lg text-[color:var(--cp-muted)] disabled:opacity-40"><SquarePen size={18} /></button>}
        <button onClick={onOpenSessionDetails} disabled={!session} aria-label={t('messagehub.sessionDetails')} title={t('messagehub.sessionDetails')} className="flex min-h-11 min-w-11 items-center justify-center rounded-lg text-[color:var(--cp-muted)] disabled:opacity-40" type="button"><MoreVertical size={18} /></button>
      </div>
      {session ? <RequestBanner entity={entity} context={context} requestCount={requestCount} /> : null}

      {notifyCategory ? <div role="group" aria-label={t('messagehub.notify.filter.label')} className="flex shrink-0 items-center gap-1.5 overflow-x-auto border-b border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] px-3 py-1.5" data-testid="notify-filter">
        {NOTIFY_FILTERS.map(value => <button key={value} type="button" className="mh-notify-filter" aria-pressed={notifyFilter === value} onClick={() => setNotifyFilter(value)} data-filter={value}>{t(`messagehub.notify.filter.${value}`)}</button>)}
      </div> : null}
      {defaultSessionError && <div role="alert" className="flex shrink-0 items-center gap-2 px-3 py-2 text-xs text-[color:var(--cp-danger)]"><span>{t('messagehub.operationFailed')}</span><button type="button" className="min-h-8 rounded-lg border border-[color:var(--cp-border)] px-2" onClick={onRetryDefaultSession}>{t('messagehub.retry')}</button></div>}
      <div className="relative flex min-h-0 flex-1">
        {leadingPane}

        <div
          ref={bodyRef}
          className="flex min-h-0 flex-1 flex-col"
        >
          <div
            className="relative flex flex-1 min-h-0 flex-col"
            style={{ minHeight: MIN_HISTORY_PANE_HEIGHT }}
          >
            {session?.panel ? (
              <SessionPanel key={`custom:${session.id}`} icon={<Ticket size={15} />} title={session.panel.title} text={session.panel.text} fields={session.panel.fields} onCollapsedHeight={setPanelInset} testId="session-panel">{sessionPanelDetails}</SessionPanel>
            ) : pinned && sessionId ? (
              <SessionPanel key={`pin:${pinned.id}`} icon={<Pin size={15} />} title={t('messagehub.panel.pinnedBy', undefined, { name: messageActions.displayName(pinned.senderDid) })} text={pinned.text} dismissLabel={t('messagehub.message.unpin')} onDismiss={isOwner ? () => { void store.updatePreferences(context, sessionId, { pinnedMessage: null }).catch(() => undefined) } : undefined} onCollapsedHeight={setPanelInset} testId="session-panel">{sessionPanelDetails}</SessionPanel>
            ) : null}
            <ConversationMediaScopeContext.Provider value={mediaScope}>
              <ConversationMessageActionsContext.Provider value={messageActions}>
              <ConversationHistoryPane
                ref={historyPaneRef}
                reader={historyReader}
                showActions={showActions}
                emptyLabel={t(!session ? 'messagehub.noSessions' : historyStatus === 'loading' ? 'messagehub.loadingHistory' : historyStatus === 'error' ? 'messagehub.historyFailed' : notifyCategory ? `messagehub.notify.empty.${notifyFilter}` : canSend ? 'messagehub.startConversation' : 'messagehub.noMessages')}
                selfDid={selfDid}
                isGroup={isGroup}
                peerMarkdown={entity.type === 'agent'}
                hasOlder={hasOlder}
                onLoadOlder={onLoadOlder}
                onVisibleMessages={onVisibleMessages}
                topInset={panelInset > 0 ? panelInset + 12 : 0}
              />
              </ConversationMessageActionsContext.Provider>
            </ConversationMediaScopeContext.Provider>
          </div>

          {notifyCategory ? null : canSend ? <ConversationComposer
            initialDraft={draft}
            initialAttachments={draftAttachments}
            onAttachmentsChange={onAttachmentsChange}
            onDraftChange={onDraftChange}
            ref={composerRef}
            placeholder={t('messagehub.inputPlaceholder', 'Message...')}
            maxHeight={composerMaxHeight}
            onSendMessage={handleSendMessage}
            onQueueChange={onQueueChange}
            relation={answer ? null : relation}
            onCancelRelation={() => setRelation(null)}
            answer={answer}
            mentionCandidates={isGroup ? mentionCandidates : undefined}
            canMentionAll={isGroup && (group?.can.mentionAll ?? false)}
          /> : <div className="flex shrink-0 items-center justify-center gap-2 border-t border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] px-4 py-3 text-[13px] text-[color:var(--cp-muted)]" data-testid="composer-readonly"><Lock size={14} className="shrink-0" aria-hidden /><span>{access?.readOnlyReason ? t(`messagehub.reason.${access.readOnlyReason}`) : creationReason ? t(`messagehub.reason.${creationReason}`) : t('messagehub.noSessions')}</span></div>}
        </div>
      </div>

      {isDropActive ? (
        <div
          className="pointer-events-none absolute inset-0 z-30 flex items-center justify-center p-5"
          style={{
            background: 'color-mix(in srgb, var(--cp-accent) 12%, transparent)',
          }}
        >
          <div
            className="flex max-w-sm flex-col items-center gap-2 rounded-[28px] px-6 py-5 text-center"
            style={{
              background: 'color-mix(in srgb, var(--cp-surface) 94%, white)',
              border: '1px solid color-mix(in srgb, var(--cp-accent) 26%, var(--cp-border))',
              boxShadow: '0 20px 60px color-mix(in srgb, var(--cp-shadow) 18%, transparent)',
            }}
          >
            <div
              className="rounded-full p-3"
              style={{
                background: 'color-mix(in srgb, var(--cp-accent) 16%, transparent)',
                color: 'var(--cp-accent)',
              }}
            >
              <FileUp size={20} />
            </div>
            <p
              className="text-sm font-semibold"
              style={{ color: 'var(--cp-text)' }}
            >
              {t('messagehub.dropFilesTitle', 'Drop files or folders to attach')}
            </p>
            <p
              className="text-xs"
              style={{ color: 'var(--cp-muted)' }}
            >
              {t(
                'messagehub.dropFilesHint',
                'Everything you drop here will be added to the current draft.',
              )}
            </p>
          </div>
        </div>
      ) : null}
    </div>
  )
}


