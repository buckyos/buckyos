import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import App from './App'
import './index.css'
import { startAppFrame } from './pickFile'

// Answer the Desktop's handshake while it is still waiting for one.
void startAppFrame()

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
