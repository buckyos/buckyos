/* The AI Workspace page of a Workspace's own origin (`aiws-<id>.<zonehost>`, 内核架构改进 D21 / T4-8): a second entry
 * of the Desktop build (`aiws.html`), served by the aiworkspace service. It is the workspace alone — no Desktop
 * shell — and signs in on its own: the Zone session this origin holds (the host-only cookies `/sso_callback` set
 * here, renewed by `/sso_refresh`), or nobody (a guest of a public link). See app/aiworkspace/links.ts. */

import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { ANONYMOUS_IDENTITY, buckyos, RuntimeType, type CurrentIdentity, type UserPreferences } from 'buckyos'
import './index.css'
import { aiwsText, loadAiwsLocale } from './app/aiworkspace/i18n'
import { FollowUserPreferences } from './app/aiworkspace/standalone/FollowUserPreferences'
import { StandalonePage } from './app/aiworkspace/standalone/StandalonePage'
import { defaultLocaleFromBrowser, I18nProvider } from './i18n/provider'
import { matchSupportedLocale } from './models/ui'
import { setPresentationViewer } from './presentation/zone'
import { isMockRuntime } from './runtime'
import { registerDesktopServiceWorker } from './serviceWorker'
import { setBootIdentity } from './sso/identity'
import { PrototypeThemeProvider } from './theme/provider'

/** How long the first paint waits for the account's settings: a slow Zone shows the page in this origin's last ones. */
const PREFERENCES_WAIT_MS = 1500

async function bootstrap() {
  const url = new URL(window.location.href)
  // the language of the page this one was opened from (links.ts workspaceLinkForLocale); the address drops it
  const launchLocale = url.searchParams.get('lang')
  let locale = defaultLocaleFromBrowser(launchLocale)
  if (launchLocale !== null) {
    url.searchParams.delete('lang')
    window.history.replaceState(window.history.state, '', url)
  }
  // the language's text table is a chunk of its own: fetched while the session starts, there before the first render
  let text = loadAiwsLocale(locale)
  let identity: CurrentIdentity = ANONYMOUS_IDENTITY
  // this origin keeps no Desktop's settings: the account's are asked for (a guest has none)
  let preferences: UserPreferences | null = {}
  if (!isMockRuntime()) {
    if (import.meta.env.VITE_ZONE_PROXY) {
      await buckyos.initBuckyOS('aiworkspace', { appId: 'aiworkspace', zoneHost: window.location.host, defaultProtocol: `${window.location.protocol}//`, runtimeType: RuntimeType.Browser })
    } else {
      await buckyos.initBuckyOS('aiworkspace')
    }
    // signing in is optional here: a public link opens for anyone
    identity = await buckyos.getCurrentIdentity().catch(() => ANONYMOUS_IDENTITY)
    if (identity.kind === 'account') setPresentationViewer(identity.user_id)
    else if (identity.kind === 'external') setPresentationViewer(identity.did)
    if (identity.kind === 'account') {
      preferences = await Promise.race([
        buckyos.getUserPreferences(),
        new Promise<null>((resolve) => setTimeout(() => resolve(null), PREFERENCES_WAIT_MS)),
      ])
      const accountLocale = launchLocale === null ? matchSupportedLocale(preferences?.locale) : null
      if (accountLocale && accountLocale !== locale) {
        locale = defaultLocaleFromBrowser(accountLocale)
        text = loadAiwsLocale(locale)
      }
    }
  } else {
    const { exposeMockZoneFiles } = await import('./app/filebrowser/mock/content')
    exposeMockZoneFiles()
  }
  setBootIdentity(identity)
  await text
  createRoot(document.getElementById('root')!).render(
    <StrictMode>
      <PrototypeThemeProvider initialMode={preferences?.theme}>
        <I18nProvider initialLocale={locale}>
          {identity.kind === 'account' && <FollowUserPreferences startup={preferences} />}
          <StandalonePage />
        </I18nProvider>
      </PrototypeThemeProvider>
    </StrictMode>,
  )
}

// application resources for a cold start without the network (production builds only; the worker serves aiws.html here)
registerDesktopServiceWorker()

bootstrap().catch((error) => {
  console.error('[aiws] failed to start:', error)
  const root = document.getElementById('root')
  if (root) root.textContent = aiwsText().t('aiworkspace.core.app.openFailed', { error: error instanceof Error ? error.message : String(error) })
})
