import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Menu } from 'lucide-react'
import { useI18n } from '../../i18n/provider'
import { ConversationSurface } from '../../components/conversation/ConversationSurface'
import type { ConversationSource } from '../../components/conversation/source'
import type { OutgoingPayload } from '../../components/conversation/model/outgoing'
import { friendlyDidName } from '../messagehub/api/projection'
import { InMemoryConversationMessageReader } from '../../components/conversation/history/data-source'
import type { AppendableConversationMessageReader } from '../../components/conversation/history/types'
import type { MessageObject } from '../../components/conversation/model/msgobj'
import { EntityDetails } from '../messagehub/EntityDetails'
import { createOutgoingMockMessage, mockEntities, mockEntityDetails, mockSessions } from '../messagehub/mock/data'
import { MOCK_SELF_DID } from '../../components/conversation/mock/ids'
import { SessionSidebar } from '../messagehub/SessionSidebar'
import {
  PANEL_SPLITTER_WIDTH,
  SESSION_SIDEBAR_DEFAULT_WIDTH,
  SESSION_SIDEBAR_MAX_WIDTH,
  SESSION_SIDEBAR_MIN_WIDTH,
} from '../messagehub/layout'
import type { AppContentLoaderProps } from '../types'
import { createCodeAssistantMockReaders } from './mockHistory'

const codeAssistantEntityId = 'did:buckyos:agent:codeassistant'
const EMPTY_READER = InMemoryConversationMessageReader.empty()

/**
 * A conversation that never reaches msg-center (Q8): the mock CodeAssistant history in memory, shown by the
 * conversation component through a source of its own.
 */
export function CodeAssistantAppPanel(props: AppContentLoaderProps) {
  void props
  const { t } = useI18n()
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
  // Messages sent before the persistent readers resolve; replayed onto them once available.
  const pendingMessagesRef = useRef<Map<string, MessageObject[]>>(new Map())
  const readersLoadedRef = useRef(false)
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
      if (cancelled) {
        return
      }

      readersLoadedRef.current = true
      const pendingMessages = pendingMessagesRef.current
      pendingMessagesRef.current = new Map()

      setLocalReaders((prev) => {
        const next = { ...prev, ...readers }
        pendingMessages.forEach((messages, sessionId) => {
          next[sessionId] = messages.reduce(
            (reader, message) => reader.append(message),
            readers[sessionId] ?? prev[sessionId] ?? EMPTY_READER,
          )
        })
        return next
      })
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

  const handleSendMessage = useCallback(async (payload: OutgoingPayload) => {
    if (!activeSession) {
      throw Error('session_missing')
    }

    const newMessage = createOutgoingMockMessage({
      sessionId: activeSession.id,
      entityId: codeAssistantEntityId,
      content: buildOutgoingDraftContent(payload),
      createdAtMs: Date.now(),
    })

    if (!readersLoadedRef.current) {
      const pending = pendingMessagesRef.current.get(activeSession.id) ?? []
      pendingMessagesRef.current.set(activeSession.id, [...pending, newMessage])
    }

    setLocalReaders((prev) => ({
      ...prev,
      [activeSession.id]: (prev[activeSession.id] ?? EMPTY_READER).append(newMessage),
    }))
  }, [activeSession])

  const entityName = entity?.name ?? ''
  const source = useMemo<ConversationSource>(() => ({
    sessionId: activeSession?.id ?? '',
    selfDid: MOCK_SELF_DID,
    reader: messageReader,
    historyStatus: 'ready',
    hasOlder: false,
    loadOlder: () => Promise.resolve(false),
    markRead: () => undefined,
    access: activeSession ? { mode: 'read_write', canManage: false, canEnableWrite: false, canEditPresentation: false, canEditSharedState: false, canEditOwnMemberState: false } : null,
    canAct: false,
    send: handleSendMessage,
    displayName: (did) => did === codeAssistantEntityId ? entityName : friendlyDidName(did, false),
    runtime: [],
    preferences: { showActions: true, pinned: null },
    peerMarkdown: true,
  }), [activeSession, messageReader, handleSendMessage, entityName])

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
        <ConversationSurface
          source={source}
          leadingPane={desktopSessionSidebarPane}
          header={({ searchButton }) => (
            <div className="flex flex-shrink-0 items-center gap-1 border-b border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] px-2 py-1.5">
              <button type="button" onClick={() => setShowSessionSidebar((prev) => !prev)} aria-label={t('messagehub.sessions')} title={t('messagehub.sessions')} className="flex min-h-11 min-w-11 items-center justify-center" style={{ color: showSessionSidebar ? 'var(--cp-accent)' : 'var(--cp-muted)' }}><Menu size={18} /></button>
              <button type="button" onClick={() => setShowDetails((prev) => !prev)} className="flex min-h-11 min-w-0 flex-1 flex-col justify-center text-left" aria-label={`${t('messagehub.entityDetails')}: ${entity.name}`}>
                <span className="truncate text-[15px] font-semibold">{entity.name}</span>
                <span className="truncate text-xs text-[color:var(--cp-muted)]">{activeSession?.title}</span>
              </button>
              {searchButton}
            </div>
          )}
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
}: OutgoingPayload): string {
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
