import {
  fetchBuckyOSDevConfig,
  fetchBuckyOSInfo,
  fetchZoneCertificates,
  fetchZoneDevices,
  fetchZoneOverview,
  setBuckyOSDevMode,
  type BuckyOSDevConfig,
  type BuckyOSInfo,
  type ZoneCertificatesResponse,
  type ZoneDevicesResponse,
  type ZoneOverviewResponse,
} from '../../../api/settings'
import type { GridDensity } from '../../../models/layout'
import { loadPrivacyOverview, type PrivacyOverview } from './privacy'

export interface Resource<T> {
  status: 'idle' | 'loading' | 'ready' | 'error'
  data: T | null
  error: string | null
}

interface ResourceData {
  software: BuckyOSInfo
  developer: BuckyOSDevConfig
  devices: ZoneDevicesResponse
  certificates: ZoneCertificatesResponse
  zoneOverview: ZoneOverviewResponse
  privacy: PrivacyOverview
}

export type ResourceKey = keyof ResourceData

export type SettingsSnapshot = { [K in ResourceKey]: Resource<ResourceData[K]> } & {
  developerSaving: boolean
  /** Desktop icon grid density (the shell reads it for its grid). */
  desktopIconSize: GridDensity
  /** Whether the status-bar message tray shows message text (this device only). */
  messagePreview: boolean
}

function errorText(error: unknown, fallback: string): string {
  if (error instanceof Error && error.message) return error.message
  if (typeof error === 'string' && error) return error
  return fallback
}

async function required<T>(call: Promise<{ data: T | null; error: unknown }>): Promise<T> {
  const { data, error } = await call
  if (data === null) throw error ?? new Error('unavailable')
  return data
}

const loaders: { [K in ResourceKey]: () => Promise<ResourceData[K]> } = {
  software: () => required(fetchBuckyOSInfo()),
  developer: () => required(fetchBuckyOSDevConfig()),
  devices: () => required(fetchZoneDevices()),
  certificates: () => required(fetchZoneCertificates()),
  zoneOverview: () => required(fetchZoneOverview()),
  privacy: loadPrivacyOverview,
}

const ICON_SIZE_KEY = 'buckyos.desktop.iconSize'
const iconSizes: readonly GridDensity[] = ['small', 'medium', 'large']

function readIconSize(): GridDensity {
  try {
    const stored = window.localStorage.getItem(ICON_SIZE_KEY)
    return iconSizes.includes(stored as GridDensity) ? (stored as GridDensity) : 'medium'
  } catch {
    return 'medium'
  }
}

const MESSAGE_PREVIEW_KEY = 'buckyos.desktop.messagePreview'

function readMessagePreview(): boolean {
  try {
    return window.localStorage.getItem(MESSAGE_PREVIEW_KEY) !== 'hidden'
  } catch {
    return true
  }
}

const idle = <T,>(): Resource<T> => ({ status: 'idle', data: null, error: null })

export class SettingsStore {
  private snapshot: SettingsSnapshot = {
    software: idle(),
    developer: idle(),
    devices: idle(),
    certificates: idle(),
    zoneOverview: idle(),
    privacy: idle(),
    developerSaving: false,
    desktopIconSize: readIconSize(),
    messagePreview: readMessagePreview(),
  }
  private listeners = new Set<() => void>()
  private inflight = new Map<ResourceKey, Promise<void>>()

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener)
    return () => this.listeners.delete(listener)
  }

  getSnapshot = (): SettingsSnapshot => this.snapshot

  private patch(next: Partial<SettingsSnapshot>) {
    this.snapshot = { ...this.snapshot, ...next }
    this.listeners.forEach((listener) => listener())
  }

  private setResource<K extends ResourceKey>(key: K, resource: Resource<ResourceData[K]>) {
    this.patch({ [key]: resource } as Partial<SettingsSnapshot>)
  }

  /** Loads once; `force` reloads, keeping the previous data on screen until the answer arrives. */
  load(key: ResourceKey, force = false): Promise<void> {
    const current = this.snapshot[key]
    const running = this.inflight.get(key)
    if (running) return running
    if (!force && current.status !== 'idle') return Promise.resolve()

    this.setResource(key, { ...current, status: 'loading', error: null })
    const work = (loaders[key]() as Promise<ResourceData[typeof key]>)
      .then((data) => this.setResource(key, { status: 'ready', data, error: null }))
      .catch((error: unknown) =>
        this.setResource(key, { status: 'error', data: current.data, error: errorText(error, `${key} unavailable`) }),
      )
      .finally(() => this.inflight.delete(key))
    this.inflight.set(key, work)
    return work
  }

  /** `sudoToken` is the administrator grant; the mock runtime needs none. */
  async setDeveloperMode(enabled: boolean, sudoToken: string | null): Promise<boolean> {
    if (this.snapshot.developerSaving) return false
    this.patch({ developerSaving: true })
    try {
      const { data, error } = await setBuckyOSDevMode(enabled, { sessionToken: sudoToken })
      if (data) {
        this.setResource('developer', { status: 'ready', data, error: null })
        return true
      }
      this.setResource('developer', {
        ...this.snapshot.developer,
        status: 'error',
        error: errorText(error, 'developer mode unavailable'),
      })
      return false
    } finally {
      this.patch({ developerSaving: false })
    }
  }

  setDesktopIconSize(size: GridDensity) {
    try {
      window.localStorage.setItem(ICON_SIZE_KEY, size)
    } catch {
      // Not persisted (private window); still applies to this page.
    }
    this.patch({ desktopIconSize: size })
  }

  setMessagePreview(shown: boolean) {
    try {
      window.localStorage.setItem(MESSAGE_PREVIEW_KEY, shown ? 'shown' : 'hidden')
    } catch {
      // Not persisted (private window); still applies to this page.
    }
    this.patch({ messagePreview: shown })
  }
}

/** Shared by the Settings app and the desktop shell (icon size, message preview). */
export const globalSettingsStore = new SettingsStore()
