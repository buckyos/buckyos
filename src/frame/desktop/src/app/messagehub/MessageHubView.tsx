import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { useMediaQuery } from '@mui/material'
import { ChevronLeft, ChevronRight, House, ImagePlay, MessageSquare, SquarePen, Users } from 'lucide-react'
import './messagehub.css'
import { useI18n } from '../../i18n/provider'
import { ConversationView } from './ConversationView'
import { ForwardMessageForm } from './ForwardDialog'
import { InMemoryConversationMessageReader } from './conversation/history/data-source'
import { useMessageMediaHost } from './conversation/media/context'
import { MessageMediaHost } from './conversation/media/MessageMediaHost'
import type { ConversationComposerSubmitPayload } from './conversation/input/ConversationComposer'
import { EntityDetails } from './EntityDetails'
import { MessageDetails } from './MessageDetails'
import { messageObjId } from './conversation/history/relations'
import { friendlyDidName } from './api/projection'
import { EntityList } from './EntityList'
import { CreateGroupForm } from './GroupDialogs'
import { resolveMessageHubContext, type MessageHubContextRequest } from './launch'
import { useMessageHubStore, useMessageHubReady, useMessageHubRuntime } from './store'
import { creationReason, viewerSessionKey } from './sessionModel'
import { usePaneResizer } from './paneResize'
import { WindowDialogProvider, useWindowDialog } from '../../desktop/windows/dialogs'
import { SessionDetails } from './SessionDetails'
import { CreateSessionForm, ManageSessionForm, hubButtonClass, hubIconButtonClass } from './SessionDialogs'
import type { MessageHubContext, Session } from './types'
import { SessionSidebar } from './SessionSidebar'
import {
  CONVERSATION_MIN_READING_WIDTH,
  DETAILS_PANEL_WIDTH,
  ENTITY_LIST_COLLAPSED_WIDTH,
  ENTITY_LIST_DEFAULT_WIDTH,
  ENTITY_LIST_MAX_WIDTH,
  ENTITY_LIST_MIN_WIDTH,
  PANEL_SPLITTER_WIDTH,
  SESSION_SIDEBAR_DEFAULT_WIDTH,
  SESSION_SIDEBAR_MAX_WIDTH,
  SESSION_SIDEBAR_MIN_WIDTH,
} from './layout'
import type {
  EntityFilter,
  MobileView,
} from './types'

const EMPTY_READER = InMemoryConversationMessageReader.empty()
const EMPTY_SESSIONS: Session[] = []

interface MessageHubViewProps {
  initialEntityId?: string | null
  /** Session to open instead of the entity's default one; without an entity, the session's own entity opens. */
  initialSessionId?: string | null
  /** ObjId of a message of that session to locate; its details open on the desktop layout. */
  initialMessageId?: string | null
  contextRequest?: MessageHubContextRequest | MessageHubContext
  windowId?: string
  /** Standalone route only: leave MessageHub for the desktop. */
  onHome?: () => void
  /** Standalone route only: drop the observed owner from the address, so a reload stays in the viewer's own messages. */
  onExitObserver?: () => void
}

export function MessageHubView({ initialEntityId = null, initialSessionId = null, initialMessageId = null, contextRequest, windowId, onHome, onExitObserver }: MessageHubViewProps) {
  const { t } = useI18n()
  const [exitFrom, setExitFrom] = useState<string | null>(null)
  const { status, retry } = useMessageHubReady()
  const isDesktop = useMediaQuery('(min-width: 769px)')
  if (status !== 'ready') return <div className="flex h-full items-center justify-center gap-3"><p role="status">{t(status === 'loading' ? 'messagehub.loading' : 'messagehub.loadFailed')}</p>{status === 'error' && <button type="button" className={hubButtonClass} onClick={retry}>{t('messagehub.retry')}</button>}</div>
  const exit = (value: string | null) => { setExitFrom(value); onExitObserver?.() }
  return <MessageHubOwnerGate initialEntityId={initialEntityId} initialSessionId={initialSessionId} initialMessageId={initialMessageId} contextRequest={contextRequest} exitFrom={exitFrom} onExit={exit} isDesktop={isDesktop} windowId={windowId} onHome={onHome} />
}

/** How long a freshly created account may wait for its mailbox permission to propagate. */
const SELF_DENIED_RETRY_DELAYS_MS = [1_000, 2_000, 4_000, 8_000, 15_000]

function MessageHubOwnerGate({ initialEntityId, initialSessionId, initialMessageId, contextRequest, exitFrom, onExit, isDesktop, windowId, onHome }: { initialEntityId: string | null; initialSessionId: string | null; initialMessageId: string | null; contextRequest?: MessageHubContextRequest | MessageHubContext; exitFrom: string | null; onExit: (value: string | null) => void; isDesktop: boolean; windowId?: string; onHome?: () => void }) {
  const { t } = useI18n()
  const store = useMessageHubStore()
  const requested = resolveMessageHubContext(store.defaultContext(), contextRequest)
  const context = exitFrom === JSON.stringify(requested) ? store.defaultContext() : requested
  const ownerStatus = store.ownerStatus(context)
  const isOwn = context.mode === 'self' && context.ownerDid === context.viewerDid
  const [selfRetry, setSelfRetry] = useState(0)
  useEffect(() => { if (store.canView(context)) void store.ensureOwner(context) }, [store, context.ownerDid, context.mode, context.viewerDid]) // eslint-disable-line react-hooks/exhaustive-deps
  // Permission for a new account's own mailbox reaches msg-center a little
  // after the account itself; the own view retries on a backoff instead of
  // stopping at "no permission".
  const selfDenied = isOwn && ownerStatus.phase === 'denied'
  useEffect(() => {
    if (!selfDenied || selfRetry >= SELF_DENIED_RETRY_DELAYS_MS.length) return
    const timer = setTimeout(() => { setSelfRetry(value => value + 1); void store.ensureOwner(context, true) }, SELF_DENIED_RETRY_DELAYS_MS[selfRetry])
    return () => clearTimeout(timer)
  }, [selfDenied, selfRetry]) // eslint-disable-line react-hooks/exhaustive-deps
  if (selfDenied || (isOwn && selfRetry > 0 && ownerStatus.phase === 'loading')) {
    const waiting = selfRetry < SELF_DENIED_RETRY_DELAYS_MS.length
    return <div role={waiting ? 'status' : 'alert'} className="flex h-full flex-col items-center justify-center gap-3 p-6 text-center">
      <p>{t(waiting ? 'messagehub.preparingMailbox' : 'messagehub.mailboxUnavailable')}</p>
      {!waiting && <button type="button" className={hubButtonClass} onClick={() => { setSelfRetry(0); void store.ensureOwner(context, true) }}>{t('messagehub.retry')}</button>}
    </div>
  }
  if (!store.canView(context)) return <div role="alert" className="flex h-full flex-col items-center justify-center gap-3 p-6"><p>{t('messagehub.reason.permission_denied')}</p>{context.mode === 'observe' && <button type="button" className={hubButtonClass} onClick={() => onExit(JSON.stringify(requested))}>{t('messagehub.exitObserver')}</button>}</div>
  if (ownerStatus.phase === 'loading' || ownerStatus.phase === 'idle') return <div className="flex h-full items-center justify-center"><p role="status">{t('messagehub.loading')}</p></div>
  if (ownerStatus.phase === 'error') return <div role="alert" className="flex h-full flex-col items-center justify-center gap-3 p-6"><p>{t('messagehub.loadFailed')}</p><p className="max-w-md break-words text-xs text-[color:var(--cp-muted)]">{ownerStatus.message}</p><button type="button" className={hubButtonClass} onClick={() => void store.ensureOwner(context, true)}>{t('messagehub.retry')}</button></div>
  return <div className="relative flex h-full min-h-0 flex-col text-[color:var(--cp-text)]">
    {context.mode === 'observe' && <div className="flex shrink-0 items-center justify-between gap-2 border-b border-[color:var(--cp-border)] p-2 text-xs" data-testid="owner-banner"><span>{t('messagehub.observing')} · {context.ownerDid.split(':').at(-1)} · {t('messagehub.readOnly')}</span><button type="button" className={hubButtonClass} onClick={() => onExit(JSON.stringify(requested))}>{t('messagehub.exitObserver')}</button></div>}
    <div className="relative min-h-0 flex-1"><MessageMediaHost windowId={windowId}><WindowDialogProvider key={JSON.stringify(context)} surface={isDesktop ? 'desktop' : 'mobile'} permissions={{ fullscreen: false }}><MessageHubContent key={`${JSON.stringify(context)}:${initialEntityId}:${initialSessionId}:${initialMessageId}`} initialEntityId={initialEntityId} initialSessionId={initialSessionId} initialMessageId={initialMessageId} context={context} onHome={onHome} /></WindowDialogProvider></MessageMediaHost></div>
  </div>
}

function HomeButton({ onHome }: { onHome: () => void }) {
  const { t } = useI18n()
  const label = t('messagehub.home', 'Back to desktop')
  return (
    <button
      type="button"
      onClick={onHome}
      className={hubIconButtonClass}
      aria-label={label}
      title={label}
      data-testid="messagehub-home"
    >
      <House size={17} />
    </button>
  )
}

function MessageHubContent({ initialEntityId, initialSessionId, initialMessageId, context, onHome }: { initialEntityId: string | null; initialSessionId: string | null; initialMessageId: string | null; context: MessageHubContext; onHome?: () => void }) {
  const { t } = useI18n()
  const isDesktop = useMediaQuery('(min-width: 769px)')
  const store = useMessageHubStore()
  const dialog = useWindowDialog()
  useMessageHubRuntime()
  // Without an explicit entity the most recent one opens (pinned first), which
  // is what the mock route did with the CodeAssistant seed.
  // Resolved once: the component is keyed on the context and the initial entity.
  // A requested session (a link from another app) wins over the default one;
  // it may be archived, and without an entity it brings its own.
  const findRequestedSession = (entityId?: string | null) => initialSessionId
    ? (['active', 'archived'] as const).flatMap(lifecycle => store.sessions(context, entityId ?? undefined, lifecycle)).find(session => session.id === initialSessionId) ?? null
    : null
  const [resolvedInitialEntityId] = useState(() => initialEntityId ? store.findEntity(context, initialEntityId)?.id ?? null : findRequestedSession()?.entityId ?? store.entities(context)[0]?.id ?? null)
  const getDefaultSessionId = (entityId: string | null) => entityId ? store.defaultSession(context, entityId)?.id ?? null : null
  const contextEpoch = useRef(0)
  const ownerDid = context.ownerDid
  useEffect(() => () => { contextEpoch.current++; store.clearTransient(ownerDid) }, [ownerDid, store])
  const [archived, setArchived] = useState(() => findRequestedSession(resolvedInitialEntityId)?.lifecycle === 'archived')
  const [writeConfirmations, setWriteConfirmations] = useState<Record<string, string>>({})

  const [selectedEntityId, setSelectedEntityId] = useState<string | null>(resolvedInitialEntityId)
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(
    () => findRequestedSession(resolvedInitialEntityId)?.id ?? getDefaultSessionId(resolvedInitialEntityId),
  )
  const [defaultSessionError, setDefaultSessionError] = useState(false)
  const resolveDefaultSession = useCallback((entityId: string, epoch: number) => {
    void store.ensureDefaultSession(context, entityId).then(session => {
      if (contextEpoch.current === epoch) {
        setSelectedSessionId(session?.id ?? null)
        setDefaultSessionError(false)
      }
    }).catch(() => { if (contextEpoch.current === epoch) setDefaultSessionError(true) })
  }, [store, context.ownerDid, context.viewerDid, context.mode]) // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    if (resolvedInitialEntityId && !findRequestedSession(resolvedInitialEntityId)) resolveDefaultSession(resolvedInitialEntityId, contextEpoch.current)
  }, [resolvedInitialEntityId, resolveDefaultSession]) // eslint-disable-line react-hooks/exhaustive-deps
  const [filter, setFilter] = useState<EntityFilter>('all')
  const [searchQuery, setSearchQuery] = useState('')
  const [mobileView, setMobileView] = useState<MobileView>(
    () => (!isDesktop && resolvedInitialEntityId ? 'conversation' : 'entity-list'),
  )
  const [showSessionSidebar, setShowSessionSidebar] = useState(false)
  // A message link only applies to the session it names: without that session the message is not looked up elsewhere.
  const [linkedMessageId] = useState(() => initialMessageId && findRequestedSession(resolvedInitialEntityId) ? initialMessageId : null)
  const [detailsTarget, setDetailsTarget] = useState<'entity' | 'session' | 'message' | null>(() => linkedMessageId && isDesktop ? 'message' : null)
  const [detailMessageId, setDetailMessageId] = useState<string | null>(linkedMessageId)
  const showDetails = detailsTarget !== null
  const [entityListDrilldownPath, setEntityListDrilldownPath] = useState<string[]>([])
  const [entityListWidth, setEntityListWidth] = useState(ENTITY_LIST_DEFAULT_WIDTH)
  const [sessionSidebarWidth, setSessionSidebarWidth] = useState(SESSION_SIDEBAR_DEFAULT_WIDTH)
  const [isEntityListCollapsed, setIsEntityListCollapsed] = useState(false)
  const [layoutWidth, setLayoutWidth] = useState(0)
  const desktopLayoutRef = useRef<HTMLDivElement>(null)
  const entityListPaneRef = useRef<HTMLDivElement>(null)
  const sessionSidebarPaneRef = useRef<HTMLDivElement>(null)
  const entityListWidthRef = useRef(ENTITY_LIST_DEFAULT_WIDTH)
  const sessionSidebarWidthRef = useRef(SESSION_SIDEBAR_DEFAULT_WIDTH)

  const clampEntityListWidth = useCallback((width: number) => (
    Math.min(Math.max(width, ENTITY_LIST_MIN_WIDTH), ENTITY_LIST_MAX_WIDTH)
  ), [])
  const clampSessionSidebarWidth = useCallback((width: number) => (
    Math.min(Math.max(width, SESSION_SIDEBAR_MIN_WIDTH), SESSION_SIDEBAR_MAX_WIDTH)
  ), [])

  useEffect(() => {
    entityListWidthRef.current = entityListWidth
  }, [entityListWidth])

  useEffect(() => {
    sessionSidebarWidthRef.current = sessionSidebarWidth
  }, [sessionSidebarWidth])

  useLayoutEffect(() => {
    const element = desktopLayoutRef.current

    if (!isDesktop || !element) {
      return
    }

    setLayoutWidth(element.clientWidth)
    const resizeObserver = new ResizeObserver(() => {
      setLayoutWidth(element.clientWidth)
      setEntityListWidth((prev) => clampEntityListWidth(prev))
      setSessionSidebarWidth((prev) => clampSessionSidebarWidth(prev))
    })

    resizeObserver.observe(element)

    return () => {
      resizeObserver.disconnect()
    }
  }, [clampEntityListWidth, clampSessionSidebarWidth, isDesktop])

  const snapshot = store.getSnapshot()
  const entities = useMemo(() => store.entities(context), [store, snapshot, context.ownerDid, context.mode, context.viewerDid]) // eslint-disable-line react-hooks/exhaustive-deps
  const findProjectedEntity = (id: string | null) => {
    const queue = [...entities]
    while (queue.length) { const item = queue.shift()!; if (item.id === id) return item; queue.push(...(item.children ?? [])) }
    return null
  }
  const selectedEntity = findProjectedEntity(selectedEntityId)
  // Without a selected entity nothing is rendered, so no session may become
  // active (an empty entity id would otherwise return every session).
  const sessions = useMemo(() => selectedEntityId ? store.sessions(context, selectedEntityId, archived ? 'archived' : 'active') : EMPTY_SESSIONS, [store, snapshot, selectedEntityId, archived, context.ownerDid, context.mode, context.viewerDid]) // eslint-disable-line react-hooks/exhaustive-deps
  const activeSession = sessions.find(session => session.id === selectedSessionId) ?? (archived ? sessions[0] : selectedEntityId ? store.defaultSession(context, selectedEntityId) : null) ?? null
  const messageReader = activeSession ? store.reader(context, activeSession.id) : EMPTY_READER
  const entityDetail = selectedEntity ? store.entityDetail(context, selectedEntity.id) : null
  const confirmed = !!activeSession && writeConfirmations[activeSession.id] === JSON.stringify(activeSession.binding)
  const access = activeSession ? store.access(context, activeSession, confirmed) : null
  const canManage = context.mode === 'self' && context.viewerDid === context.ownerDid
  const activeSessionId = activeSession?.id ?? null
  useEffect(() => store.startSync(context, activeSessionId), [store, context.ownerDid, context.mode, context.viewerDid, activeSessionId]) // eslint-disable-line react-hooks/exhaustive-deps
  const selectedGroupId = selectedEntity?.type === 'group' ? selectedEntity.id : null
  useEffect(() => { if (selectedGroupId) void store.ensureGroup(context, selectedGroupId) }, [store, context.ownerDid, context.mode, context.viewerDid, selectedGroupId]) // eslint-disable-line react-hooks/exhaustive-deps
  const createReason = selectedEntity ? (() => {
    const choices = store.connections(context, selectedEntity.id)
    return choices.some(choice => !creationReason(context, selectedEntity, store.policy(context, selectedEntity.id), choice.binding)) ? undefined : creationReason(context, selectedEntity, store.policy(context, selectedEntity.id), choices[0]?.binding)
  })() : undefined
  const openCreate = (entityId: string | null = selectedEntityId) => {
    const epoch = ++contextEpoch.current, trigger = document.activeElement
    setDefaultSessionError(false)
    void dialog.open({ title: t('messagehub.newSession'), size: 'sm', dismissible: false, renderBody: controls => <CreateSessionForm context={context} entityId={entityId} onCancel={() => { contextEpoch.current++; controls.close() }} onCreated={session => {
      controls.close()
      if (contextEpoch.current !== epoch) return
      setSelectedEntityId(session.entityId); setSelectedSessionId(session.id); setArchived(false); setDetailsTarget(null); setMobileView('conversation')
    }} /> }).then(() => { if (trigger instanceof HTMLElement && trigger.isConnected) trigger.focus() })
  }
  const openManage = (session: Session) => {
    const epoch = contextEpoch.current, trigger = document.activeElement
    void dialog.open({ title: `${t('messagehub.manageSession')}: ${store.title(context, session)}`, size: 'sm', dismissible: false, renderBody: controls => <ManageSessionForm context={context} session={session} onCancel={() => controls.close()} onDone={() => {
      controls.close()
      if (contextEpoch.current !== epoch) return
      if (session.id === activeSession?.id) { setSelectedSessionId(getDefaultSessionId(session.entityId)); setArchived(false); setDetailsTarget(null); setMobileView('conversation') }
    }} /> }).then(() => { if (trigger instanceof HTMLElement && trigger.isConnected) trigger.focus() })
  }
  const sidebarProps = {
    onCreate: () => openCreate(), onManage: openManage, onToggleArchived: () => { contextEpoch.current++; setArchived(value => !value); setSelectedSessionId(null); setDetailsTarget(null); setDefaultSessionError(false) }, archived,
    archivedCount: selectedEntityId ? store.sessions(context, selectedEntityId, 'archived').length : 0, canManage, creationReason: createReason, titleFor: (session: Session) => store.title(context, session),
    statusFor: (session: Session) => store.runtimeFor(context, session.id).map(state => t(`messagehub.runtime.${state.status}`)).join(' · '),
    badgeFor: (session: Session) => selectedEntity?.type === 'group' && store.groupSession(context, selectedEntity.id, session.id)?.hasGuests ? t('messagehub.group.hasGuests') : undefined,
  }

  const handleSelectEntity = (
    (id: string) => {
      const epoch = ++contextEpoch.current
      setArchived(false)
      setSelectedEntityId(id)
      setSelectedSessionId(getDefaultSessionId(id))
      setDefaultSessionError(false)
      resolveDefaultSession(id, epoch)
      setDetailsTarget(null)
      setShowSessionSidebar(false)

      if (!isDesktop) {
        setMobileView('conversation')
      }
    }
  )

  const openCreateGroup = (members: string[] = []) => {
    const trigger = document.activeElement
    void dialog.open({ title: t('messagehub.group.create'), size: 'md', dismissible: false, renderBody: controls => <CreateGroupForm context={context} initialMembers={members} onCancel={() => controls.close()} onCreated={groupDid => {
      controls.close()
      setSearchQuery('')
      setFilter(current => current === 'groups' || current === 'all' ? current : 'all')
      handleSelectEntity(groupDid)
    }} /> }).then(() => { if (trigger instanceof HTMLElement && trigger.isConnected) trigger.focus() })
  }

  const handleBack = useCallback(() => {
    setMobileView('entity-list')
    setDetailsTarget(null)
    setShowSessionSidebar(false)
  }, [])

  const handleOpenDetails = (target: 'entity' | 'session' = 'entity') => {
    setDetailsTarget(target)
    if (!isDesktop) setMobileView('details')
  }

  const handleCloseDetails = useCallback(() => {
    if (isDesktop) {
      setDetailsTarget(null)
      return
    }

    setMobileView('conversation')
  }, [isDesktop])

  const handleSelectSession = (id: string) => {
    contextEpoch.current++; setSelectedSessionId(id); setDefaultSessionError(false)
    if (detailsTarget === 'message') setDetailsTarget(null)
    if (!isDesktop) setShowSessionSidebar(false)
  }

  const handleCollapseEntityList = useCallback(() => {
    setIsEntityListCollapsed(true)
  }, [])

  const handleExpandEntityList = useCallback(() => {
    setIsEntityListCollapsed(false)
    setEntityListWidth(clampEntityListWidth(entityListWidthRef.current))
  }, [clampEntityListWidth])

  // While dragging, the pane width is written straight to the DOM once per
  // frame; React state (and the layout decisions derived from it) only change
  // when the drag ends, so the rest of the view does not re-render per move.
  const entityListResizer = usePaneResizer({
    getWidth: () => entityListWidthRef.current,
    clamp: clampEntityListWidth,
    preview: (width: number) => { const element = entityListPaneRef.current; if (element) element.style.width = `${width}px` },
    commit: setEntityListWidth,
    disabled: isEntityListCollapsed,
  })
  // The session list stays inline while it is dragged: a width that would
  // turn it into an overlay drawer mid-drag is not reachable by dragging.
  const sessionSidebarDragMax = Math.max(SESSION_SIDEBAR_MIN_WIDTH, Math.min(SESSION_SIDEBAR_MAX_WIDTH, layoutWidth - (isEntityListCollapsed ? ENTITY_LIST_COLLAPSED_WIDTH : entityListWidth) - CONVERSATION_MIN_READING_WIDTH))
  const sessionSidebarResizer = usePaneResizer({
    getWidth: () => sessionSidebarWidthRef.current,
    clamp: (width: number) => Math.min(clampSessionSidebarWidth(width), sessionSidebarDragMax),
    preview: (width: number) => { const element = sessionSidebarPaneRef.current; if (element) element.style.width = `${width}px` },
    commit: setSessionSidebarWidth,
  })
  const isResizingEntityList = entityListResizer.active
  const isResizingSessionSidebar = sessionSidebarResizer.active

  const handleSendMessage = async (payload: ConversationComposerSubmitPayload) => {
    if (!activeSession || !selectedEntityId) throw Error('session_missing')
    await store.send(context, activeSession.id, { content: payload.content, attachments: payload.attachments.map(({ file, relativePath }) => ({ file, relativePath })), ...(payload.relatesTo ? { relatesTo: payload.relatesTo } : {}), ...(payload.mentions ? { mentions: payload.mentions } : {}) }, writeConfirmations[activeSession.id])
  }
  const handleResend = async (message: import('./protocol/msgobj').MessageObject) => {
    if (!activeSession) throw Error('session_missing')
    await store.resend(context, activeSession.id, message, writeConfirmations[activeSession.id])
  }
  const openForward = (message: import('./protocol/msgobj').MessageObject) => {
    void dialog.open({ title: t('messagehub.forward.title'), size: 'sm', dismissible: false, renderBody: controls => <ForwardMessageForm context={context} message={message} confirmationFor={sessionId => writeConfirmations[sessionId]} onCancel={() => controls.close()} onDone={() => controls.close()} /> })
  }
  const openMessageDetails = (message: import('./protocol/msgobj').MessageObject) => {
    const id = messageObjId(message)
    if (!id) return
    setDetailMessageId(id)
    setDetailsTarget('message')
    if (!isDesktop) setMobileView('details')
  }
  const conversationProps = {
    onResend: handleResend,
    onOpenMessage: openMessageDetails,
    focusMessageId: activeSession && activeSession.id === initialSessionId ? linkedMessageId : null,
    onForward: openForward,
    context, access, title: activeSession ? store.title(context, activeSession) : '', onCreate: () => openCreate(), creationReason: createReason,
    defaultSessionError, onRetryDefaultSession: () => { if (selectedEntityId) handleSelectEntity(selectedEntityId) },
    historyStatus: activeSession ? store.historyStatus(context, activeSession.id) : 'ready' as const,
    hasOlder: activeSession ? store.hasOlder(context, activeSession.id) : false,
    onLoadOlder: () => activeSession ? store.loadOlder(context, activeSession.id) : Promise.resolve(false),
    onVisibleMessages: (recordIds: string[]) => { if (activeSession) void store.markRead(context, activeSession.id, recordIds) },
    admission: selectedEntityId ? store.admission(context, selectedEntityId) : null,
    onAdmission: (action: 'accept' | 'block') => selectedEntityId ? store.setAdmission(context, selectedEntityId, action) : Promise.resolve(),
    onOpenEntity: handleSelectEntity,
    onOpenSessionDetails: () => handleOpenDetails('session'), onOpenDetails: () => handleOpenDetails('entity'),
    draft: activeSession ? store.draft(context, activeSession.id) : '',
    draftAttachments: activeSession ? store.attachments(context, activeSession.id) : [],
    onAttachmentsChange: (attachments: import('./conversation/input/attachmentDraft').ComposerAttachmentInput[]) => activeSession ? store.saveAttachments(context, activeSession.id, attachments) : undefined,
    onDraftChange: (value: string) => { if (activeSession) return store.saveDraft(context, activeSession.id, value) },
    showActions: activeSession ? store.preferences(context, activeSession.id).showActions : true,
  }
  const detailsPane = detailsTarget === 'message' && activeSession && selectedEntity && detailMessageId ? <MessageDetails key={`${viewerSessionKey(context, activeSession.id)}:${detailMessageId}`} context={context} entityId={selectedEntity.id} sessionId={activeSession.id} messageId={detailMessageId} reader={messageReader} displayName={did => did === context.ownerDid ? t('messagehub.you') : store.findEntity(context, did)?.name ?? friendlyDidName(did, false)} onClose={handleCloseDetails} /> : detailsTarget === 'session' && activeSession && selectedEntity && access ? <SessionDetails key={viewerSessionKey(context, activeSession.id)} session={activeSession} entity={selectedEntity} context={context} access={access} showActions={conversationProps.showActions} onShowActions={async showActions => { await store.updatePreferences(context, activeSession.id, { showActions }) }} onClose={handleCloseDetails} onManage={() => openManage(activeSession)} onWrite={enabled => setWriteConfirmations(previous => ({ ...previous, [activeSession.id]: enabled ? JSON.stringify(activeSession.binding) : '' }))} /> : detailsTarget === 'entity' && entityDetail ? <EntityDetails entity={entityDetail} context={context} onClose={handleCloseDetails} onCreateGroup={canManage ? openCreateGroup : undefined} /> : null
  const entityListExtras = { hasMore: store.hasMoreEntities(context), onLoadMore: () => store.loadMoreEntities(context), onCreateGroup: canManage ? () => openCreateGroup() : undefined }
  const newSessionLabel = t('messagehub.newSession')
  const newSessionButton = <button type="button" disabled={!canManage} className={hubIconButtonClass} onClick={() => openCreate(null)} aria-label={newSessionLabel} title={newSessionLabel}><SquarePen size={17} /></button>
  const newGroupLabel = t('messagehub.group.create')
  const newGroupButton = <button type="button" disabled={!canManage} className={hubIconButtonClass} onClick={() => openCreateGroup()} aria-label={newGroupLabel} title={newGroupLabel} data-testid="new-group"><Users size={17} /></button>
  const conversationSpace = layoutWidth - (isEntityListCollapsed ? ENTITY_LIST_COLLAPSED_WIDTH : entityListWidth)
  const sessionSidebarInline = conversationSpace - sessionSidebarWidth >= CONVERSATION_MIN_READING_WIDTH
  const detailsInline = conversationSpace - (showSessionSidebar && sessionSidebarInline ? sessionSidebarWidth : 0) - DETAILS_PANEL_WIDTH >= CONVERSATION_MIN_READING_WIDTH
  const closeDesktopDrawer = (event: React.KeyboardEvent) => {
    if (event.key !== 'Escape' || event.defaultPrevented) return
    if (showDetails && !detailsInline) { event.preventDefault(); setDetailsTarget(null) }
    else if (showSessionSidebar && !sessionSidebarInline) { event.preventDefault(); setShowSessionSidebar(false) }
  }

  const desktopSessionSidebarPane = showSessionSidebar && !sessionSidebarInline ? (
    <>
      <div className="absolute inset-0 z-30" style={{ background: 'rgba(15, 23, 42, 0.18)' }} onClick={() => setShowSessionSidebar(false)} />
      <div
        className="absolute inset-y-0 left-0 z-40"
        style={{
          width: sessionSidebarWidth,
          maxWidth: '85%',
          borderRight: '1px solid var(--cp-border)',
          background: 'var(--cp-surface-opaque)',
          boxShadow: 'var(--cp-panel-shadow)',
        }}
      >
        <SessionSidebar
          {...sidebarProps}
          sessions={sessions}
          activeSessionId={activeSession?.id ?? null}
          onSelectSession={handleSelectSession}
          onClose={() => setShowSessionSidebar(false)}
          showHeader={false}
        />
      </div>
    </>
  ) : showSessionSidebar ? (
    <>
      <div
        ref={sessionSidebarPaneRef}
        className="h-full flex-shrink-0"
        style={{
          width: sessionSidebarWidth,
          minWidth: SESSION_SIDEBAR_MIN_WIDTH,
          maxWidth: SESSION_SIDEBAR_MAX_WIDTH,
          borderRight: '1px solid var(--cp-border)',
          background: 'var(--cp-surface)',
        }}
      >
        <SessionSidebar
          {...sidebarProps}
          sessions={sessions}
          activeSessionId={activeSession?.id ?? null}
          onSelectSession={handleSelectSession}
          onClose={() => setShowSessionSidebar(false)}
          showHeader={false}
        />
      </div>

      <PaneSplitter
        label={t('messagehub.resizeSessionList', 'Resize session list')}
        active={isResizingSessionSidebar}
        value={sessionSidebarWidth}
        min={SESSION_SIDEBAR_MIN_WIDTH}
        max={sessionSidebarDragMax}
        onPointerDown={sessionSidebarResizer.onPointerDown}
        onKeyDown={sessionSidebarResizer.onKeyDown}
        testId="session-sidebar-splitter"
      />
    </>
  ) : null

  if (!isDesktop) {
    return (
      <div className="relative h-full w-full" style={{ background: 'var(--cp-bg)', zIndex: 1 }}>
        {mobileView === 'entity-list' ? (
          <EntityList
            entities={entities}
            selectedEntityId={selectedEntityId}
            filter={filter}
            searchQuery={searchQuery}
            headerActions={<>{onHome ? <HomeButton onHome={onHome} /> : null}{newSessionButton}{newGroupButton}<MediaSettingsButton /></>}
            enableDrilldownNavigation
            useCompactInlineChildren
            childNavigationTrigger="icon"
            drilldownPath={entityListDrilldownPath}
            onDrilldownPathChange={setEntityListDrilldownPath}
            {...entityListExtras}
            onSelectEntity={handleSelectEntity}
            onFilterChange={setFilter}
            onSearchChange={setSearchQuery}
          />
        ) : null}

        {mobileView !== 'entity-list' && selectedEntity ? (
          <div className="relative h-full" inert={mobileView === 'details' && !!detailsPane}>
            <ConversationView
              {...conversationProps}
              key={activeSession ? viewerSessionKey(context, activeSession.id) : selectedEntityId}
              entity={selectedEntity}
              session={activeSession}
              messageReader={messageReader}
              selfDid={context.ownerDid}
              onBack={handleBack}
              onOpenSessionSidebar={() => setShowSessionSidebar(true)}
              onSendMessage={handleSendMessage}
              sessionCount={sessions.length}
            />

            {showSessionSidebar ? (
              <>
                <div
                  className="absolute inset-0 z-40"
                  style={{ background: 'rgba(0,0,0,0.3)' }}
                  onClick={() => setShowSessionSidebar(false)}
                />
                <div
                  className="absolute bottom-0 left-0 top-0 z-50"
                  style={{ width: 280 }}
                >
                  <SessionSidebar
          {...sidebarProps}
                    sessions={sessions}
                    activeSessionId={activeSession?.id ?? null}
                    onSelectSession={handleSelectSession}
                    onClose={() => setShowSessionSidebar(false)}
                  />
                </div>
              </>
            ) : null}
          </div>
        ) : null}

        {mobileView === 'details' && detailsPane ? (
          <div className="absolute inset-0 z-50 h-full">
            {detailsPane}
          </div>
        ) : null}
      </div>
    )
  }

  return (
    <div
      ref={desktopLayoutRef}
      className="relative flex h-full w-full"
      onKeyDown={closeDesktopDrawer}
      style={{
        background: 'var(--cp-bg)',
        zIndex: 1,
        cursor: isResizingEntityList || isResizingSessionSidebar ? 'col-resize' : 'default',
      }}
    >
      <div
        ref={entityListPaneRef}
        className="h-full flex-shrink-0"
        style={{
          width: isEntityListCollapsed ? ENTITY_LIST_COLLAPSED_WIDTH : entityListWidth,
          minWidth: isEntityListCollapsed ? ENTITY_LIST_COLLAPSED_WIDTH : ENTITY_LIST_MIN_WIDTH,
          maxWidth: isEntityListCollapsed ? ENTITY_LIST_COLLAPSED_WIDTH : ENTITY_LIST_MAX_WIDTH,
          borderRight: '1px solid var(--cp-border)',
          background: 'var(--cp-surface)',
          transition: isResizingEntityList ? 'none' : 'width 220ms var(--cp-ease-emphasis)',
        }}
      >
        {isEntityListCollapsed ? (
          <div
            className="flex h-full flex-col items-center gap-3 px-2 py-4"
            style={{
              background:
                'linear-gradient(180deg, color-mix(in srgb, var(--cp-surface) 96%, transparent), color-mix(in srgb, var(--cp-surface-2) 94%, transparent))',
            }}
          >
            <button
              type="button"
              onClick={handleExpandEntityList}
              className="flex h-10 w-10 items-center justify-center rounded-2xl"
              style={{
                color: 'var(--cp-accent)',
                background: 'color-mix(in srgb, var(--cp-accent) 12%, transparent)',
              }}
              aria-label={t('messagehub.expandEntityList', 'Expand entity list')}
              title={t('messagehub.expandEntityList', 'Expand entity list')}
            >
              <ChevronRight size={18} />
            </button>
            <div
              className="flex flex-1 items-center justify-center"
              style={{ color: 'var(--cp-muted)' }}
            >
              <span
                className="text-[11px] font-semibold uppercase tracking-[0.24em]"
                style={{ writingMode: 'vertical-rl', textOrientation: 'mixed' }}
              >
                {t('messagehub.entitiesShort', 'Entities')}
              </span>
            </div>
          </div>
        ) : (
          <EntityList
            entities={entities}
            selectedEntityId={selectedEntityId}
            filter={filter}
            searchQuery={searchQuery}
            enableDrilldownNavigation
            useCompactInlineChildren
            headerActions={(
              <>{onHome ? <HomeButton onHome={onHome} /> : null}{newSessionButton}{newGroupButton}<MediaSettingsButton /><button
                type="button"
                onClick={handleCollapseEntityList}
                className={hubIconButtonClass}
                aria-label={t('messagehub.collapseEntityList', 'Collapse entity list')}
                title={t('messagehub.collapseEntityList', 'Collapse entity list')}
              >
                <ChevronLeft size={18} />
              </button></>
            )}
            {...entityListExtras}
            onSelectEntity={handleSelectEntity}
            onFilterChange={setFilter}
            onSearchChange={setSearchQuery}
          />
        )}
      </div>

      <PaneSplitter
        label={t('messagehub.resizeEntityList', 'Resize entity list')}
        active={isResizingEntityList}
        disabled={isEntityListCollapsed}
        value={entityListWidth}
        min={ENTITY_LIST_MIN_WIDTH}
        max={ENTITY_LIST_MAX_WIDTH}
        onPointerDown={entityListResizer.onPointerDown}
        onKeyDown={entityListResizer.onKeyDown}
        testId="entity-list-splitter"
      />

      <div className="h-full min-w-0 flex-1">
        {selectedEntity ? (
          <ConversationView
              {...conversationProps}
              key={activeSession ? viewerSessionKey(context, activeSession.id) : selectedEntityId}
            entity={selectedEntity}
            session={activeSession}
            messageReader={messageReader}
            selfDid={context.ownerDid}
            onBack={handleBack}
            onOpenSessionSidebar={() => setShowSessionSidebar((prev) => !prev)}
            onSendMessage={handleSendMessage}
            sessionCount={sessions.length}
            leadingPane={desktopSessionSidebarPane}
            isSessionSidebarOpen={showSessionSidebar}
          />
        ) : (
          <EmptyConversation action={<div className="flex flex-wrap justify-center gap-2"><button className={`${hubButtonClass} flex items-center gap-2`} type="button" disabled={!canManage} onClick={() => openCreate(null)}><SquarePen size={16} />{newSessionLabel}</button><button className={`${hubButtonClass} flex items-center gap-2`} type="button" disabled={!canManage} onClick={() => openCreateGroup()}><Users size={16} />{newGroupLabel}</button></div>} />
        )}
      </div>

      {showDetails && entityDetail && detailsInline ? (
        <div
          className="h-full flex-shrink-0"
          style={{
            width: DETAILS_PANEL_WIDTH,
            borderLeft: '1px solid var(--cp-border)',
          }}
        >
          {detailsPane}
        </div>
      ) : null}
      {showDetails && entityDetail && !detailsInline ? (
        <>
          <div className="absolute inset-0 z-40" style={{ background: 'rgba(15, 23, 42, 0.18)' }} onClick={handleCloseDetails} />
          <div
            className="absolute inset-y-0 right-0 z-50"
            style={{
              width: DETAILS_PANEL_WIDTH,
              maxWidth: '90%',
              borderLeft: '1px solid var(--cp-border)',
              background: 'var(--cp-surface-opaque)',
              boxShadow: 'var(--cp-panel-shadow)',
            }}
          >
            {detailsPane}
          </div>
        </>
      ) : null}
    </div>
  )
}

function EmptyConversation({ action }: { action?: React.ReactNode }) {
  const { t } = useI18n()

  return (
    <div
      className="flex h-full flex-col items-center justify-center gap-3"
      style={{ color: 'var(--cp-muted)', background: 'var(--cp-message-canvas)' }}
    >
      <MessageSquare size={48} strokeWidth={1.2} />
      <p className="text-sm">
        {t('messagehub.selectConversation', 'Select a conversation to start')}
      </p>
      {action}
    </div>
  )
}

function MediaSettingsButton() {
  const { t } = useI18n()
  const host = useMessageMediaHost()
  if (!host) return null
  const label = t('messagehub.media.settings', 'Media settings')
  return (
    <button
      type="button"
      onClick={() => host.openSettings()}
      className={hubIconButtonClass}
      aria-label={label}
      title={label}
    >
      <ImagePlay size={17} />
    </button>
  )
}

/** The draggable vertical splitter next to a resizable pane (a `separator`, not a button, so clicking it does not steal focus). */
function PaneSplitter({ label, active, disabled = false, value, min, max, onPointerDown, onKeyDown, testId }: {
  label: string
  active: boolean
  disabled?: boolean
  value: number
  min: number
  max: number
  onPointerDown: (event: React.PointerEvent<HTMLElement>) => void
  onKeyDown: (event: React.KeyboardEvent<HTMLElement>) => void
  testId: string
}) {
  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={label}
      aria-valuenow={Math.round(value)}
      aria-valuemin={Math.round(min)}
      aria-valuemax={Math.round(max)}
      aria-disabled={disabled || undefined}
      aria-hidden={disabled || undefined}
      tabIndex={disabled ? -1 : 0}
      title={disabled ? undefined : label}
      className="group relative h-full flex-shrink-0 outline-none focus-visible:ring-2 focus-visible:ring-[color:var(--cp-accent)]"
      onPointerDown={onPointerDown}
      onKeyDown={onKeyDown}
      data-testid={testId}
      data-active={active || undefined}
      style={{
        width: PANEL_SPLITTER_WIDTH,
        marginLeft: -(PANEL_SPLITTER_WIDTH / 2),
        marginRight: -(PANEL_SPLITTER_WIDTH / 2),
        cursor: disabled ? 'default' : 'col-resize',
        pointerEvents: disabled ? 'none' : undefined,
        background: active ? 'color-mix(in srgb, var(--cp-accent) 8%, transparent)' : 'transparent',
        zIndex: 10,
        touchAction: 'none',
      }}
    >
      <span
        className="pointer-events-none absolute inset-y-0 left-1/2 -translate-x-1/2 rounded-full transition-all duration-150"
        style={{
          width: active ? 3 : 1,
          top: 18,
          bottom: 18,
          background: active ? 'var(--cp-accent)' : 'color-mix(in srgb, var(--cp-border) 92%, transparent)',
          boxShadow: active ? '0 0 0 4px color-mix(in srgb, var(--cp-accent) 12%, transparent)' : 'none',
        }}
      />
    </div>
  )
}
