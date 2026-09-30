import { SquareArrowOutUpRight } from 'lucide-react'
import { useCallback, useMemo, useRef, useState, type ReactNode } from 'react'
import { ContentPreview, type PreviewHostAction } from '../../../../components/ContentPreview'
import type { ContentRef, PreviewSessionContext } from '../../../../components/preview/types'
import { useI18n } from '../../../../i18n/provider'
import { openPreview } from '../../../preview/launch'
import { MessageMediaHostContext, type MediaOpenRequest, type MessageMediaHostValue } from './context'
import { MediaSettingsDialog } from './MediaSettingsDialog'
import { mediaSettingsStore } from './settings'
import { conversationMediaSession, installMessageHubPreviewSources } from './source'

interface ViewerState {
  key: number
  source: ContentRef
  session?: PreviewSessionContext
  title: string
  hostContext?: string
}

let viewerCounter = 0

export function MessageMediaHost({ windowId, children }: { windowId?: string; children: ReactNode }) {
  const [viewer, setViewer] = useState<ViewerState | null>(null)
  const [settingsOpen, setSettingsOpen] = useState(false)
  const returnFocus = useRef<HTMLElement | null>(null)

  const openInWindow = useCallback((source: ContentRef, session: PreviewSessionContext | undefined, hostContext: string | undefined) => {
    if (!windowId) return null
    installMessageHubPreviewSources()
    return openPreview({ source, session, origin: { app: 'messagehub', windowId, hostContext } })
  }, [windowId])

  const value = useMemo<MessageMediaHostValue>(() => ({
    open({ attachment, kind, reader, hostContext }: MediaOpenRequest) {
      const session = reader && (kind === 'image' || kind === 'video') ? conversationMediaSession(reader, attachment) : undefined
      if (mediaSettingsStore.getSnapshot().previewTarget === 'window' && openInWindow(attachment.source, session, hostContext)) return
      returnFocus.current = document.activeElement instanceof HTMLElement ? document.activeElement : null
      viewerCounter += 1
      setViewer({ key: viewerCounter, source: attachment.source, session, title: attachment.label, hostContext })
    },
    openSettings() {
      setSettingsOpen(true)
    },
  }), [openInWindow])

  const close = useCallback(() => {
    setViewer(null)
    const target = returnFocus.current
    returnFocus.current = null
    if (target?.isConnected) window.setTimeout(() => target.focus(), 0)
  }, [])

  return (
    <MessageMediaHostContext.Provider value={value}>
      {children}
      {viewer ? (
        <MediaViewerOverlay
          viewer={viewer}
          onClose={close}
          onOpenInWindow={windowId ? (source, session) => {
            if (openInWindow(source, session, viewer.hostContext)) close()
          } : undefined}
        />
      ) : null}
      {settingsOpen ? <MediaSettingsDialog canUseWindow={!!windowId} onClose={() => setSettingsOpen(false)} /> : null}
    </MessageMediaHostContext.Provider>
  )
}

function MediaViewerOverlay({
  viewer,
  onClose,
  onOpenInWindow,
}: {
  viewer: ViewerState
  onClose: () => void
  onOpenInWindow?: (source: ContentRef, session: PreviewSessionContext | undefined) => void
}) {
  const { t } = useI18n()
  const hostActions = useMemo<PreviewHostAction[]>(() => onOpenInWindow ? [{
    id: 'messagehub-open-preview-window',
    label: t('messagehub.media.openInWindow', 'Open in Preview window'),
    icon: <SquareArrowOutUpRight size={15} />,
    placement: 'toolbar',
    onInvoke: ({ item }) => {
      const session = viewer.session?.kind === 'provider' ? { ...viewer.session, currentItemId: item.id } : viewer.session
      onOpenInWindow(item.source, session)
    },
  }] : [], [onOpenInWindow, t, viewer.session])

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label={viewer.title}
      data-testid="messagehub-media-viewer"
      className="absolute inset-0 z-50 flex items-center justify-center sm:p-6"
      style={{ background: 'rgba(8, 10, 16, 0.62)' }}
      onClick={onClose}
    >
      <div
        className="relative h-full w-full max-w-[1400px] overflow-hidden sm:rounded-[20px]"
        style={{ boxShadow: 'var(--cp-panel-shadow)' }}
        onClick={event => event.stopPropagation()}
      >
        <ContentPreview
          key={viewer.key}
          source={viewer.source}
          session={viewer.session}
          uiMode="auto"
          hostActions={hostActions}
          autoFocus
          onRequestExit={onClose}
          data-testid="messagehub-media-preview"
        />
      </div>
    </div>
  )
}
