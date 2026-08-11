import type { SettingsStoreSnapshot, FontSize } from './types'
import { getEmptySeed, getPopulatedSeed } from './seed'

function getScenarioFromURL(): 'empty' | 'populated' {
  const params = new URLSearchParams(window.location.search)
  return params.get('scenario') === 'empty' ? 'empty' : 'populated'
}

export class SettingsMockStore {
  private data: SettingsStoreSnapshot
  private snapshot: SettingsStoreSnapshot
  private listeners: Set<() => void> = new Set()

  constructor() {
    const scenario = getScenarioFromURL()
    this.data = scenario === 'empty' ? getEmptySeed() : getPopulatedSeed()
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
    this.data.developer.modeEnabled = !this.data.developer.modeEnabled
    this.notify()
  }

  // ---- Copy helpers ----

  getSystemInfoJSON(): string {
    const { software, device, snapshot } = this.data.general
    return JSON.stringify({
      buckyos_version: software.version,
      build: software.buildVersion,
      channel: software.releaseChannel,
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
