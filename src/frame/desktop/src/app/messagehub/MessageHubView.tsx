import { useCallback, useEffect, useRef, useState } from 'react'
import { useMediaQuery } from '@mui/material'
import { ChevronLeft, ChevronRight, MessageSquare } from 'lucide-react'
import { useI18n } from '../../i18n/provider'
import { ConversationView } from './ConversationView'
import { InMemoryConversationMessageReader } from './conversation/history/data-source'
import type { ConversationComposerSubmitPayload } from './conversation/input/ConversationComposer'
import { EntityDetails } from './EntityDetails'
import { EntityList } from './EntityList'
import { resolveMessageHubContext, type MessageHubContextRequest } from './launch'
import { useMessageHubStore, useMessageHubReady, useMessageHubRuntime } from './store'
import { creationReason, viewerSessionKey } from './sessionModel'
import { WindowDialogProvider, useWindowDialog } from '../../desktop/windows/dialogs'
import { SessionDetails } from './SessionDetails'
import { CreateSessionForm, ManageSessionForm, hubButtonClass } from './SessionDialogs'
import type { MessageHubContext, Session } from './types'
import { SessionSidebar } from './SessionSidebar'
import {
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

export function MessageHubView({ initialEntityId = null, contextRequest }: { initialEntityId?: string | null; contextRequest?: MessageHubContextRequest | MessageHubContext }) {
  const { t } = useI18n()
  const [exitFrom, setExitFrom] = useState<string | null>(null)
  const { status, retry } = useMessageHubReady()
  const isDesktop = useMediaQuery('(min-width: 769px)')
  if (status !== 'ready') return <div className="flex h-full items-center justify-center gap-3"><p role="status">{t(status === 'loading' ? 'messagehub.loading' : 'messagehub.loadFailed')}</p>{status === 'error' && <button type="button" className={hubButtonClass} onClick={retry}>{t('messagehub.retry')}</button>}</div>
  return <MessageHubOwnerGate initialEntityId={initialEntityId} contextRequest={contextRequest} exitFrom={exitFrom} onExit={setExitFrom} isDesktop={isDesktop} />
}

function MessageHubOwnerGate({ initialEntityId, contextRequest, exitFrom, onExit, isDesktop }: { initialEntityId: string | null; contextRequest?: MessageHubContextRequest | MessageHubContext; exitFrom: string | null; onExit: (value: string | null) => void; isDesktop: boolean }) {
  const { t } = useI18n()
  const store = useMessageHubStore()
  const requested = resolveMessageHubContext(store.defaultContext(), contextRequest)
  const context = exitFrom === JSON.stringify(requested) ? store.defaultContext() : requested
  const ownerStatus = store.ownerStatus(context)
  useEffect(() => { if (store.canView(context)) void store.ensureOwner(context) }, [store, context.ownerDid, context.mode, context.viewerDid]) // eslint-disable-line react-hooks/exhaustive-deps
  if (!store.canView(context)) return <div role="alert" className="flex h-full flex-col items-center justify-center gap-3 p-6"><p>{t('messagehub.reason.permission_denied')}</p>{context.mode === 'observe' && <button type="button" className={hubButtonClass} onClick={() => onExit(JSON.stringify(requested))}>{t('messagehub.exitObserver')}</button>}</div>
  if (ownerStatus.phase === 'loading' || ownerStatus.phase === 'idle') return <div className="flex h-full items-center justify-center"><p role="status">{t('messagehub.loading')}</p></div>
  if (ownerStatus.phase === 'error') return <div role="alert" className="flex h-full flex-col items-center justify-center gap-3 p-6"><p>{t('messagehub.loadFailed')}</p><p className="max-w-md break-words text-xs text-[color:var(--cp-muted)]">{ownerStatus.message}</p><button type="button" className={hubButtonClass} onClick={() => void store.ensureOwner(context, true)}>{t('messagehub.retry')}</button></div>
  return <div className="relative flex h-full min-h-0 flex-col text-[color:var(--cp-text)]">
    {context.mode === 'observe' && <div className="flex shrink-0 items-center justify-between gap-2 border-b border-[color:var(--cp-border)] p-2 text-xs" data-testid="owner-banner"><span>{t('messagehub.observing')} · {context.ownerDid.split(':').at(-1)} · {t('messagehub.readOnly')}</span><button type="button" className={hubButtonClass} onClick={() => onExit(JSON.stringify(requested))}>{t('messagehub.exitObserver')}</button></div>}
    <div className="relative min-h-0 flex-1"><WindowDialogProvider key={JSON.stringify(context)} surface={isDesktop ? 'desktop' : 'mobile'} permissions={{ fullscreen: false }}><MessageHubContent key={`${JSON.stringify(context)}:${initialEntityId}`} initialEntityId={initialEntityId} context={context} /></WindowDialogProvider></div>
  </div>
}

function MessageHubContent({ initialEntityId, context }: { initialEntityId: string | null; context: MessageHubContext }) {
  const { t } = useI18n()
  const isDesktop = useMediaQuery('(min-width: 769px)')
  const store = useMessageHubStore()
  const dialog = useWindowDialog()
  useMessageHubRuntime()
  // Without an explicit entity the most recent one opens (pinned first), which
  // is what the mock route did with the CodeAssistant seed.
  const resolvedInitialEntityId = initialEntityId ? store.findEntity(context, initialEntityId)?.id ?? null : store.entities(context)[0]?.id ?? null
  const getDefaultSessionId = (entityId: string | null) => entityId ? store.sessions(context, entityId, 'active')[0]?.id ?? null : null
  const contextEpoch = useRef(0)
  const ownerDid = context.ownerDid
  useEffect(() => () => { contextEpoch.current++; store.clearTransient(ownerDid) }, [ownerDid, store])
  const [archived, setArchived] = useState(false)
  const [writeConfirmations, setWriteConfirmations] = useState<Record<string, string>>({})

  const [selectedEntityId, setSelectedEntityId] = useState<string | null>(resolvedInitialEntityId)
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(
    () => getDefaultSessionId(resolvedInitialEntityId),
  )
  const [filter, setFilter] = useState<EntityFilter>('all')
  const [searchQuery, setSearchQuery] = useState('')
  const [mobileView, setMobileView] = useState<MobileView>(
    () => (!isDesktop && resolvedInitialEntityId ? 'conversation' : 'entity-list'),
  )
  const [showSessionSidebar, setShowSessionSidebar] = useState(false)
  const [detailsTarget, setDetailsTarget] = useState<'entity' | 'session' | null>(null)
  const showDetails = detailsTarget !== null
  const [entityListDrilldownPath, setEntityListDrilldownPath] = useState<string[]>([])
  const [entityListWidth, setEntityListWidth] = useState(ENTITY_LIST_DEFAULT_WIDTH)
  const [sessionSidebarWidth, setSessionSidebarWidth] = useState(SESSION_SIDEBAR_DEFAULT_WIDTH)
  const [isEntityListCollapsed, setIsEntityListCollapsed] = useState(false)
  const [isResizingEntityList, setIsResizingEntityList] = useState(false)
  const [isResizingSessionSidebar, setIsResizingSessionSidebar] = useState(false)
  const desktopLayoutRef = useRef<HTMLDivElement>(null)
  const entityListWidthRef = useRef(ENTITY_LIST_DEFAULT_WIDTH)
  const sessionSidebarWidthRef = useRef(SESSION_SIDEBAR_DEFAULT_WIDTH)
  const entityListResizeRef = useRef<{
    pointerId: number
    startX: number
    startWidth: number
  } | null>(null)
  const sessionSidebarResizeRef = useRef<{
    pointerId: number
    startX: number
    startWidth: number
  } | null>(null)

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

  useEffect(() => {
    const element = desktopLayoutRef.current

    if (!isDesktop || !element) {
      return
    }

    const resizeObserver = new ResizeObserver(() => {
      setEntityListWidth((prev) => clampEntityListWidth(prev))
      setSessionSidebarWidth((prev) => clampSessionSidebarWidth(prev))
    })

    resizeObserver.observe(element)

    return () => {
      resizeObserver.disconnect()
    }
  }, [clampEntityListWidth, clampSessionSidebarWidth, isDesktop])

  const entities = store.entities(context)
  const findProjectedEntity = (id: string | null) => {
    const queue = [...entities]
    while (queue.length) { const item = queue.shift()!; if (item.id === id) return item; queue.push(...(item.children ?? [])) }
    return null
  }
  const selectedEntity = findProjectedEntity(selectedEntityId)
  const sessions = store.sessions(context, selectedEntityId ?? '', archived ? 'archived' : 'active')
  const activeSession = sessions.find(session => session.id === selectedSessionId) ?? sessions[0] ?? null
  const messageReader = activeSession ? store.reader(context, activeSession.id) : EMPTY_READER
  const entityDetail = selectedEntity ? store.entityDetail(context, selectedEntity.id) : null
  const confirmed = !!activeSession && writeConfirmations[activeSession.id] === JSON.stringify(activeSession.binding)
  const access = activeSession ? store.access(context, activeSession, confirmed) : null
  const canManage = context.mode === 'self' && context.viewerDid === context.ownerDid
  const activeSessionId = activeSession?.id ?? null
  useEffect(() => store.startSync(context, activeSessionId), [store, context.ownerDid, context.mode, context.viewerDid, activeSessionId]) // eslint-disable-line react-hooks/exhaustive-deps
  const createReason = selectedEntity ? (() => {
    const choices = store.connections(context, selectedEntity.id)
    return choices.some(choice => !creationReason(context, selectedEntity, store.policy(context, selectedEntity.id), choice.binding)) ? undefined : creationReason(context, selectedEntity, store.policy(context, selectedEntity.id), choices[0]?.binding)
  })() : undefined
  const openCreate = (entityId: string | null = selectedEntityId) => {
    const epoch = ++contextEpoch.current, trigger = document.activeElement
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
      if (session.id === activeSession?.id) { setSelectedSessionId(store.sessions(context, session.entityId, 'active')[0]?.id ?? null); setArchived(false); setDetailsTarget(null); setMobileView('conversation') }
    }} /> }).then(() => { if (trigger instanceof HTMLElement && trigger.isConnected) trigger.focus() })
  }
  const sidebarProps = {
    onCreate: () => openCreate(), onManage: openManage, onToggleArchived: () => { setArchived(value => !value); setSelectedSessionId(null); setDetailsTarget(null) }, archived,
    archivedCount: selectedEntityId ? store.sessions(context, selectedEntityId, 'archived').length : 0, canManage, creationReason: createReason, titleFor: (session: Session) => store.title(context, session),
    statusFor: (session: Session) => store.runtimeFor(context, session.id).map(state => t(`messagehub.runtime.${state.status}`)).join(' · '),
  }

  const handleSelectEntity = (
    (id: string) => {
      contextEpoch.current++; setArchived(false)
      setSelectedEntityId(id)
      setSelectedSessionId(getDefaultSessionId(id))
      setDetailsTarget(null)
      setShowSessionSidebar(false)

      if (!isDesktop) {
        setMobileView('conversation')
      }
    }
  )

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
    contextEpoch.current++; setSelectedSessionId(id)
    if (!isDesktop) setShowSessionSidebar(false)
  }

  const handleCollapseEntityList = useCallback(() => {
    setIsEntityListCollapsed(true)
    setIsResizingEntityList(false)
    entityListResizeRef.current = null
  }, [])

  const handleExpandEntityList = useCallback(() => {
    setIsEntityListCollapsed(false)
    setEntityListWidth(clampEntityListWidth(entityListWidthRef.current))
  }, [clampEntityListWidth])

  const handleEntityListSplitterPointerDown = useCallback((event: React.PointerEvent<HTMLButtonElement>) => {
    if (isEntityListCollapsed) {
      return
    }

    entityListResizeRef.current = {
      pointerId: event.pointerId,
      startX: event.clientX,
      startWidth: entityListWidthRef.current,
    }
    setIsResizingEntityList(true)
    event.currentTarget.setPointerCapture(event.pointerId)
    event.preventDefault()
  }, [isEntityListCollapsed])

  const handleEntityListSplitterPointerMove = useCallback((event: React.PointerEvent<HTMLButtonElement>) => {
    if (
      !entityListResizeRef.current ||
      entityListResizeRef.current.pointerId !== event.pointerId
    ) {
      return
    }

    const deltaX = event.clientX - entityListResizeRef.current.startX
    const nextWidth = clampEntityListWidth(entityListResizeRef.current.startWidth + deltaX)
    entityListWidthRef.current = nextWidth
    setEntityListWidth(nextWidth)
  }, [clampEntityListWidth])

  const handleEntityListSplitterPointerUp = useCallback((event: React.PointerEvent<HTMLButtonElement>) => {
    if (
      !entityListResizeRef.current ||
      entityListResizeRef.current.pointerId !== event.pointerId
    ) {
      return
    }

    entityListResizeRef.current = null
    setIsResizingEntityList(false)
    event.currentTarget.releasePointerCapture(event.pointerId)
  }, [])

  const handleSessionSidebarSplitterPointerDown = useCallback((event: React.PointerEvent<HTMLButtonElement>) => {
    sessionSidebarResizeRef.current = {
      pointerId: event.pointerId,
      startX: event.clientX,
      startWidth: sessionSidebarWidthRef.current,
    }
    setIsResizingSessionSidebar(true)
    event.currentTarget.setPointerCapture(event.pointerId)
    event.preventDefault()
  }, [])

  const handleSessionSidebarSplitterPointerMove = useCallback((event: React.PointerEvent<HTMLButtonElement>) => {
    if (
      !sessionSidebarResizeRef.current ||
      sessionSidebarResizeRef.current.pointerId !== event.pointerId
    ) {
      return
    }

    const deltaX = event.clientX - sessionSidebarResizeRef.current.startX
    const nextWidth = clampSessionSidebarWidth(sessionSidebarResizeRef.current.startWidth + deltaX)
    sessionSidebarWidthRef.current = nextWidth
    setSessionSidebarWidth(nextWidth)
  }, [clampSessionSidebarWidth])

  const handleSessionSidebarSplitterPointerUp = useCallback((event: React.PointerEvent<HTMLButtonElement>) => {
    if (
      !sessionSidebarResizeRef.current ||
      sessionSidebarResizeRef.current.pointerId !== event.pointerId
    ) {
      return
    }

    sessionSidebarResizeRef.current = null
    setIsResizingSessionSidebar(false)
    event.currentTarget.releasePointerCapture(event.pointerId)
  }, [])

  const handleSendMessage = async (payload: ConversationComposerSubmitPayload) => {
    if (!activeSession || !selectedEntityId) throw Error('session_missing')
    await store.send(context, activeSession.id, { content: payload.content, attachments: payload.attachments.map(({ file, relativePath }) => ({ file, relativePath })) }, writeConfirmations[activeSession.id])
  }
  const conversationProps = {
    context, access, title: activeSession ? store.title(context, activeSession) : '', onCreate: () => openCreate(), creationReason: createReason,
    historyStatus: activeSession ? store.historyStatus(context, activeSession.id) : 'ready' as const,
    hasOlder: activeSession ? store.hasOlder(context, activeSession.id) : false,
    onLoadOlder: () => activeSession ? store.loadOlder(context, activeSession.id) : Promise.resolve(false),
    onVisibleMessages: (recordIds: string[]) => { if (activeSession) void store.markRead(context, activeSession.id, recordIds) },
    admission: selectedEntityId ? store.admission(context, selectedEntityId) : null,
    onAdmission: (action: 'accept' | 'block') => selectedEntityId ? store.setAdmission(context, selectedEntityId, action) : Promise.resolve(),
    onOpenSessionDetails: () => handleOpenDetails('session'), onOpenDetails: () => handleOpenDetails('entity'),
    draft: activeSession ? store.draft(context, activeSession.id) : '',
    draftAttachments: activeSession ? store.attachments(context, activeSession.id) : [],
    onAttachmentsChange: (attachments: import('./conversation/input/attachmentDraft').ComposerAttachmentInput[]) => activeSession ? store.saveAttachments(context, activeSession.id, attachments) : undefined,
    onDraftChange: (value: string) => { if (activeSession) return store.saveDraft(context, activeSession.id, value) },
    showActions: activeSession ? store.preferences(context, activeSession.id).showActions : true,
    onShowActions: async (showActions: boolean) => { if (activeSession) await store.updatePreferences(context, activeSession.id, { showActions }) },
  }
  const detailsPane = detailsTarget === 'session' && activeSession && selectedEntity && access ? <SessionDetails key={viewerSessionKey(context, activeSession.id)} session={activeSession} entity={selectedEntity} context={context} access={access} onClose={handleCloseDetails} onManage={() => openManage(activeSession)} onWrite={enabled => setWriteConfirmations(previous => ({ ...previous, [activeSession.id]: enabled ? JSON.stringify(activeSession.binding) : '' }))} /> : detailsTarget === 'entity' && entityDetail ? <EntityDetails entity={entityDetail} context={context} onClose={handleCloseDetails} /> : null
  const entityListExtras = { hasMore: store.hasMoreEntities(context), onLoadMore: () => store.loadMoreEntities(context) }

  const desktopSessionSidebarPane = showSessionSidebar ? (
    <>
      <div
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

      <button
        type="button"
        className="group relative h-full flex-shrink-0"
        onPointerDown={handleSessionSidebarSplitterPointerDown}
        onPointerMove={handleSessionSidebarSplitterPointerMove}
        onPointerUp={handleSessionSidebarSplitterPointerUp}
        onPointerCancel={handleSessionSidebarSplitterPointerUp}
        title={t('messagehub.resizeSessionList', 'Resize session list')}
        style={{
          width: PANEL_SPLITTER_WIDTH,
          marginLeft: -(PANEL_SPLITTER_WIDTH / 2),
          marginRight: -(PANEL_SPLITTER_WIDTH / 2),
          cursor: 'col-resize',
          background: isResizingSessionSidebar
            ? 'color-mix(in srgb, var(--cp-accent) 8%, transparent)'
            : 'transparent',
          zIndex: 10,
          touchAction: 'none',
        }}
      >
        <span
          className="pointer-events-none absolute inset-y-0 left-1/2 -translate-x-1/2 rounded-full transition-all duration-150"
          style={{
            width: isResizingSessionSidebar ? 3 : 1,
            top: 18,
            bottom: 18,
            background: isResizingSessionSidebar
              ? 'var(--cp-accent)'
              : 'color-mix(in srgb, var(--cp-border) 92%, transparent)',
            boxShadow: isResizingSessionSidebar
              ? '0 0 0 4px color-mix(in srgb, var(--cp-accent) 12%, transparent)'
              : 'none',
          }}
        />
      </button>
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
            headerActions={<button type="button" disabled={!canManage} className={hubButtonClass} onClick={() => openCreate(null)}>{t('messagehub.newSession')}</button>}
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
      className="flex h-full w-full"
      style={{
        background: 'var(--cp-bg)',
        zIndex: 1,
        cursor: isResizingEntityList || isResizingSessionSidebar ? 'col-resize' : 'default',
      }}
    >
      <div
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
              <><button type="button" disabled={!canManage} className={hubButtonClass} onClick={() => openCreate(null)}>{t('messagehub.newSession')}</button><button
                type="button"
                onClick={handleCollapseEntityList}
                className="flex h-9 w-9 items-center justify-center rounded-xl"
                style={{
                  color: 'var(--cp-muted)',
                  background: 'color-mix(in srgb, var(--cp-text) 7%, transparent)',
                }}
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

      <button
        type="button"
        disabled={isEntityListCollapsed}
        className="group relative h-full flex-shrink-0"
        onPointerDown={handleEntityListSplitterPointerDown}
        onPointerMove={handleEntityListSplitterPointerMove}
        onPointerUp={handleEntityListSplitterPointerUp}
        onPointerCancel={handleEntityListSplitterPointerUp}
        aria-hidden={isEntityListCollapsed}
        tabIndex={isEntityListCollapsed ? -1 : 0}
        title={t('messagehub.resizeEntityList', 'Resize entity list')}
        style={{
          width: PANEL_SPLITTER_WIDTH,
          marginLeft: -(PANEL_SPLITTER_WIDTH / 2),
          marginRight: -(PANEL_SPLITTER_WIDTH / 2),
          cursor: isEntityListCollapsed ? 'default' : 'col-resize',
          background: isResizingEntityList
            ? 'color-mix(in srgb, var(--cp-accent) 8%, transparent)'
            : 'transparent',
          zIndex: 10,
          touchAction: 'none',
        }}
      >
        <span
          className="pointer-events-none absolute inset-y-0 left-1/2 -translate-x-1/2 rounded-full transition-all duration-150"
          style={{
            width: isResizingEntityList ? 3 : 1,
            top: 18,
            bottom: 18,
            background: isResizingEntityList
              ? 'var(--cp-accent)'
              : 'color-mix(in srgb, var(--cp-border) 92%, transparent)',
            boxShadow: isResizingEntityList
              ? '0 0 0 4px color-mix(in srgb, var(--cp-accent) 12%, transparent)'
              : 'none',
          }}
        />
      </button>

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
          <div className="h-full"><EmptyConversation /><button className={hubButtonClass} type="button" disabled={!canManage} onClick={() => openCreate(null)}>{t('messagehub.newSession')}</button></div>
        )}
      </div>

      {showDetails && entityDetail ? (
        <div
          className="h-full flex-shrink-0"
          style={{
            width: 320,
            borderLeft: '1px solid var(--cp-border)',
          }}
        >
          {detailsPane}
        </div>
      ) : null}
    </div>
  )
}

function EmptyConversation() {
  const { t } = useI18n()

  return (
    <div
      className="flex h-full flex-col items-center justify-center gap-3"
      style={{ color: 'var(--cp-muted)' }}
    >
      <MessageSquare size={48} strokeWidth={1.2} />
      <p className="text-sm">
        {t('messagehub.selectConversation', 'Select a conversation to start')}
      </p>
    </div>
  )
}
