import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { buckyos } from 'buckyos'
import 'swiper/css'
import 'swiper/css/pagination'
import 'react-grid-layout/css/styles.css'
import './index.css'
import App from './App.tsx'
import { consumePendingSiteDataReset } from './app/settings/siteDataReset'
import { isMockRuntime } from './runtime'

function redirectToDesktopLogin() {
  const loginUrl = new URL('/login', window.location.origin)
  loginUrl.searchParams.set('appid', 'control-panel')
  loginUrl.searchParams.set('redirect_url', window.location.href)
  window.location.replace(loginUrl.toString())
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
    const pathname =
      (window.location.pathname || '/').replace(/\/+$/, '') || '/'
    const isLoginRoute = pathname === '/login'
    if (!isLoginRoute) {
      const accountInfo = await buckyos.getAccountInfo()
      console.log('[bootstrap] accountInfo:', accountInfo)
      if (accountInfo == null) {
        console.log('[bootstrap] accountInfo is null, redirect to login')
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
