/* ── Editor settings (persisted in localStorage; never in the document) ── */

import type { MockDebugMode } from '../agent/mock'

export interface CanvasSettings {
  adapter: 'mock' | 'http'
  httpBaseUrl: string
  timeoutMs: number
  mockDebugMode: MockDebugMode
  autoRunOnChange: boolean
  reducedMotion: boolean
}

const KEY = 'aicanvas.settings'

export const DEFAULT_SETTINGS: CanvasSettings = {
  adapter: 'mock',
  httpBaseUrl: '',
  timeoutMs: 120_000,
  mockDebugMode: 'normal',
  autoRunOnChange: false,
  reducedMotion: false,
}

export function loadSettings(): CanvasSettings {
  try {
    const raw = localStorage.getItem(KEY)
    return raw ? { ...DEFAULT_SETTINGS, ...(JSON.parse(raw) as Partial<CanvasSettings>) } : DEFAULT_SETTINGS
  } catch {
    return DEFAULT_SETTINGS
  }
}

export function saveSettings(s: CanvasSettings) {
  try {
    localStorage.setItem(KEY, JSON.stringify(s))
  } catch {
    /* ignore */
  }
}
