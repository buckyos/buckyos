import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { SWRConfig } from 'swr'
import { initialLanguage } from './i18n'
import App from './App'
import { swrConfig } from './lib'
import './index.css'
import { startAppFrame } from './pickFile'

// Answer the Desktop's handshake while it is still waiting for one.
void startAppFrame()

void initialLanguage.then(() => {
  createRoot(document.getElementById('root')!).render(<StrictMode><SWRConfig value={swrConfig}><App /></SWRConfig></StrictMode>)
})
