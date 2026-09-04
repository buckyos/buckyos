/**
 * Preview App — the system's standalone content viewer (PRD §13).
 *
 * One window = one Preview Component with its own Session and browsing
 * position. The panel adds only app-level concerns: window title, "open in
 * new window", pinning, the Open-with sheet, settings and the landing page
 * shown when the app is launched without content.
 */

import { Copy, Download, Pin, PinOff, Settings2, SquareArrowOutUpRight, X } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  ContentPreview,
  type ContentPreviewHandle,
  type PreviewHostAction,
  type PreviewOpenWithRequest,
} from '../../components/ContentPreview'
import { refIdentity } from '../../components/preview/session'
import { isCyfsPathRef, isObjectIdRef } from '../../components/preview/types'
import { useI18n } from '../../i18n/provider'
import { desktopUIStore } from '../../models/DesktopUIDataModel'
import type { AppContentLoaderProps } from '../types'
import { openPreviewInNewWindow } from './launch'
import { PreviewLanding } from './PreviewLanding'
import { PreviewSettingsDialog } from './PreviewSettingsDialog'
import { usePreviewSettings } from './settings'
import { isPreviewLaunchPayload, PREVIEW_LAUNCH_KIND, type PreviewLaunchPayload, type PreviewOpenRequest } from './types'
import { previewWindowManager } from './windowManager'

let localRequestCounter = 0
function nextLocalRequestId() {
  localRequestCounter += 1
  return `pv-local-${localRequestCounter}`
}

export function PreviewAppPanel({ windowId, launch }: AppContentLoaderProps) {
  const { t } = useI18n()
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
    if (windowId) desktopUIStore.closeWindow(windowId)
    else setOverride({ base: launchId, payload: null })
  }, [launchId, windowId])

  const togglePin = useCallback(() => {
    setPinned((prev) => {
      const next = !prev
      if (windowId) previewWindowManager.setPinned(windowId, next)
      return next
    })
  }, [windowId])

  const hostActions = useMemo<PreviewHostAction[]>(
    () => [
      {
        id: 'open-new-window',
        label: t('previewApp.action.newWindow', 'Open in new window'),
        icon: <SquareArrowOutUpRight size={15} />,
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
        label: pinned ? t('previewApp.action.unpin', 'Unpin window') : t('previewApp.action.pin', 'Pin window (keep for comparison)'),
        icon: pinned ? <PinOff size={15} className="text-[color:var(--cp-accent)]" /> : <Pin size={15} />,
        placement: 'toolbar',
        onInvoke: () => togglePin(),
      },
      {
        id: 'settings',
        label: t('previewApp.action.settings', 'Preview settings…'),
        icon: <Settings2 size={14} />,
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
        onRequestExit={exit}
        onRequestOpenWith={(request) => setOpenWithState({ requestId: payload.requestId, request })}
        onItemChanged={(item) => {
          if (!windowId) return
          desktopUIStore.updateWindow(windowId, { title: pinned ? `📌 ${item.title}` : item.title })
          previewWindowManager.touch(windowId, { currentKey: refIdentity(item.source) })
        }}
        onReady={(info) => {
          if (windowId) previewWindowManager.touch(windowId, { currentMediaType: info.mediaType })
        }}
      />
      {openWith ? <OpenWithSheet request={openWith} onClose={() => setOpenWithState(null)} preferFullApp={settings.preferFullApp} /> : null}
      {settingsOpen ? <PreviewSettingsDialog onClose={() => setSettingsOpen(false)} /> : null}
    </div>
  )
}

/**
 * "Open with…" (§8 Level 2). The Full App association protocol is not
 * frozen yet (§23.8 item 10): today the sheet offers the system fallbacks —
 * download the original and copy its reference — and states that plainly.
 */
function OpenWithSheet({
  request,
  onClose,
  preferFullApp,
}: {
  request: PreviewOpenWithRequest
  onClose: () => void
  preferFullApp: boolean
}) {
  const { t } = useI18n()
  const readRef = request.resolved?.readRef
  const source = request.item.source
  const reference = isCyfsPathRef(source) ? source.path : isObjectIdRef(source) ? source.objectId : request.item.title
  const download = () => {
    if (!readRef) return
    const anchor = document.createElement('a')
    anchor.download = request.resolved?.displayName ?? request.item.title
    if (readRef.kind === 'url') {
      anchor.href = readRef.downloadUrl ?? readRef.url
      anchor.click()
    } else {
      const url = URL.createObjectURL(readRef.blob)
      anchor.href = url
      anchor.click()
      window.setTimeout(() => URL.revokeObjectURL(url), 10_000)
    }
    onClose()
  }
  return (
    <div className="absolute inset-0 z-40 flex items-end justify-center bg-[color:color-mix(in_srgb,var(--cp-shadow)_30%,transparent)] p-4 sm:items-center" onClick={onClose} data-testid="preview-open-with">
      <div
        className="w-full max-w-sm rounded-[20px] border border-[color:var(--cp-border)] bg-[color:var(--cp-surface-opaque)] p-4 shadow-[var(--cp-panel-shadow)]"
        onClick={(event) => event.stopPropagation()}
        role="dialog"
        aria-label={t('preview.action.openWith', 'Open with…')}
      >
        <div className="flex items-center justify-between">
          <div className="text-[14px] font-semibold">{t('preview.action.openWith', 'Open with…')}</div>
          <button type="button" onClick={onClose} aria-label={t('common.close', 'Close')} className="rounded-full p-1 hover:bg-[color:var(--cp-surface-2)]">
            <X size={14} />
          </button>
        </div>
        <p className="mt-2 text-[12px] leading-5 text-[color:var(--cp-muted)]">
          {preferFullApp
            ? t('previewApp.openWith.noFullApp', 'No dedicated app is installed for {{type}}. You can download the original or copy its reference.', { type: request.mediaType ?? request.item.title })
            : t('previewApp.openWith.fallback', 'Download the original or copy its reference to open it elsewhere.')}
        </p>
        <div className="mt-3 flex flex-col gap-2">
          {readRef ? (
            <button type="button" onClick={download} className="flex items-center gap-2 rounded-xl border border-[color:var(--cp-border)] px-3 py-2 text-[13px] hover:border-[color:var(--cp-accent)]">
              <Download size={15} /> {t('preview.action.download', 'Download original')}
            </button>
          ) : null}
          <button
            type="button"
            onClick={() => {
              void navigator.clipboard?.writeText(reference).catch(() => {})
              onClose()
            }}
            className="flex items-center gap-2 rounded-xl border border-[color:var(--cp-border)] px-3 py-2 text-[13px] hover:border-[color:var(--cp-accent)]"
          >
            <Copy size={15} /> {t('preview.action.copySource', 'Copy path / object id')}
          </button>
        </div>
      </div>
    </div>
  )
}
