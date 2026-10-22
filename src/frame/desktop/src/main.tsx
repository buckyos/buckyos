import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { ANONYMOUS_IDENTITY, buckyos, RuntimeType, type CurrentIdentity } from 'buckyos'
import 'swiper/css'
import 'swiper/css/pagination'
import 'react-grid-layout/css/styles.css'
import './index.css'
import App from './App.tsx'
import { consumePendingSiteDataReset } from './app/settings/siteDataReset'
import { isMockRuntime } from './runtime'
import { isPublicRoute } from './publicRoutes'
import { isPortalRoot } from './app/homestation/links'
import { registerDesktopServiceWorker } from './serviceWorker'
import { setPresentationViewer } from './presentation/zone'
import { redirectToDesktopLogin } from './auth/loginRedirect'
import { setBootIdentity } from './sso/identity'

function viewerKey(identity: CurrentIdentity): string | null {
  if (identity.kind === 'account') return identity.user_id
  if (identity.kind === 'external') return identity.did
  return null
}

async function bootstrap() {
  const didRedirect = await consumePendingSiteDataReset()
  if (didRedirect) {
    return
  }

  // Login-optional internal pages (see publicRoutes.ts) skip the sign-in
  // gate so they remain reachable in a logged-out state; so does `/` on the
  // zone's short hosts, which opens the default HomeStation feed. A signed-in
  // viewer is still recognised there. The identity (account, External DID or
  // nobody) comes from the session refresh, never from a cached account.
  const optional = isPublicRoute(window.location.pathname) || isPortalRoot()
  let identity: CurrentIdentity
  if (!isMockRuntime()) {
    console.log('[bootstrap] initBuckyOS starting...')
    // Behind the Vite zone proxy (VITE_ZONE_PROXY) the dev origin *is* the
    // zone host: every /kapi, /sso_* and /ndm call is forwarded to the real
    // gateway. Passing the config skips the SDK's host discovery, which would
    // otherwise probe unreachable parent hosts of 127.0.0.1.
    if (import.meta.env.VITE_ZONE_PROXY) {
      await buckyos.initBuckyOS('control-panel', {
        appId: 'control-panel',
        zoneHost: window.location.host,
        defaultProtocol: `${window.location.protocol}//`,
        runtimeType: RuntimeType.Browser,
      })
    } else {
      await buckyos.initBuckyOS('control-panel')
    }
    console.log('[bootstrap] initBuckyOS done')
    identity = optional
      ? await buckyos.getCurrentIdentity().catch(() => ANONYMOUS_IDENTITY)
      : await buckyos.getCurrentIdentity()
    console.log('[bootstrap] identity:', identity.kind)
    if (identity.kind !== 'anonymous') {
      setPresentationViewer(viewerKey(identity))
    }
  } else {
    // The mock runtime's session lives in its in-browser control panel.
    const { mockCurrentIdentity } = await import('./api/control_panel_mock.ts')
    identity = mockCurrentIdentity()
  }
  if (!optional && identity.kind === 'anonymous') {
    console.log('[bootstrap] no active session, redirect to login')
    redirectToDesktopLogin()
    return
  }
  setBootIdentity(identity)

  createRoot(document.getElementById('root')!).render(
    <StrictMode>
      <App />
    </StrictMode>,
  )
}

function renderBootstrapFailure(error: unknown) {
  const root = document.getElementById('root')
  if (!root) return
  const message = error instanceof Error ? error.message : String(error)
  root.replaceChildren()
  const panel = document.createElement('div')
  panel.setAttribute('role', 'alert')
  panel.style.cssText =
    'max-width:32rem;margin:15vh auto 0;padding:1.5rem;font:14px/1.6 system-ui,sans-serif;'
  const title = document.createElement('p')
  title.style.cssText = 'font-size:1.25rem;font-weight:600;margin:0 0 .5rem'
  title.textContent = 'BuckyOS failed to start'
  const body = document.createElement('p')
  body.style.margin = '0 0 1rem'
  body.textContent = message
  const retry = document.createElement('button')
  retry.type = 'button'
  retry.textContent = 'Retry'
  retry.style.cssText = 'padding:.5rem 1rem;font:inherit;cursor:pointer'
  retry.addEventListener('click', () => window.location.reload())
  panel.append(title, body, retry)
  root.append(panel)
}

// Application resources for a cold start without the network (production builds only).
registerDesktopServiceWorker()

// Without this, any rejection during bootstrap (SDK host discovery, site
// data reset, session refresh) leaves a permanently blank page with the
// error only in the console.
bootstrap().catch((error) => {
  console.error('[bootstrap] failed:', error)
  renderBootstrapFailure(error)
})
