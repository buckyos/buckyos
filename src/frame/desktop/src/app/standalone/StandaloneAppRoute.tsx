/* ── `/app/<id>`: a system app in a page of its own ── */

import { useEffect, useMemo, useState } from 'react'
import { Navigate, useLocation, useParams } from 'react-router-dom'
import { LoadingOrb } from '../../components/LoadingOrb'
import { useMediaQuery } from '@mui/material'
import { useI18n } from '../../i18n/provider'
import { desktopUIStore, useDesktopUISnapshot } from '../../models/DesktopUIDataModel'
import type { SystemPreferencesInput, WindowLaunch } from '../../models/ui'
import { useThemeMode } from '../../theme/provider'
import { agentGuide } from '../agent-setup/guide'
import { AGENT_GUIDE_APP_ID } from '../agent-setup/model'
import { pathOfLaunch } from '../launchPath'
import { AppContentRenderer } from '../registry'
import { systemAppAddress } from './address'
import { hasStandalonePage } from './catalog'
import { standaloneFrame } from './frame'
import { closeStandalonePage, openStandaloneWindow } from './host'
import { pageLaunch } from './pageLaunch'
import { StandalonePage } from './StandalonePage'

const APP_ROUTE = /^\/app\/([^/]+)/

export function StandaloneAppRoute() {
  const { appId = '' } = useParams()
  const location = useLocation()
  if (!hasStandalonePage(appId)) return <Navigate to="/" replace />
  // An app that has a route of its own is shown there.
  if (!APP_ROUTE.test(systemAppAddress(appId))) {
    const rest = location.pathname.replace(APP_ROUTE, '')
    return <Navigate to={systemAppAddress(appId, `${rest || '/'}${location.search}${location.hash}`)} replace />
  }
  return appId === AGENT_GUIDE_APP_ID ? <AgentGuidePage /> : <SystemAppPage key={appId} appId={appId} />
}

function SystemAppPage({ appId }: { appId: string }) {
  const location = useLocation()
  // eslint-disable-next-line react-hooks/exhaustive-deps -- every arrival at an address is a launch
  const launch = useMemo(() => pageLaunch(appId, location), [appId, location.key])
  return (
    <StandalonePage appId={appId} launch={launch} editsAppearance={appId === 'settings'}>
      {({ app, windowId, launch }) => (
        <div className={app.manifest.contentPadding === 'none' ? 'min-h-0 flex-1 overflow-hidden' : 'desktop-scrollbar min-h-0 flex-1 overflow-y-auto p-4 md:p-5'}>
          <SystemAppContent appId={app.id} windowId={windowId} launch={launch} />
        </div>
      )}
    </StandalonePage>
  )
}

function SystemAppContent({ appId, windowId, launch }: { appId: string; windowId: string; launch?: WindowLaunch }) {
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
    />
  )
}

/** The Jarvis guide is no app of its own: it goes on to the setup wizard, or to the Agent created with it. */
function AgentGuidePage() {
  const narrow = useMediaQuery('(max-width: 768px)')
  const [formFactor] = useState(narrow ? 'mobile' as const : 'desktop' as const)
  useEffect(() => {
    desktopUIStore.startStandalone({
      appId: AGENT_GUIDE_APP_ID,
      openWindow: (app, request) => {
        if (hasStandalonePage(app.id)) {
          window.location.replace(systemAppAddress(app.id, request.path ?? pathOfLaunch(app, request.launch)))
        } else {
          openStandaloneWindow(app, request)
          closeStandalonePage()
        }
        return app.id
      },
      close: closeStandalonePage,
    }, formFactor)
    const disconnect = standaloneFrame.connect({ beforeClose: () => Promise.resolve(true), back: () => false, ownAppearance: false })
    void agentGuide.open()
    return disconnect
  }, [formFactor])
  return (
    <main className="flex h-dvh items-center justify-center bg-[color:var(--cp-bg)]" data-testid="standalone-agent-guide" aria-busy="true">
      <LoadingOrb size={44} />
    </main>
  )
}
