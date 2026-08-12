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
import { isTransferWithFiles } from './conversation/input/attachmentDraft'
import type { DID } from './protocol/msgobj'
import type { Entity, Session } from './types'

interface ConversationViewProps {
  entity: Entity
  session: Session | null
  messageReader: ConversationMessageReader
  selfDid: DID
  onBack: () => void
  onOpenSessionSidebar: () => void
  onOpenDetails: () => void
  onSendMessage: (payload: ConversationComposerSubmitPayload) => void
  sessionCount: number
  leadingPane?: ReactNode
  isSessionSidebarOpen?: boolean
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
  sessionCount,
  leadingPane = null,
  isSessionSidebarOpen = false,
}: ConversationViewProps) {
  const { t } = useI18n()
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

  const handleSendMessage = useCallback((payload: ConversationComposerSubmitPayload) => {
    onSendMessage(payload)
    historyPaneRef.current?.scrollToBottom()
  }, [onSendMessage])

  const handleDragEnter = (event: React.DragEvent<HTMLDivElement>) => {
    if (!isTransferWithFiles(event.dataTransfer)) {
      return
    }

    event.preventDefault()
    dragDepthRef.current += 1
    setIsDropActive(true)
  }

  const handleDragOver = (event: React.DragEvent<HTMLDivElement>) => {
    if (!isTransferWithFiles(event.dataTransfer)) {
      return
    }

    event.preventDefault()
    event.dataTransfer.dropEffect = 'copy'
    setIsDropActive(true)
  }

  const handleDragLeave = (event: React.DragEvent<HTMLDivElement>) => {
    if (!isTransferWithFiles(event.dataTransfer)) {
      return
    }

    event.preventDefault()
    dragDepthRef.current = Math.max(0, dragDepthRef.current - 1)

    if (dragDepthRef.current === 0) {
      setIsDropActive(false)
    }
  }

  const handleDrop = (event: React.DragEvent<HTMLDivElement>) => {
    if (!isTransferWithFiles(event.dataTransfer)) {
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

        {sessionCount > 1 ? (
          <button
            onClick={onOpenSessionSidebar}
            className="p-1.5 rounded-lg"
            style={{
              color: isSessionSidebarOpen ? 'var(--cp-accent)' : 'var(--cp-muted)',
              background: isSessionSidebarOpen
                ? 'color-mix(in srgb, var(--cp-accent) 12%, transparent)'
                : 'transparent',
            }}
            type="button"
          >
            <Menu size={18} />
          </button>
        ) : null}

        <button
          onClick={onOpenDetails}
          className="flex items-center gap-2 flex-1 min-w-0 text-left"
          type="button"
        >
          <div className="min-w-0">
            <div className="flex items-center gap-1.5">
              <EntityTypeIcon type={entity.type} />
              <span
                className="font-semibold text-sm truncate"
                style={{ color: 'var(--cp-text)' }}
              >
                {entity.name}
              </span>
            </div>
            <p
              className="text-xs truncate"
              style={{ color: 'var(--cp-muted)' }}
            >
              {session?.title !== 'Direct Message'
                ? session?.title
                : entity.statusText}
            </p>
          </div>
        </button>

        <button
          onClick={onOpenDetails}
          className="p-1.5 rounded-lg"
          style={{ color: 'var(--cp-muted)' }}
          type="button"
        >
          <MoreVertical size={18} />
        </button>
      </div>

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
              selfDid={selfDid}
              isGroup={isGroup}
            />
          </div>

          <ConversationComposer
            ref={composerRef}
            placeholder={t('messagehub.inputPlaceholder', 'Message...')}
            maxHeight={composerMaxHeight}
            onSendMessage={handleSendMessage}
          />
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
