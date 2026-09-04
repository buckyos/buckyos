/**
 * Preview App settings (PRD §13.8) — persisted per browser in localStorage.
 * Runtime-only preferences; nothing here is a Pipeline or cache identity.
 */

import { useSyncExternalStore } from 'react'
import type { NavigationMode, PreviewFitMode, PreviewUIMode } from '../../components/preview/types'

export type PreviewWindowMode = 'smart' | 'single'

export interface PreviewAppSettings {
  windowMode: PreviewWindowMode
  /** Upper bound for automatically created windows (manual windows excluded). */
  autoWindowLimit: number
  defaultUiMode: PreviewUIMode
  defaultFitMode: PreviewFitMode
  /** Previous / Next at a container edge. */
  containerNavigation: NavigationMode
  /** Previous / Next at the edge of an explicit multi-selection. */
  listNavigation: NavigationMode
  restoreLastSession: boolean
  prefetchAdjacent: boolean
  preferFullApp: boolean
}

/** PRD §22 default decisions. */
export const DEFAULT_PREVIEW_SETTINGS: PreviewAppSettings = {
  windowMode: 'smart',
  autoWindowLimit: 8,
  defaultUiMode: 'auto',
  defaultFitMode: 'contain',
  containerNavigation: 'bounded',
  listNavigation: 'wrap',
  restoreLastSession: false,
  prefetchAdjacent: true,
  preferFullApp: true,
}

const STORAGE_KEY = 'buckyos.preview.settings.v1'

function readStored(): PreviewAppSettings {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY)
    if (!raw) return DEFAULT_PREVIEW_SETTINGS
    const parsed = JSON.parse(raw) as Partial<PreviewAppSettings>
    const merged: PreviewAppSettings = { ...DEFAULT_PREVIEW_SETTINGS, ...parsed }
    merged.autoWindowLimit = Math.min(Math.max(Math.round(merged.autoWindowLimit) || 8, 1), 32)
    return merged
  } catch {
    return DEFAULT_PREVIEW_SETTINGS
  }
}

class PreviewSettingsStore {
  private snapshot: PreviewAppSettings = readStored()
  private listeners = new Set<() => void>()

  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }

  getSnapshot = () => this.snapshot

  update(patch: Partial<PreviewAppSettings>) {
    this.snapshot = { ...this.snapshot, ...patch }
    try {
      window.localStorage.setItem(STORAGE_KEY, JSON.stringify(this.snapshot))
    } catch {
      // Storage may be unavailable (private mode); settings stay in memory.
    }
    this.listeners.forEach((listener) => listener())
  }

  reset() {
    this.update(DEFAULT_PREVIEW_SETTINGS)
  }
}

export const previewSettingsStore = new PreviewSettingsStore()

export function usePreviewSettings(): PreviewAppSettings {
  return useSyncExternalStore(previewSettingsStore.subscribe, previewSettingsStore.getSnapshot)
}
