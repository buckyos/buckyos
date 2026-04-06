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

async function bootstrap() {
  const didRedirect = await consumePendingSiteDataReset()
  if (didRedirect) {
    return
  }

  if (!isMockRuntime()) {
    console.log('[bootstrap] initBuckyOS starting...')
    await buckyos.initBuckyOS('control-panel')
    console.log('[bootstrap] initBuckyOS done')
  }

  createRoot(document.getElementById('root')!).render(
    <StrictMode>
      <App />
    </StrictMode>,
  )
}

void bootstrap()
