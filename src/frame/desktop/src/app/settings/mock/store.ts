import {
  fetchBuckyOSDevConfig,
  fetchBuckyOSInfo,
  setBuckyOSDevMode,
  type BuckyOSDevConfig,
  type BuckyOSInfo,
} from '../../../api/settings'
import { isMockRuntime } from '../../../runtime'
import type {
  DeveloperInfo,
  SettingsStoreSnapshot,
  FontSize,
  SoftwareInfo,
} from './types'
import { getEmptySeed, getPopulatedSeed } from './seed'

function getScenarioFromURL(): 'empty' | 'populated' {
  const params = new URLSearchParams(window.location.search)
  return params.get('scenario') === 'empty' ? 'empty' : 'populated'
}

function normalizeReleaseChannel(channel: string): SoftwareInfo['releaseChannel'] {
  switch (channel.trim().toLowerCase()) {
    case 'stable':
      return 'stable'
    case 'beta':
      return 'beta'
    case 'dev':
    case 'nightly':
      return 'dev'
    default:
      return 'unknown'
  }
}

function unixTimestampToISO(timestamp: number): string | null {
  const date = new Date(timestamp * 1000)
  return Number.isFinite(timestamp) && timestamp > 0 && !Number.isNaN(date.getTime())
    ? date.toISOString()
    : null
}

function softwareInfoFromBuckyOSInfo(info: BuckyOSInfo): SoftwareInfo {
  return {
    version: info.version,
    buildVersion: info.build_version?.trim() || '—',
    releaseChannel: normalizeReleaseChannel(info.release_channel),
    target: info.target,
    installedTime: unixTimestampToISO(info.installed_at),
    lastUpdateTime: unixTimestampToISO(info.updated_at),
    updateAvailable: false,
    latestVersion: null,
    autoUpdate: false,
    loading: false,
    loadError: null,
  }
}

function unloadedSoftwareInfo(): SoftwareInfo {
  return {
    version: '—',
    buildVersion: '—',
    releaseChannel: 'unknown',
    target: '—',
    installedTime: null,
    lastUpdateTime: null,
    updateAvailable: false,
    latestVersion: null,
    autoUpdate: false,
    loading: false,
    loadError: null,
  }
}

function developerStateFromConfig(
  current: DeveloperInfo,
  config: BuckyOSDevConfig,
): DeveloperInfo {
  return {
    ...current,
    modeEnabled: config.enabled,
    enabledAt: config.enabled_at ? unixTimestampToISO(config.enabled_at) : null,
    enabledBy: config.enabled_by?.trim() || null,
    loading: false,
    saving: false,
    loadError: null,
  }
}

export class SettingsMockStore {
  private data: SettingsStoreSnapshot
  private snapshot: SettingsStoreSnapshot
  private listeners: Set<() => void> = new Set()
  private loadingBuckyOSInfo = false
  private loadingBuckyOSDevConfig = false

  constructor() {
    const scenario = getScenarioFromURL()
    this.data = scenario === 'empty' ? getEmptySeed() : getPopulatedSeed()
    if (!isMockRuntime()) {
      this.data = {
        ...this.data,
        general: {
          ...this.data.general,
          software: unloadedSoftwareInfo(),
        },
        developer: {
          ...this.data.developer,
          modeEnabled: false,
          enabledAt: null,
          enabledBy: null,
          loading: false,
          saving: false,
          loadError: null,
        },
      }
    }
    this.snapshot = { ...this.data }
  }

  // ---- Subscription ----

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener)
    return () => this.listeners.delete(listener)
  }

  getSnapshot = (): SettingsStoreSnapshot => this.snapshot

  private notify() {
    this.snapshot = { ...this.data }
    this.listeners.forEach((fn) => fn())
  }

  async reloadBuckyOSInfo() {
    if (isMockRuntime() || this.loadingBuckyOSInfo) return

    this.loadingBuckyOSInfo = true
    this.setSoftwareInfo({
      ...this.data.general.software,
      loading: true,
      loadError: null,
    })

    try {
      const { data, error } = await fetchBuckyOSInfo()
      if (data) {
        this.setSoftwareInfo(softwareInfoFromBuckyOSInfo(data))
        return
      }
      const message = error instanceof Error
        ? error.message
        : 'BuckyOS information is unavailable.'
      this.setSoftwareInfo({
        ...unloadedSoftwareInfo(),
        loadError: message,
      })
    } catch (error) {
      const message = error instanceof Error
        ? error.message
        : 'BuckyOS information is unavailable.'
      this.setSoftwareInfo({
        ...unloadedSoftwareInfo(),
        loadError: message,
      })
    } finally {
      this.loadingBuckyOSInfo = false
    }
  }

  private setSoftwareInfo(software: SoftwareInfo) {
    this.data = {
      ...this.data,
      general: {
        ...this.data.general,
        software,
      },
    }
    this.notify()
  }

  // ---- Appearance mutations ----

  setTheme(theme: 'light' | 'dark') {
    this.data.session.appearance.theme = theme
    this.notify()
  }

  setLanguage(language: string) {
    this.data.session.appearance.language = language
    this.notify()
  }

  setFontSize(size: FontSize) {
    this.data.session.appearance.fontSize = size
    this.notify()
  }

  setWallpaper(wallpaper: string) {
    this.data.session.appearance.wallpaper = wallpaper
    this.notify()
  }

  renameSession(name: string) {
    this.data.session.session.name = name
    this.notify()
  }

  cloneToDeviceSession() {
    this.data.session.session = {
      ...this.data.session.session,
      sessionId: `session_device_${Date.now()}`,
      type: 'device',
      deviceId: this.data.cluster.nodes[0]?.deviceId ?? 'local',
      name: `${this.data.session.session.name} (Device)`,
    }
    this.notify()
  }

  // ---- Developer Mode ----

  toggleDeveloperMode() {
    if (isMockRuntime()) {
      this.setDeveloperInfo({
        ...this.data.developer,
        modeEnabled: !this.data.developer.modeEnabled,
      })
    }
  }

  async reloadBuckyOSDevConfig() {
    if (isMockRuntime() || this.loadingBuckyOSDevConfig) return

    this.loadingBuckyOSDevConfig = true
    this.setDeveloperInfo({
      ...this.data.developer,
      loading: true,
      loadError: null,
    })

    try {
      const { data, error } = await fetchBuckyOSDevConfig()
      if (data) {
        this.setDeveloperInfo(developerStateFromConfig(this.data.developer, data))
        return
      }
      this.setDeveloperLoadError(error)
    } catch (error) {
      this.setDeveloperLoadError(error)
    } finally {
      this.loadingBuckyOSDevConfig = false
    }
  }

  async setDeveloperMode(enabled: boolean, sudoToken: string): Promise<boolean> {
    if (isMockRuntime()) {
      this.toggleDeveloperMode()
      return true
    }
    if (this.data.developer.saving) return false

    this.setDeveloperInfo({
      ...this.data.developer,
      saving: true,
      loadError: null,
    })
    try {
      const { data, error } = await setBuckyOSDevMode(enabled, {
        sessionToken: sudoToken,
      })
      if (data) {
        this.setDeveloperInfo(developerStateFromConfig(this.data.developer, data))
        return true
      }
      this.setDeveloperLoadError(error)
      return false
    } catch (error) {
      this.setDeveloperLoadError(error)
      return false
    } finally {
      if (this.data.developer.saving) {
        this.setDeveloperInfo({
          ...this.data.developer,
          saving: false,
        })
      }
    }
  }

  private setDeveloperLoadError(error: unknown) {
    const message = error instanceof Error
      ? error.message
      : 'Developer mode configuration is unavailable.'
    this.setDeveloperInfo({
      ...this.data.developer,
      loading: false,
      saving: false,
      loadError: message,
    })
  }

  private setDeveloperInfo(developer: DeveloperInfo) {
    this.data = {
      ...this.data,
      developer,
    }
    this.notify()
  }

  // ---- Copy helpers ----

  getSystemInfoJSON(): string {
    const { software, device, snapshot } = this.data.general
    return JSON.stringify({
      buckyos_version: software.version,
      build: software.buildVersion,
      channel: software.releaseChannel,
      target: software.target,
      installed_at: software.installedTime,
      updated_at: software.lastUpdateTime,
      os: `${device.osType} ${device.osVersion}`,
      cpu: device.cpuModel,
      memory: device.totalMemory,
      storage_total: device.totalStorage,
      storage_used: snapshot.storageUsed,
      install_mode: snapshot.installMode,
    }, null, 2)
  }

  getClusterInfoJSON(): string {
    const { connectivity, zones, certificates } = this.data.cluster
    const zone = zones[0]
    const cert = certificates[0]
    return JSON.stringify({
      zone_did: zone?.zoneDID,
      owner: zone?.ownerDID,
      domain: connectivity.domain,
      sn_region: connectivity.snRegion,
      relay: connectivity.snRelay,
      ipv6: connectivity.ipv6,
      port_mapping: connectivity.portMapping,
      certificate: cert ? { type: cert.source, expiry: cert.expiryDate.slice(0, 10) } : null,
    }, null, 2)
  }
}

/** Global singleton – shared by SettingsAppPanel and any other consumer. */
export const globalSettingsStore = new SettingsMockStore()
