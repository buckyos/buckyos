import {
  ArrowLeft,
  Bot,
  FileUp,
  Lock,
  Menu,
  MoreVertical,
  SlidersHorizontal,
  SquarePen,
  User,
  Users,
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
  type ConversationComposerHandle,
  type ConversationComposerSubmitPayload,
} from './conversation/input/ConversationComposer'
import { isTransferWithFiles, type ComposerAttachmentInput } from './conversation/input/attachmentDraft'
import { ConversationMediaScopeContext } from './conversation/media/context'
import type { DID, MessageObject } from './protocol/msgobj'
import type { Entity, Session, SessionAccess, MessageHubContext } from './types'
import { useMessageHubRuntime, useMessageHubStore, type EntityAdmission } from './store'
import { friendlyDidName } from './api/projection'
import { parseGroupNotice } from './groupModel'

interface ConversationViewProps {
  entity: Entity
  session: Session | null
  messageReader: ConversationMessageReader
  selfDid: DID
  onBack: () => void
  onOpenSessionSidebar: () => void
  onOpenDetails: () => void
  onSendMessage: (payload: ConversationComposerSubmitPayload) => void | Promise<void>
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
  historyStatus?: 'idle' | 'loading' | 'ready' | 'error'
  hasOlder?: boolean
  onLoadOlder?: () => Promise<boolean>
  onVisibleMessages?: (recordIds: string[]) => void
  admission?: EntityAdmission | null
  onAdmission?: (action: 'accept' | 'block') => Promise<void>
  onOpenEntity?: (id: string) => void
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
  onResend,
  context: contextProp, access, title = session?.title ?? '', onOpenSessionDetails, onCreate, creationReason, defaultSessionError, onRetryDefaultSession, draft, draftAttachments, onAttachmentsChange, onDraftChange, showActions = true,
  leadingPane = null,
  isSessionSidebarOpen = false,
  historyStatus = 'ready',
  hasOlder = false,
  onLoadOlder,
  onVisibleMessages,
  admission = null,
  onAdmission,
  onOpenEntity,
}: ConversationViewProps) {
  const { t } = useI18n()
  const store = useMessageHubStore()
  useMessageHubRuntime()
  const context = contextProp ?? store.defaultContext()
  const runtime = session ? store.runtimeFor(context, session.id) : []
  const [admissionPending, setAdmissionPending] = useState(false)
  const [admissionError, setAdmissionError] = useState(false)
  const requestCount = session?.requestCount ?? 0
  const runAdmission = async (action: 'accept' | 'block') => {
    if (!onAdmission || admissionPending) return
    setAdmissionPending(true); setAdmissionError(false)
    try { await onAdmission(action) } catch { setAdmissionError(true) } finally { setAdmissionPending(false) }
  }
  const canSend = access === undefined || access?.mode === 'read_write'
  const isGroup = entity.type === 'group'
  const subtitle = session ? [title, session.binding.kind === 'tunnel' ? session.binding.connectionName : ''].filter(Boolean).join(' · ') : t('messagehub.noSessions')
  const admissionDetail = [
    t('messagehub.requestBanner', undefined, { count: requestCount }),
    admission?.accessLevel ? t(`messagehub.access.${admission.accessLevel}`) : '',
    admission?.temporaryExpiresAt ? t('messagehub.temporaryUntil', undefined, { time: new Date(admission.temporaryExpiresAt).toLocaleString() }) : '',
  ].filter(Boolean).join(' · ')
  const bodyRef = useRef<HTMLDivElement>(null)
  const composerRef = useRef<ConversationComposerHandle>(null)
  const dragDepthRef = useRef(0)
  const historyPaneRef = useRef<ConversationHistoryPaneHandle>(null)
  const [isDropActive, setIsDropActive] = useState(false)
  const [composerMaxHeight, setComposerMaxHeight] = useState<number | undefined>(undefined)
  const mediaScope = useMemo(() => ({ reader: messageReader, hostContext: session?.id }), [messageReader, session?.id])
  const snapshot = store.getSnapshot()
  const isOwner = context.mode === 'self' && context.ownerDid === context.viewerDid
  const messageActions = useMemo(() => ({
    resend: canSend ? onResend : undefined,
    displayName: (did: DID) => did === context.ownerDid ? t('messagehub.you') : store.findEntity(context, did)?.name ?? friendlyDidName(did, false),
    groupNotice: (message: MessageObject) => {
      const notice = parseGroupNotice(message)
      if (!notice) return null
      return notice.invitation ? store.groupInvitation(context, notice.invitation) : { groupName: store.findEntity(context, notice.groupDid)?.name ?? friendlyDidName(notice.groupDid, false) }
    },
    joinGroup: isOwner ? (message: MessageObject) => {
      const invitation = parseGroupNotice(message)?.invitation
      return invitation ? store.acceptGroupInvitation(context, invitation) : Promise.reject(Error('not-found'))
    } : undefined,
    openEntity: onOpenEntity,
  }), [canSend, onResend, store, snapshot, context.ownerDid, context.viewerDid, context.mode, isOwner, onOpenEntity, t]) // eslint-disable-line react-hooks/exhaustive-deps

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

        <button onClick={onOpenSessionSidebar} aria-label={t('messagehub.sessions')} title={t('messagehub.sessions')} className="flex min-h-11 min-w-11 items-center justify-center" style={{ color: isSessionSidebarOpen ? 'var(--cp-accent)' : 'var(--cp-muted)' }} type="button"><Menu size={18} /></button>
        <div className="min-w-0 flex-1">
          {/* On touch layouts the whole title block is one entity target; session
              details stay on the ⋮ button. */}
          <button onClick={onOpenDetails} className="flex min-h-11 w-full min-w-0 flex-col justify-center text-left md:hidden" type="button" aria-label={`${t('messagehub.entityDetails')}: ${entity.name}`}>
            <span className="flex max-w-full items-center gap-1.5"><EntityTypeIcon type={entity.type} /><span className="truncate text-base font-semibold">{entity.name}</span></span>
            <span className="block max-w-full truncate text-xs text-[color:var(--cp-muted)]">{subtitle}</span>
          </button>
          <button onClick={onOpenDetails} className="hidden max-w-full items-center gap-1.5 text-left md:flex" type="button" aria-label={`${t('messagehub.entityDetails')}: ${entity.name}`} title={t('messagehub.entityDetails')}><EntityTypeIcon type={entity.type} /><span className="truncate text-[15px] font-semibold">{entity.name}</span></button>
          <button onClick={onOpenSessionDetails} disabled={!session} type="button" className="hidden max-w-full truncate text-xs text-[color:var(--cp-muted)] md:block" aria-label={t('messagehub.sessionDetails')} title={t('messagehub.sessionDetails')}>{subtitle}</button>
          <div role="status" data-testid="session-runtime" className="truncate text-xs text-[color:var(--cp-accent)]">{runtime.map(state => `${state.memberDid === context.ownerDid ? t('messagehub.you') : session?.members[state.memberDid]?.nickname || entity.name} · ${t(`messagehub.runtime.${state.status}`)}${state.statusLine ? ` · ${state.statusLine}` : ''}`).join(' · ')}</div>
        </div>
        {onCreate && <button type="button" onClick={onCreate} disabled={!!creationReason} title={creationReason ? t(`messagehub.reason.${creationReason}`) : t('messagehub.newSession')} aria-label={t('messagehub.newSession')} className="flex min-h-11 min-w-11 items-center justify-center rounded-lg text-[color:var(--cp-muted)] disabled:opacity-40"><SquarePen size={18} /></button>}
        <button onClick={onOpenSessionDetails} disabled={!session} aria-label={t('messagehub.sessionDetails')} title={t('messagehub.sessionDetails')} className="flex min-h-11 min-w-11 items-center justify-center rounded-lg text-[color:var(--cp-muted)] disabled:opacity-40" type="button"><MoreVertical size={18} /></button>
      </div>
      {session && requestCount > 0 && <div role="note" data-testid="request-banner" title={admissionDetail} className="flex shrink-0 items-center gap-2 border-b border-[color:var(--cp-border)] bg-[color:color-mix(in_srgb,var(--cp-warning)_12%,var(--cp-surface))] px-3 py-1 text-[13px]">
        <span className="min-w-0 flex-1 truncate font-medium">{t('messagehub.requestPending', undefined, { count: requestCount })}{admission?.accessLevel && admission.accessLevel !== 'stranger' ? <span className="font-normal text-[color:var(--cp-muted)]"> · {t(`messagehub.access.${admission.accessLevel}`)}</span> : null}</span>
        {admission?.canChange && admission.accessLevel !== 'friend' && <button type="button" disabled={admissionPending} className="min-h-11 shrink-0 rounded-lg px-3 font-medium text-[color:var(--cp-accent)] disabled:opacity-40 md:min-h-9" onClick={() => void runAdmission('accept')}>{t('messagehub.acceptContact')}</button>}
        {admission?.canChange && admission.accessLevel !== 'block' && <button type="button" disabled={admissionPending} className="min-h-11 shrink-0 rounded-lg px-3 text-[color:var(--cp-danger)] disabled:opacity-40 md:min-h-9" onClick={() => void runAdmission('block')}>{t('messagehub.blockContact')}</button>}
      </div>}
      {session && requestCount > 0 && admissionError && <p role="alert" className="shrink-0 px-3 py-1 text-xs text-[color:var(--cp-danger)]">{t('messagehub.admissionFailed')}</p>}

      {defaultSessionError && <div role="alert" className="flex shrink-0 items-center gap-2 px-3 py-2 text-xs text-[color:var(--cp-danger)]"><span>{t('messagehub.operationFailed')}</span><button type="button" className="min-h-8 rounded-lg border border-[color:var(--cp-border)] px-2" onClick={onRetryDefaultSession}>{t('messagehub.retry')}</button></div>}
      <div className="relative flex min-h-0 flex-1">
        {leadingPane}

        <div
          ref={bodyRef}
          className="flex min-h-0 flex-1 flex-col"
        >
          <div
            className="flex flex-1 min-h-0 flex-col"
            style={{ minHeight: MIN_HISTORY_PANE_HEIGHT }}
          >
            <ConversationMediaScopeContext.Provider value={mediaScope}>
              <ConversationMessageActionsContext.Provider value={messageActions}>
              <ConversationHistoryPane
                ref={historyPaneRef}
                reader={messageReader}
                showActions={showActions}
                emptyLabel={t(!session ? 'messagehub.noSessions' : historyStatus === 'loading' ? 'messagehub.loadingHistory' : historyStatus === 'error' ? 'messagehub.historyFailed' : canSend ? 'messagehub.startConversation' : 'messagehub.noMessages')}
                selfDid={selfDid}
                isGroup={isGroup}
                peerMarkdown={entity.type === 'agent'}
                hasOlder={hasOlder}
                onLoadOlder={onLoadOlder}
                onVisibleMessages={onVisibleMessages}
              />
              </ConversationMessageActionsContext.Provider>
            </ConversationMediaScopeContext.Provider>
          </div>

          {canSend ? <ConversationComposer
            initialDraft={draft}
            initialAttachments={draftAttachments}
            onAttachmentsChange={onAttachmentsChange}
            onDraftChange={onDraftChange}
            ref={composerRef}
            placeholder={t('messagehub.inputPlaceholder', 'Message...')}
            maxHeight={composerMaxHeight}
            onSendMessage={handleSendMessage}
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


function EntityTypeIcon({ type }: { type: string }) {
  switch (type) {
    case 'agent':
      return <Bot size={16} />
    case 'group':
      return <Users size={16} />
    case 'service':
      return <SlidersHorizontal size={16} />
    default:
      return <User size={16} />
  }
}
