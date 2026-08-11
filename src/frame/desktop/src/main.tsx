import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { buckyos, getActiveSessionToken } from 'buckyos'
import 'swiper/css'
import 'swiper/css/pagination'
import 'react-grid-layout/css/styles.css'
import './index.css'
import App from './App.tsx'
import { consumePendingSiteDataReset } from './app/settings/siteDataReset'
import { isMockRuntime } from './runtime'
import { isPublicRoute } from './publicRoutes'

interface AccountInfo {
  session_token?: unknown
}

function redirectToDesktopLogin() {
  const loginUrl = new URL('/login', window.location.origin)
  loginUrl.searchParams.set('appid', 'control-panel')
  loginUrl.searchParams.set('redirect_url', window.location.href)
  window.location.replace(loginUrl.toString())
}

function accountSessionToken(accountInfo: AccountInfo | null): string {
  return typeof accountInfo?.session_token === 'string'
    ? accountInfo.session_token.trim()
    : ''
}

async function ensureDesktopSession(accountInfo: AccountInfo | null): Promise<boolean> {
  if (accountSessionToken(accountInfo)) {
    return true
  }
  const refreshedToken = await getActiveSessionToken()
  return typeof refreshedToken === 'string' && refreshedToken.trim().length > 0
}

async function bootstrap() {
  const didRedirect = await consumePendingSiteDataReset()
  if (didRedirect) {
    return
  }

  if (!isMockRuntime()) {
    console.log('[bootstrap] initBuckyOS starting...')
    await buckyos.initBuckyOS('control-panel')
    console.log('[bootstrap] initBuckyOS done')
    // Login-optional internal pages (see publicRoutes.ts) skip the account
    // gate so they remain reachable in a logged-out state.
    if (!isPublicRoute(window.location.pathname)) {
      const accountInfo = await buckyos.getAccountInfo() as AccountInfo | null
      console.log('[bootstrap] accountInfo:', accountInfo)
      if (accountInfo == null || !(await ensureDesktopSession(accountInfo))) {
        console.log('[bootstrap] accountInfo has no active session, redirect to login')
        redirectToDesktopLogin()
        return
      }
    }
  }

  createRoot(document.getElementById('root')!).render(
    <StrictMode>
      <App />
    </StrictMode>,
  )
}

void bootstrap()
