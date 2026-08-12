import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { ConversationView } from '../messagehub/ConversationView'
import { InMemoryConversationMessageReader } from '../messagehub/conversation/history/data-source'
import type { AppendableConversationMessageReader } from '../messagehub/conversation/history/types'
import type { ConversationComposerSubmitPayload } from '../messagehub/conversation/input/ConversationComposer'
import { EntityDetails } from '../messagehub/EntityDetails'
import {
  createOutgoingMockMessage,
  MOCK_SELF_DID,
  mockEntities,
  mockEntityDetails,
  mockSessions,
} from '../messagehub/mock/data'
import { SessionSidebar } from '../messagehub/SessionSidebar'
import {
  PANEL_SPLITTER_WIDTH,
  SESSION_SIDEBAR_DEFAULT_WIDTH,
  SESSION_SIDEBAR_MAX_WIDTH,
  SESSION_SIDEBAR_MIN_WIDTH,
} from '../messagehub/layout'
import type { AppContentLoaderProps } from '../types'
import { createCodeAssistantMockReaders } from './mockHistory'

const codeAssistantEntityId = 'agent-coder'
const EMPTY_READER = InMemoryConversationMessageReader.empty()

export function CodeAssistantAppPanel(props: AppContentLoaderProps) {
  void props
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(
    () => mockSessions[codeAssistantEntityId]?.[0]?.id ?? null,
  )
  const [showSessionSidebar, setShowSessionSidebar] = useState(false)
  const [showDetails, setShowDetails] = useState(false)
  const [sessionSidebarWidth, setSessionSidebarWidth] = useState(SESSION_SIDEBAR_DEFAULT_WIDTH)
  const [isResizingSessionSidebar, setIsResizingSessionSidebar] = useState(false)
  const [localReaders, setLocalReaders] = useState<Record<string, AppendableConversationMessageReader>>(
    {},
  )
  const panelRef = useRef<HTMLDivElement>(null)
  const sessionSidebarWidthRef = useRef(SESSION_SIDEBAR_DEFAULT_WIDTH)
  const sessionSidebarResizeRef = useRef<{
    pointerId: number
    startX: number
    startWidth: number
  } | null>(null)

  const clampSessionSidebarWidth = useCallback((width: number) => (
    Math.min(Math.max(width, SESSION_SIDEBAR_MIN_WIDTH), SESSION_SIDEBAR_MAX_WIDTH)
  ), [])

  useEffect(() => {
    let cancelled = false

    void createCodeAssistantMockReaders().then((readers) => {
      if (!cancelled) {
        setLocalReaders(readers)
      }
    })

    return () => {
      cancelled = true
    }
  }, [])

  useEffect(() => {
    sessionSidebarWidthRef.current = sessionSidebarWidth
  }, [sessionSidebarWidth])

  useEffect(() => {
    const element = panelRef.current

    if (!element) {
      return
    }

    const resizeObserver = new ResizeObserver(() => {
      setSessionSidebarWidth((prev) => clampSessionSidebarWidth(prev))
    })

    resizeObserver.observe(element)

    return () => {
      resizeObserver.disconnect()
    }
  }, [clampSessionSidebarWidth])

  const entity = useMemo(
    () => mockEntities.find((item) => item.id === codeAssistantEntityId) ?? null,
    [],
  )
  const sessions = useMemo(
    () => mockSessions[codeAssistantEntityId] ?? [],
    [],
  )
  const activeSession = useMemo(() => {
    if (!selectedSessionId) {
      return sessions[0] ?? null
    }

    return sessions.find((session) => session.id === selectedSessionId) ?? sessions[0] ?? null
  }, [selectedSessionId, sessions])
  const messageReader = useMemo(() => {
    const sessionId = activeSession?.id
    return sessionId ? localReaders[sessionId] ?? EMPTY_READER : EMPTY_READER
  }, [activeSession, localReaders])
  const entityDetail = useMemo(
    () => mockEntityDetails[codeAssistantEntityId] ?? null,
    [],
  )

  const handleSendMessage = useCallback((payload: ConversationComposerSubmitPayload) => {
    if (!activeSession) {
      return
    }

    const newMessage = createOutgoingMockMessage({
      sessionId: activeSession.id,
      entityId: codeAssistantEntityId,
      content: buildOutgoingDraftContent(payload),
      createdAtMs: Date.now(),
    })

    setLocalReaders((prev) => ({
      ...prev,
      [activeSession.id]: (prev[activeSession.id] ?? EMPTY_READER).append(newMessage),
    }))
  }, [activeSession])

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
          onSelectSession={setSelectedSessionId}
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
        title="Resize session list"
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

  if (!entity || !entityDetail) {
    return null
  }

  return (
    <div
      ref={panelRef}
      className="flex h-full min-h-0 bg-[color:var(--cp-bg)]"
      style={{ cursor: isResizingSessionSidebar ? 'col-resize' : 'default' }}
    >
      <div className="min-w-0 flex-1">
        <ConversationView
          entity={entity}
          session={activeSession}
          messageReader={messageReader}
          selfDid={MOCK_SELF_DID}
          onBack={() => undefined}
          onOpenSessionSidebar={() => setShowSessionSidebar((prev) => !prev)}
          onOpenDetails={() => setShowDetails((prev) => !prev)}
          onSendMessage={handleSendMessage}
          sessionCount={sessions.length}
          leadingPane={desktopSessionSidebarPane}
          isSessionSidebarOpen={showSessionSidebar && sessions.length > 1}
        />
      </div>

      {showDetails ? (
        <div
          className="h-full w-[320px] flex-shrink-0"
          style={{ borderLeft: '1px solid var(--cp-border)' }}
        >
          <EntityDetails
            entity={entityDetail}
            onClose={() => setShowDetails(false)}
          />
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
