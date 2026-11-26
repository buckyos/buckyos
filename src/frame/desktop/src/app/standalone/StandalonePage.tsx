/* ── The page of one app alone: no desktop around it ──
 *
 * Gives the app what a desktop window gives it — its title, Back, dialogs, opening other apps — in a browser tab or,
 * as the top document of one of the BuckyOS client's windows, through the client.
 */

import { Alert, useMediaQuery } from '@mui/material'
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore, type ReactNode } from 'react'
import { useLocation, useNavigate } from 'react-router-dom'
import { AccountAppearanceSync } from '../../desktop/appearance/AccountAppearanceSync'
import { StandaloneAppTitleBar } from '../../desktop/StandaloneAppTitleBar'
import { MobileNavProvider, useMobileNavState } from '../../desktop/windows/MobileNavContext'
import { resolveWindowDialogPermissions, WindowDialogProvider } from '../../desktop/windows/dialogs'
import { useI18n } from '../../i18n/provider'
import { useDesktopUISnapshot, desktopUIStore, type OpenWindowRequest } from '../../models/DesktopUIDataModel'
import type { SystemPreferencesInput, WindowLaunch } from '../../models/ui'
import { useAppText } from '../../presentation/appText'
import { useCurrentUser } from '../../presentation/currentUser'
import { useDocumentTitle } from '../../presentation/documentHead'
import { useThemeMode } from '../../theme/provider'
import { OpenWithHost } from '../content/OpenWithSheet'
import { pathOfLaunch } from '../launchPath'
import { AppPathContext } from '../pathContext'
import { PREVIEW_APP_ID } from '../preview/types'
import { AppContentRenderer } from '../registry'
import type { DesktopAppItem } from '../types'
import { systemAppAddress, systemAppPath } from './address'
import { StandalonePageContext } from './context'
import { standaloneFrame } from './frame'
import { closeStandalonePage, openStandaloneWindow } from './host'

export interface StandalonePageProps {
  appId: string
  launch?: WindowLaunch
  /** The app draws its own header and Back (MessageHub, HomeStation, AI Workspace). */
  ownHeader?: boolean
  /** The app's pages are routes: another page of it is another address, not another launch. */
  routed?: boolean
  /** Replaces the app's name in the title while the app has not set one. */
  title?: string
  /** Keeps the account's appearance, for an app that edits it. */
  editsAppearance?: boolean
  children: (page: { app: DesktopAppItem; windowId: string; launch?: WindowLaunch }) => ReactNode
}

function usePage() {
  return useSyncExternalStore(standaloneFrame.subscribe, standaloneFrame.getPage)
}

/** Theme and language: the client's where it shows the page, the account's where the page is a tab of its own. */
function PageAppearance({ editsAppearance = false }: { editsAppearance?: boolean }) {
  const user = useCurrentUser()
  return !standaloneFrame.inClient || editsAppearance
    ? user ? <AccountAppearanceSync key={user.userId} userId={user.userId} /> : null
    : null
}

/** The connection to the client. Inside the navigation context, whose Back it answers with. */
function PageFrame({ title, editsAppearance = false }: { title: string; editsAppearance?: boolean }) {
  const { canGoBack, goBack, titleOverride } = useMobileNavState()
  const latest = useRef({ canGoBack, goBack })
  useEffect(() => {
    latest.current = { canGoBack, goBack }
  })
  useEffect(() => standaloneFrame.connect({
    beforeClose: (reason) => desktopUIStore.mayCloseAll(reason),
    back: () => {
      if (!latest.current.canGoBack) return false
      latest.current.goBack()
      return true
    },
    ownAppearance: editsAppearance,
  }), [editsAppearance])
  useEffect(() => { standaloneFrame.setTitle(title) }, [title])
  useEffect(() => {
    standaloneFrame.setHeader({ title: titleOverride?.title ?? title, back: canGoBack })
  }, [title, titleOverride?.title, canGoBack])
  useDocumentTitle(title)
  return null
}

/** The bar a phone-sized page needs for its title and Back; the client's mobile container draws its own. */
function PageTitleBar({ app, appTitle }: { app: DesktopAppItem; appTitle: string }) {
  const narrow = useMediaQuery('(max-width: 768px)')
  const navigate = useNavigate()
  const { container } = usePage()
  const { canGoBack } = useMobileNavState()
  if (!narrow || container === 'mobile-app' || (container === 'desktop-app' && !canGoBack)) return null
  return (
    <StandaloneAppTitleBar
      titleKey={app.labelKey}
      title={appTitle}
      summaryKey={app.summaryKey}
      iconKey={app.iconKey}
      iconUrl={app.iconUrl}
      accent={app.accent}
      onBack={container === 'browser' ? () => { if (window.opener) closeStandalonePage(); else void navigate('/') } : undefined}
    />
  )
}

function PageSnackbar() {
  const { t } = useI18n()
  const { snackbar } = useDesktopUISnapshot().runtime
  if (!snackbar) return null
  return (
    <Alert
      severity="info"
      closeText={t('common.close')}
      onClose={() => desktopUIStore.setSnackbar(null)}
      sx={{ position: 'fixed', bottom: 20, right: 20, left: { xs: 20, sm: 'auto' }, zIndex: 2000, bgcolor: 'var(--cp-surface)', color: 'var(--cp-text)' }}
    >
      {typeof snackbar === 'string' ? snackbar : t(snackbar.key)}
    </Alert>
  )
}

export function SystemAppContent({ appId, windowId, launch, onClose }: { appId: string; windowId?: string; launch?: WindowLaunch; onClose?: () => void }) {
  const snap = useDesktopUISnapshot()
  const { locale, setLocale } = useI18n()
  const { themeMode, setThemeMode } = useThemeMode()
  const app = snap.apps.find((item) => item.id === appId)
  const { runtimeContainer, windowAppearance } = snap.syncData.appearance
  const saveSettings = (values: SystemPreferencesInput) =>
    desktopUIStore.applySettings(values, { setLocale, setThemeMode, viewportSize: { width: window.innerWidth, height: window.innerHeight } })
  if (!app || !snap.layoutState) return null
  return (
    <AppContentRenderer
      activityLog={snap.runtime.activityLog}
      app={app}
      windowId={windowId}
      launch={launch}
      layoutState={snap.layoutState}
      locale={locale}
      onSaveSettings={saveSettings}
      runtimeContainer={runtimeContainer}
      themeMode={themeMode}
      windowAppearance={windowAppearance}
      onClose={onClose}
    />
  )
}

const keepPagePath = () => {}

export function StandalonePage({ appId, launch, ownHeader = false, routed = false, title, editsAppearance, children }: StandalonePageProps) {
  const page = usePage()
  const navigate = useNavigate()
  const location = useLocation()
  const appText = useAppText()
  const narrow = useMediaQuery('(max-width: 768px)')
  const initial = useRef({ navigate, launch, narrow })
  const [windowId, setWindowId] = useState('')
  const [inPage, setInPage] = useState<{ appId: string; launch: WindowLaunch } | null>(null)
  const closeInPage = useCallback(() => setInPage(null), [])

  // Before the app renders: it asks for the other apps at once.
  useLayoutEffect(() => {
    const { navigate, launch, narrow } = initial.current
    setWindowId(desktopUIStore.startStandalone({
      appId,
      launch,
      openWindow: (app, request) => {
        if (app.id === PREVIEW_APP_ID && request.launch && !pathOfLaunch(app, request.launch)) {
          setInPage({ appId: app.id, launch: request.launch })
          return app.id
        }
        return openStandaloneWindow(app, request)
      },
      close: closeStandalonePage,
      ...(routed ? {
        navigate: (app: DesktopAppItem, request: OpenWindowRequest) => {
          const path = request.path ?? pathOfLaunch(app, request.launch)
          if (path) void navigate(systemAppAddress(appId, path))
        },
      } : {}),
    }, narrow ? 'mobile' : 'desktop'))
  }, [appId, routed])

  const snap = useDesktopUISnapshot()
  const record = snap.runtime.windows.find((w) => w.id === windowId)
  const app = snap.apps.find((item) => item.id === appId)

  // Another address of the app (a link, the browser's Back) is another launch of its window.
  const applied = useRef(location.key)
  useEffect(() => {
    if (applied.current === location.key) return
    applied.current = location.key
    if (!routed && launch) desktopUIStore.openAppWindow(appId, { launch })
  }, [appId, launch, location.key, routed])

  const requestId = record?.launch?.requestId
  const reportPath = useCallback((path: string) => {
    desktopUIStore.updateWindow(windowId, { path })
    // The client tells which page a window shows by its address; in a browser a link such as `?notify=` stays what it is.
    if (routed && !standaloneFrame.inClient) return
    // In place: an app's own navigation makes no history, as in a window.
    const address = systemAppAddress(appId, path)
    if (systemAppPath(appId, window.location) !== path) window.history.replaceState(window.history.state, '', address)
    // eslint-disable-next-line react-hooks/exhaustive-deps -- a new launch asks the app again
  }, [appId, routed, windowId, requestId])
  const value = useMemo(() => ({ container: page.container, presentation: page.presentation }), [page.container, page.presentation])

  if (!windowId || !app) return null
  const appTitle = appText(app).title
  return (
    <StandalonePageContext.Provider value={value}>
      <MobileNavProvider>
        <AppPathContext.Provider value={reportPath}>
          <main
            className="relative flex h-dvh min-h-0 w-full flex-col overflow-hidden bg-[color:var(--cp-bg)]"
            data-testid={`standalone-${appId}`}
            data-container={page.container}
            data-presentation={page.presentation}
          >
            <WindowDialogProvider permissions={resolveWindowDialogPermissions(app)} surface={narrow ? 'mobile' : 'desktop'}>
              <PageFrame title={record?.title ?? title ?? appTitle} editsAppearance={editsAppearance} />
              <PageAppearance editsAppearance={editsAppearance} />
              {ownHeader ? null : <PageTitleBar app={app} appTitle={appTitle} />}
              {children({ app, windowId, launch: record?.launch })}
              {inPage ? (
                <div className="absolute inset-0 z-40 flex flex-col bg-[color:var(--cp-bg)]" data-testid="standalone-in-page-preview">
                  <AppPathContext.Provider value={keepPagePath}>
                    <SystemAppContent appId={inPage.appId} launch={inPage.launch} onClose={closeInPage} />
                  </AppPathContext.Provider>
                </div>
              ) : null}
            </WindowDialogProvider>
          </main>
          <OpenWithHost />
          <PageSnackbar />
        </AppPathContext.Provider>
      </MobileNavProvider>
    </StandalonePageContext.Provider>
  )
}
