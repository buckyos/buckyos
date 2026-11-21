import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { initialLanguage } from './i18n'
import App from './App'
import './index.css'
import { startAppFrame } from './pickFile'

// Answer the Desktop's handshake while it is still waiting for one.
void startAppFrame()

void initialLanguage.then(() => {
  createRoot(document.getElementById('root')!).render(<StrictMode><App /></StrictMode>)
})
