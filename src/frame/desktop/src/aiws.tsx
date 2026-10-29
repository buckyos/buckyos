/* The AI Workspace page of a Workspace's own origin (`aiws-<id>.<zonehost>`, 内核架构改进 D21 / T4-8): a second entry
 * of the Desktop build (`aiws.html`), served by the aiworkspace service. It is the workspace alone — no Desktop
 * shell — and signs in on its own: the Zone session this origin holds (the host-only cookies `/sso_callback` set
 * here, renewed by `/sso_refresh`), or nobody (a guest of a public link). See app/aiworkspace/links.ts. */

import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { ANONYMOUS_IDENTITY, buckyos, RuntimeType, type CurrentIdentity } from 'buckyos'
import './index.css'
import { StandalonePage } from './app/aiworkspace/standalone/StandalonePage'
import { defaultLocaleFromBrowser, I18nProvider } from './i18n/provider'
import { setPresentationViewer } from './presentation/zone'
import { isMockRuntime } from './runtime'
import { registerDesktopServiceWorker } from './serviceWorker'
import { setBootIdentity } from './sso/identity'
import { PrototypeThemeProvider } from './theme/provider'

async function bootstrap() {
  defaultLocaleFromBrowser()
  let identity: CurrentIdentity = ANONYMOUS_IDENTITY
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
  }
  setBootIdentity(identity)
  createRoot(document.getElementById('root')!).render(
    <StrictMode>
      <PrototypeThemeProvider>
        <I18nProvider>
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
  if (root) root.textContent = `AI Workspace failed to start: ${error instanceof Error ? error.message : String(error)}`
})
