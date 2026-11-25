import { useMediaQuery } from '@mui/material'
import { useMobileBackHandler, useMobileTitleOverride } from '../../desktop/windows/MobileNavContext'
/**
 * Preview App — the system's standalone content viewer (PRD §13).
 *
 * One window = one Preview Component with its own Session and browsing
 * position. The panel adds only app-level concerns: window title (the window's
 * title bar names the item, so the component's toolbar does not), "open in
 * new window", keeping a window for comparison, the Open-with sheet, Share,
 * settings and the landing page shown when the app is launched without content.
 */

import { AppWindow, Columns2, Settings2 } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  ContentPreview,
  type ContentPreviewHandle,
  type PreviewHostAction,
  type PreviewOpenWithRequest,
} from '../../components/ContentPreview'
import { refIdentity } from '../../components/preview/session'
import type { ContentRef, PreviewShareRequest } from '../../components/preview/types'
import { useI18n } from '../../i18n/provider'
import { desktopUIStore } from '../../models/DesktopUIDataModel'
import { SHARE_DIALOG_PATH, useSystemDialog } from '../../sysdlg'
import type { AppContentLoaderProps } from '../types'
import { OpenWithSheet } from '../content/OpenWithSheet'
import { shareParamsOf } from '../content/share'
import { useAppPath } from '../pathContext'
import { openPreviewInNewWindow } from './launch'
import { previewPath } from './path'
import { PreviewLanding } from './PreviewLanding'
import { PreviewSettingsDialog } from './PreviewSettingsDialog'
import { previewSettingsStore, usePreviewSettings } from './settings'
import { isPreviewLaunchPayload, PREVIEW_LAUNCH_KIND, type PreviewLaunchPayload, type PreviewOpenRequest } from './types'
import { previewWindowManager } from './windowManager'

let localRequestCounter = 0
function nextLocalRequestId() {
  localRequestCounter += 1
  return `pv-local-${localRequestCounter}`
}

export function PreviewAppPanel({ windowId, launch }: AppContentLoaderProps) {
  const { t } = useI18n()
  const isMobile = useMediaQuery('(max-width: 900px)')
  const settings = usePreviewSettings()
  const launchPayload = launch && isPreviewLaunchPayload(launch.payload) ? launch.payload : null
  const launchId = launch?.requestId ?? null
  // Content opened from the landing page lives beside the shell launch; a
  // new shell launch (different request id) always wins.
  const [override, setOverride] = useState<{ base: string | null; payload: PreviewLaunchPayload | null } | null>(null)
  const payload = override && override.base === launchId ? override.payload : launchPayload
  const [pinned, setPinned] = useState(() => (windowId ? previewWindowManager.isPinned(windowId) : false))
  const [settingsOpen, setSettingsOpen] = useState(false)
  // Keyed by request so a re-targeted window never shows a stale sheet.
  const [openWithState, setOpenWithState] = useState<{ requestId: string; request: PreviewOpenWithRequest } | null>(null)
  const previewRef = useRef<ContentPreviewHandle | null>(null)
  const systemDialog = useSystemDialog()
  const [mobileTitle, setMobileTitle] = useState<{ requestId: string; title: string; subtitle: string; source: ContentRef } | null>(null)
  useAppPath((payload && previewPath(mobileTitle?.requestId === payload.requestId ? { ...payload, source: mobileTitle.source } : payload)) ?? '/')
  useMobileTitleOverride(isMobile && mobileTitle?.requestId === payload?.requestId && mobileTitle ? { title: mobileTitle.title, subtitle: mobileTitle.subtitle } : null)

  useEffect(() => {
    if (windowId && payload) previewWindowManager.register(windowId, payload)
  }, [payload, windowId])

  const openLocal = useCallback(
    (request: PreviewOpenRequest) => {
      const next: PreviewLaunchPayload = {
        kind: PREVIEW_LAUNCH_KIND,
        requestId: nextLocalRequestId(),
        source: request.source,
        session: request.session,
        uiMode: request.uiMode ?? settings.defaultUiMode,
        fitMode: request.fitMode ?? settings.defaultFitMode,
        origin: request.origin ?? { app: 'preview' },
        createdBy: launchPayload?.createdBy ?? 'manual',
      }
      setOverride({ base: launchId, payload: next })
    },
    [launchId, launchPayload?.createdBy, settings.defaultFitMode, settings.defaultUiMode],
  )

  const exit = useCallback(() => {
    if (windowId) {
      desktopUIStore.closeWindow(windowId)
      if (isMobile && payload?.origin?.windowId) desktopUIStore.focusWindow(payload.origin.windowId)
    }
    else setOverride({ base: launchId, payload: null })
  }, [launchId, windowId, isMobile, payload])
  useMobileBackHandler(isMobile && payload?.origin?.app === 'files' ? exit : null)

  const togglePin = useCallback(() => {
    setPinned((prev) => {
      const next = !prev
      if (windowId) previewWindowManager.setPinned(windowId, next)
      return next
    })
  }, [windowId])

  const share = useCallback(
    (request: PreviewShareRequest) => {
      const params = shareParamsOf(request.item.source, request.item.title)
      if (!params) {
        desktopUIStore.setSnackbar(t('previewApp.share.unavailable', 'Only files in your Zone can be shared; {{name}} is not one, so it cannot be shared from here.', { name: request.item.title }))
        return
      }
      // The dialog layer takes no focus: keys must not reach the preview behind it (Esc would close the window).
      if (document.activeElement instanceof HTMLElement) document.activeElement.blur()
      void systemDialog.open(SHARE_DIALOG_PATH, params)
    },
    [systemDialog, t],
  )

  const hostActions = useMemo<PreviewHostAction[]>(
    () => [
      {
        id: 'open-new-window',
        label: t('previewApp.action.newWindow', 'Open in new window'),
        icon: <AppWindow size={16} />,
        placement: 'toolbar',
        onInvoke: ({ item }) => {
          openPreviewInNewWindow({
            source: item.source,
            session: payload?.session,
            uiMode: payload?.uiMode,
            fitMode: payload?.fitMode,
            origin: { app: 'preview' },
          })
        },
      },
      {
        id: 'pin',
        label: pinned ? t('previewApp.action.unpin', 'Stop keeping this window') : t('previewApp.action.pin', 'Keep this window (for comparison)'),
        icon: <Columns2 size={16} />,
        placement: 'overflow',
        onInvoke: () => togglePin(),
      },
      {
        id: 'settings',
        label: t('previewApp.action.settings', 'Preview settings…'),
        icon: <Settings2 size={16} />,
        placement: 'overflow',
        onInvoke: () => setSettingsOpen(true),
      },
    ],
    [payload?.fitMode, payload?.session, payload?.uiMode, pinned, t, togglePin],
  )

  if (!payload) {
    return (
      <div className="relative h-full w-full" data-testid="preview-app">
        <PreviewLanding onOpen={openLocal} onOpenSettings={() => setSettingsOpen(true)} />
        {settingsOpen ? <PreviewSettingsDialog onClose={() => setSettingsOpen(false)} /> : null}
      </div>
    )
  }

  const openWith = openWithState?.requestId === payload.requestId ? openWithState.request : null

  return (
    <div
      className="relative h-full w-full"
      data-testid="preview-app"
      data-pinned={pinned ? 'true' : 'false'}
      onPointerDownCapture={() => {
        if (windowId) previewWindowManager.touch(windowId)
      }}
    >
      <ContentPreview
        key={payload.requestId}
        ref={previewRef}
        source={payload.source}
        session={payload.session}
        uiMode={payload.uiMode ?? settings.defaultUiMode}
        fitMode={payload.fitMode ?? settings.defaultFitMode}
        hostActions={hostActions}
        prefetchAdjacent={settings.prefetchAdjacent}
        autoFocus
        titleInHost
        onUiModeChange={(mode) => previewSettingsStore.update({ defaultUiMode: mode })}
        onRequestExit={exit}
        onRequestOpenWith={(request) => setOpenWithState({ requestId: payload.requestId, request })}
        onRequestShare={share}
        onItemChanged={(item) => {
          // Open with… belongs to the item it was asked for.
          setOpenWithState((prev) => (prev && refIdentity(prev.request.item.source) !== refIdentity(item.source) ? null : prev))
          setMobileTitle({ requestId: payload.requestId, title: item.title, subtitle: item.count > 1 ? t('preview.counter', '{{index}} of {{count}}', { index: item.index + 1, count: item.count }) : '', source: item.source })
          if (!windowId) return
          desktopUIStore.updateWindow(windowId, { title: pinned ? `${item.title} · ${t('previewApp.kept', 'Kept')}` : item.title })
          previewWindowManager.touch(windowId, { currentKey: refIdentity(item.source) })
        }}
        onReady={(info) => {
          if (windowId) previewWindowManager.touch(windowId, { currentMediaType: info.mediaType })
        }}
      />
      {openWith ? <OpenWithSheet target={{ source: openWith.item.source, session: payload.session, name: openWith.item.title, mediaType: openWith.mediaType, size: openWith.resolved?.size, readRef: openWith.resolved?.readRef, displayName: openWith.resolved?.displayName }} onClose={() => setOpenWithState(null)} preferFullApp={settings.preferFullApp} /> : null}
      {settingsOpen ? <PreviewSettingsDialog onClose={() => setSettingsOpen(false)} /> : null}
    </div>
  )
}
