import { useSyncExternalStore } from 'react'

export type MediaPreviewTarget = 'overlay' | 'window'

export interface MessageHubMediaSettings {
  previewTarget: MediaPreviewTarget
  autoplayGif: boolean
}

export const DEFAULT_MEDIA_SETTINGS: MessageHubMediaSettings = {
  previewTarget: 'overlay',
  autoplayGif: true,
}

const STORAGE_KEY = 'buckyos.messagehub.media.v1'

function readStored(): MessageHubMediaSettings {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY)
    if (!raw) return DEFAULT_MEDIA_SETTINGS
    const parsed = JSON.parse(raw) as Partial<MessageHubMediaSettings>
    return {
      previewTarget: parsed.previewTarget === 'window' ? 'window' : 'overlay',
      autoplayGif: typeof parsed.autoplayGif === 'boolean' ? parsed.autoplayGif : DEFAULT_MEDIA_SETTINGS.autoplayGif,
    }
  } catch {
    return DEFAULT_MEDIA_SETTINGS
  }
}

class MediaSettingsStore {
  private snapshot: MessageHubMediaSettings = readStored()
  private listeners = new Set<() => void>()

  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }

  getSnapshot = () => this.snapshot

  update(patch: Partial<MessageHubMediaSettings>) {
    this.snapshot = { ...this.snapshot, ...patch }
    try {
      window.localStorage.setItem(STORAGE_KEY, JSON.stringify(this.snapshot))
    } catch {
      // best effort
    }
    this.listeners.forEach(listener => listener())
  }
}

export const mediaSettingsStore = new MediaSettingsStore()

export function useMediaSettings(): MessageHubMediaSettings {
  return useSyncExternalStore(mediaSettingsStore.subscribe, mediaSettingsStore.getSnapshot)
}
