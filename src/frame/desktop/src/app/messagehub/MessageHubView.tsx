import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useMediaQuery } from '@mui/material'
import { ChevronLeft, ChevronRight, MessageSquare } from 'lucide-react'
import { useI18n } from '../../i18n/provider'
import { ConversationView } from './ConversationView'
import { InMemoryConversationMessageReader } from './conversation/history/data-source'
import type { AppendableConversationMessageReader } from './conversation/history/types'
import type { ConversationComposerSubmitPayload } from './conversation/input/ConversationComposer'
import { EntityDetails } from './EntityDetails'
import { EntityList } from './EntityList'
import {
  createOutgoingMockMessage,
  MOCK_SELF_DID,
  mockEntities,
  mockEntityDetails,
  mockMessageReaders,
  mockSessions,
} from './mock/data'
import { SessionSidebar } from './SessionSidebar'
import { createCodeAssistantMockReaders } from '../codeassistant/mockHistory'
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

function findEntityById(id: string | null) {
  if (!id) {
    return null
  }

  const queue = [...mockEntities]

  while (queue.length > 0) {
    const current = queue.shift()

    if (!current) {
      continue
    }

    if (current.id === id) {
      return current
    }

    if (current.children?.length) {
      queue.push(...current.children)
    }
  }

  return null
}

function getDefaultSessionId(entityId: string | null) {
  return entityId ? mockSessions[entityId]?.[0]?.id ?? null : null
}

export function MessageHubView({
  initialEntityId = null,
}: {
  initialEntityId?: string | null
}) {
  const { t } = useI18n()
  const isDesktop = useMediaQuery('(min-width: 769px)')
  const resolvedInitialEntityId = findEntityById(initialEntityId)?.id ?? null

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
  const [showDetails, setShowDetails] = useState(false)
  const [entityListDrilldownPath, setEntityListDrilldownPath] = useState<string[]>([])
  const [entityListWidth, setEntityListWidth] = useState(ENTITY_LIST_DEFAULT_WIDTH)
  const [sessionSidebarWidth, setSessionSidebarWidth] = useState(SESSION_SIDEBAR_DEFAULT_WIDTH)
  const [isEntityListCollapsed, setIsEntityListCollapsed] = useState(false)
  const [isResizingEntityList, setIsResizingEntityList] = useState(false)
  const [isResizingSessionSidebar, setIsResizingSessionSidebar] = useState(false)
  const [localReaders, setLocalReaders] = useState<Record<string, AppendableConversationMessageReader>>(
    () => ({ ...mockMessageReaders }),
  )
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
    let cancelled = false

    void createCodeAssistantMockReaders().then((readers) => {
      if (!cancelled) {
        setLocalReaders((prev) => ({
          ...prev,
          ...readers,
        }))
      }
    })

    return () => {
      cancelled = true
    }
  }, [])

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

  const selectedEntity = useMemo(
    () => findEntityById(selectedEntityId),
    [selectedEntityId],
  )

  const sessions = useMemo(
    () => (selectedEntityId ? mockSessions[selectedEntityId] ?? [] : []),
    [selectedEntityId],
  )

  const activeSession = useMemo(() => {
    if (selectedSessionId) {
      return sessions.find((session) => session.id === selectedSessionId) ?? null
    }

    return sessions[0] ?? null
  }, [selectedSessionId, sessions])

  const messageReader = useMemo(() => {
    const sessionId = activeSession?.id
    return sessionId ? localReaders[sessionId] ?? EMPTY_READER : EMPTY_READER
  }, [activeSession, localReaders])

  const entityDetail = useMemo(
    () => (selectedEntityId ? mockEntityDetails[selectedEntityId] ?? null : null),
    [selectedEntityId],
  )

  const handleSelectEntity = useCallback(
    (id: string) => {
      setSelectedEntityId(id)
      setSelectedSessionId(getDefaultSessionId(id))
      setShowDetails(false)
      setShowSessionSidebar(false)

      if (!isDesktop) {
        setMobileView('conversation')
      }
    },
    [isDesktop],
  )

  const handleBack = useCallback(() => {
    setMobileView('entity-list')
    setShowDetails(false)
    setShowSessionSidebar(false)
  }, [])

  const handleOpenDetails = useCallback(() => {
    if (isDesktop) {
      setShowDetails((prev) => !prev)
      return
    }

    setMobileView('details')
  }, [isDesktop])

  const handleCloseDetails = useCallback(() => {
    if (isDesktop) {
      setShowDetails(false)
      return
    }

    setMobileView('conversation')
  }, [isDesktop])

  const handleSelectSession = useCallback((id: string) => {
    setSelectedSessionId(id)
  }, [])

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

  const handleSendMessage = useCallback((payload: ConversationComposerSubmitPayload) => {
    if (!activeSession || !selectedEntityId) {
      return
    }

    const content = buildOutgoingDraftContent(payload)
    const newMessage = createOutgoingMockMessage({
      sessionId: activeSession.id,
      entityId: selectedEntityId,
      content,
      createdAtMs: Date.now(),
    })

    setLocalReaders((prev) => ({
      ...prev,
      [activeSession.id]: (prev[activeSession.id] ?? EMPTY_READER).append(newMessage),
    }))
  }, [activeSession, selectedEntityId])

  const desktopSessionSidebarPane = showSessionSidebar && sessions.length > 1 ? (
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
            entities={mockEntities}
            selectedEntityId={selectedEntityId}
            filter={filter}
            searchQuery={searchQuery}
            enableDrilldownNavigation
            useCompactInlineChildren
            childNavigationTrigger="icon"
            drilldownPath={entityListDrilldownPath}
            onDrilldownPathChange={setEntityListDrilldownPath}
            onSelectEntity={handleSelectEntity}
            onFilterChange={setFilter}
            onSearchChange={setSearchQuery}
          />
        ) : null}

        {mobileView === 'conversation' && selectedEntity ? (
          <div className="relative h-full">
            <ConversationView
              entity={selectedEntity}
              session={activeSession}
              messageReader={messageReader}
              selfDid={MOCK_SELF_DID}
              onBack={handleBack}
              onOpenSessionSidebar={() => setShowSessionSidebar(true)}
              onOpenDetails={handleOpenDetails}
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

        {mobileView === 'details' && entityDetail ? (
          <div className="h-full">
            <EntityDetails entity={entityDetail} onClose={handleCloseDetails} />
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
            entities={mockEntities}
            selectedEntityId={selectedEntityId}
            filter={filter}
            searchQuery={searchQuery}
            enableDrilldownNavigation
            useCompactInlineChildren
            headerActions={(
              <button
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
              </button>
            )}
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
            entity={selectedEntity}
            session={activeSession}
            messageReader={messageReader}
            selfDid={MOCK_SELF_DID}
            onBack={handleBack}
            onOpenSessionSidebar={() => setShowSessionSidebar((prev) => !prev)}
            onOpenDetails={handleOpenDetails}
            onSendMessage={handleSendMessage}
            sessionCount={sessions.length}
            leadingPane={desktopSessionSidebarPane}
            isSessionSidebarOpen={showSessionSidebar && sessions.length > 1}
          />
        ) : (
          <EmptyConversation />
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
          <EntityDetails entity={entityDetail} onClose={handleCloseDetails} />
        </div>
      ) : null}
    </div>
  )
}

function buildOutgoingDraftContent({
  attachments,
  content,
}: ConversationComposerSubmitPayload): string {
  const textContent = content.trim()

  if (attachments.length === 0) {
    return textContent
  }

  const names = attachments.map((attachment) => (
    attachment.relativePath || attachment.file.name
  ))
  const visibleNames = names.slice(0, 3).join(', ')
  const remainingCount = names.length - 3
  const attachmentLine = remainingCount > 0
    ? `[Mock attachments] ${attachments.length} items: ${visibleNames}, +${remainingCount} more`
    : `[Mock attachments] ${attachments.length} items: ${visibleNames}`

  if (!textContent) {
    return attachmentLine
  }

  return `${textContent}\n\n${attachmentLine}`
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
