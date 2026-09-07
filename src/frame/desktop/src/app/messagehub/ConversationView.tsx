import {
  ArrowLeft,
  Bot,
  FileUp,
  Menu,
  MoreVertical,
  SlidersHorizontal,
  User,
  Users,
} from 'lucide-react'
import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react'
import { useI18n } from '../../i18n/provider'
import {
  ConversationHistoryPane,
  type ConversationHistoryPaneHandle,
} from './conversation/history/ConversationHistoryPane'
import type { ConversationMessageReader } from './conversation/history/types'
import {
  ConversationComposer,
  type ConversationComposerHandle,
  type ConversationComposerSubmitPayload,
} from './conversation/input/ConversationComposer'
import { isTransferWithFiles, type ComposerAttachmentInput } from './conversation/input/attachmentDraft'
import type { DID } from './protocol/msgobj'
import type { Entity, Session, SessionAccess, MessageHubContext } from './types'
import { useMessageHubRuntime, useMessageHubStore, type EntityAdmission } from './store'

interface ConversationViewProps {
  entity: Entity
  session: Session | null
  messageReader: ConversationMessageReader
  selfDid: DID
  onBack: () => void
  onOpenSessionSidebar: () => void
  onOpenDetails: () => void
  onSendMessage: (payload: ConversationComposerSubmitPayload) => void | Promise<void>
  context?: MessageHubContext
  access?: SessionAccess | null
  title?: string
  onOpenSessionDetails?: () => void
  onCreate?: () => void
  creationReason?: string
  draft?: string
  draftAttachments?: ComposerAttachmentInput[]
  onAttachmentsChange?: (attachments: ComposerAttachmentInput[]) => Promise<void> | undefined
  onDraftChange?: (value: string) => Promise<void> | undefined
  showActions?: boolean
  onShowActions?: (value: boolean) => Promise<void>
  sessionCount: number
  leadingPane?: ReactNode
  isSessionSidebarOpen?: boolean
  historyStatus?: 'idle' | 'loading' | 'ready' | 'error'
  hasOlder?: boolean
  onLoadOlder?: () => Promise<boolean>
  onVisibleMessages?: (recordIds: string[]) => void
  admission?: EntityAdmission | null
  onAdmission?: (action: 'accept' | 'block') => Promise<void>
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
  context: contextProp, access, title = session?.title ?? '', onOpenSessionDetails, onCreate, creationReason, draft, draftAttachments, onAttachmentsChange, onDraftChange, showActions = true, onShowActions,
  leadingPane = null,
  isSessionSidebarOpen = false,
  historyStatus = 'ready',
  hasOlder = false,
  onLoadOlder,
  onVisibleMessages,
  admission = null,
  onAdmission,
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
  const [filterError, setFilterError] = useState(false)
  const [pendingFilter, setPendingFilter] = useState<boolean | null>(null)
  const isGroup = entity.type === 'group'
  const bodyRef = useRef<HTMLDivElement>(null)
  const composerRef = useRef<ConversationComposerHandle>(null)
  const dragDepthRef = useRef(0)
  const historyPaneRef = useRef<ConversationHistoryPaneHandle>(null)
  const [isDropActive, setIsDropActive] = useState(false)
  const [composerMaxHeight, setComposerMaxHeight] = useState<number | undefined>(undefined)

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
      style={{ background: 'var(--cp-bg)' }}
    >
      <div
        className="flex items-center gap-2 px-3 py-2 flex-shrink-0"
        style={{
          borderBottom: '1px solid var(--cp-border)',
          background: 'var(--cp-surface)',
        }}
      >
        <button
          onClick={onBack}
          className="p-1.5 rounded-lg md:hidden"
          style={{ color: 'var(--cp-accent)' }}
          type="button"
        >
          <ArrowLeft size={20} />
        </button>

        <button onClick={onOpenSessionSidebar} aria-label={t('messagehub.sessions')} className="p-2 min-h-11" style={{ color: isSessionSidebarOpen ? 'var(--cp-accent)' : 'var(--cp-muted)' }} type="button"><Menu size={18} /></button>
        <div className="min-w-0 flex-1">
          <button onClick={onOpenDetails} className="flex max-w-full items-center gap-1.5 text-left" type="button" aria-label={`${t('messagehub.entityDetails')}: ${entity.name}`}><EntityTypeIcon type={entity.type} /><span className="truncate text-sm font-semibold">{entity.name}</span></button>
          <button onClick={onOpenSessionDetails} disabled={!session} type="button" className="block max-w-full truncate text-xs text-[color:var(--cp-muted)]" aria-label={t('messagehub.sessionDetails')}>{session ? title : t('messagehub.noSessions')}</button>
          <div role="status" data-testid="session-runtime" className="truncate text-xs text-[color:var(--cp-accent)]">{runtime.map(state => `${state.memberDid === context.ownerDid ? t('messagehub.you') : session?.members[state.memberDid]?.nickname || entity.name} · ${t(`messagehub.runtime.${state.status}`)}${state.statusLine ? ` · ${state.statusLine}` : ''}`).join(' · ')}</div>
        </div>
        {onCreate && <button type="button" onClick={onCreate} disabled={!!creationReason} title={creationReason ? t(`messagehub.reason.${creationReason}`) : t('messagehub.newSession')} aria-label={t('messagehub.newSession')} className="min-h-11 min-w-11 text-lg disabled:opacity-40">+</button>}
        <button onClick={onOpenSessionDetails} disabled={!session} aria-label={t('messagehub.sessionDetails')} className="min-h-11 p-2 disabled:opacity-40" type="button"><MoreVertical size={18} /></button>
      </div>
      {session && requestCount > 0 && <div role="note" data-testid="request-banner" className="flex shrink-0 flex-wrap items-center gap-2 border-b border-[color:var(--cp-border)] bg-[color:color-mix(in_srgb,var(--cp-warning)_10%,transparent)] px-3 py-2 text-xs">
        <span className="flex-1">{t('messagehub.requestBanner', undefined, { count: requestCount })}{admission?.accessLevel ? ` · ${t(`messagehub.access.${admission.accessLevel}`)}` : ''}{admission?.temporaryExpiresAt ? ` · ${t('messagehub.temporaryUntil', undefined, { time: new Date(admission.temporaryExpiresAt).toLocaleString() })}` : ''}</span>
        {admission?.canChange && admission.accessLevel !== 'friend' && <button type="button" disabled={admissionPending} className="min-h-8 rounded-lg border border-[color:var(--cp-border)] px-2" onClick={() => void runAdmission('accept')}>{t('messagehub.acceptContact')}</button>}
        {admission?.canChange && admission.accessLevel !== 'block' && <button type="button" disabled={admissionPending} className="min-h-8 rounded-lg border border-[color:var(--cp-border)] px-2 text-[color:var(--cp-danger)]" onClick={() => void runAdmission('block')}>{t('messagehub.blockContact')}</button>}
        {admissionError && <span role="alert">{t('messagehub.operationFailed')}</span>}
      </div>}
      {session && onShowActions && <div className="flex shrink-0 flex-wrap items-center justify-between gap-x-2 px-3 py-1 text-[11px] text-[color:var(--cp-muted)]">
        <span>{session.binding.kind === 'tunnel' ? session.binding.connectionName : 'BuckyOS'} · {t(canSend ? 'messagehub.readWrite' : 'messagehub.readOnly')}</span>
        <label className="flex min-h-8 items-center gap-1"><input type="checkbox" checked={pendingFilter ?? showActions} disabled={pendingFilter !== null} onChange={event => { const value = event.target.checked; setPendingFilter(value); setFilterError(false); void onShowActions?.(value).catch(() => setFilterError(true)).finally(() => setPendingFilter(null)) }} />{t('messagehub.showActions')}</label>
        {filterError && <span role="alert">{t('messagehub.operationFailed')}</span>}
      </div>}

      <div className="flex min-h-0 flex-1">
        {leadingPane}

        <div
          ref={bodyRef}
          className="flex min-h-0 flex-1 flex-col"
        >
          <div
            className="flex flex-1 min-h-0 flex-col"
            style={{ minHeight: MIN_HISTORY_PANE_HEIGHT }}
          >
            <ConversationHistoryPane
              ref={historyPaneRef}
              reader={messageReader}
              showActions={showActions}
              emptyLabel={t(!session ? 'messagehub.noSessions' : historyStatus === 'loading' ? 'messagehub.loadingHistory' : historyStatus === 'error' ? 'messagehub.historyFailed' : canSend ? 'messagehub.startConversation' : 'messagehub.noMessages')}
              selfDid={selfDid}
              isGroup={isGroup}
              hasOlder={hasOlder}
              onLoadOlder={onLoadOlder}
              onVisibleMessages={onVisibleMessages}
            />
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
          /> : <div className="p-4 text-center text-xs text-[color:var(--cp-muted)]" data-testid="composer-readonly">{access?.readOnlyReason ? t(`messagehub.reason.${access.readOnlyReason}`) : creationReason ? t(`messagehub.reason.${creationReason}`) : t('messagehub.noSessions')}</div>}
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
