/* ------------------------------------------------------------------ */
/*  Settings Mock Types                                               */
/* ------------------------------------------------------------------ */

// ---- General ----

export interface SoftwareInfo {
  version: string
  buildVersion: string
  releaseChannel: 'stable' | 'beta' | 'dev'
  lastUpdateTime: string | null
  updateAvailable: boolean
  latestVersion: string | null
  autoUpdate: boolean
}

export interface DeviceInfo {
  osType: 'Windows' | 'macOS' | 'Linux'
  osVersion: string
  cpuModel: string
  cpuCores: number
  totalMemory: string
  totalStorage: string
}

export interface SystemSnapshot {
  installMode: 'desktop' | 'cluster'
  nodeCount: number
  storageUsed: string
  storageTotal: string
  enabledModules: string[]
}

export interface GeneralInfo {
  software: SoftwareInfo
  device: DeviceInfo
  snapshot: SystemSnapshot
}

// ---- Appearance ----

export type SessionType = 'shared' | 'device'
export type FontSize = 'small' | 'medium' | 'large'

export interface SessionInfo {
  sessionId: string
  name: string
  type: SessionType
  deviceId: string | null
  environment: 'desktop' | 'mobile' | 'browser'
}

export interface AppearanceSettings {
  theme: 'light' | 'dark'
  language: string
  fontSize: FontSize
  wallpaper: string
}

export interface DesktopLayout {
  layout: Record<string, unknown>
  windowState: Record<string, unknown>
}

export interface SessionConfig {
  session: SessionInfo
  appearance: AppearanceSettings
  desktop: DesktopLayout
}

// ---- Cluster Manager ----

export interface ClusterOverview {
  clusterMode: 'single_node' | 'multi_node'
  nodeCount: number
  zoneCount: number
  activeZone: string | null
}

export interface NodeInfo {
  deviceName: string
  deviceId: string
  status: 'online' | 'offline'
  zone: string
}

export type DIDMethod = 'did:web' | 'did:bns' | 'did:key'

export interface ZoneInfo {
  zoneDID: string
  didMethod: DIDMethod
  ownerDID: string
  name: string
  description: string
}

export interface ConnectivityInfo {
  domainType: 'bns_subdomain' | 'custom_domain'
  domain: string
  snRelay: boolean
  snRegion: string | null
  snTrafficUsed: string | null
  snTrafficTotal: string | null
  dnsInfo: string
  ipv4: boolean
  ipv6: boolean
  directConnect: boolean
  portMapping: boolean
  portMappingDetails: string | null
}

export interface CertificateInfo {
  source: 'auto' | 'custom'
  domain: string
  issuer: string
  expiryDate: string
  valid: boolean
  x509Raw: string
}

export interface ClusterInfo {
  overview: ClusterOverview
  nodes: NodeInfo[]
  zones: ZoneInfo[]
  connectivity: ConnectivityInfo
  certificates: CertificateInfo[]
}

// ---- Privacy ----

export interface PublicAccessEntry {
  domain: string
  label: string
  description: string
  isPublic: boolean
  enabled: boolean
}

export interface MessagingAccess {
  enabled: boolean
  canToggle: boolean
  description: string
}

export type DataVisibility = 'public' | 'shared' | 'private'

export interface DataVisibilityEntry {
  folderName: string
  visibility: DataVisibility
  description: string
  icon: string
}

export type AppRiskLevel = 'trusted' | 'elevated' | 'high_risk'

export interface AppAccessEntry {
  id: string
  name: string
  type: 'system' | 'third_party' | 'agent'
  riskLevel: AppRiskLevel
  accessScope: string
  multiUser: boolean
  extraPermissions: string[]
  description: string
}

export type DeviceCapability = 'camera' | 'microphone' | 'screen_recording' | 'iot_camera' | 'docker'
export type DeviceAccessType = 'direct' | 'data_routed'

export interface DevicePermissionEntry {
  id: string
  appName: string
  capability: DeviceCapability
  accessType: DeviceAccessType
  granted: boolean
  description: string
}

export interface PrivacyInfo {
  publicAccess: PublicAccessEntry[]
  messagingAccess: MessagingAccess
  dataVisibility: DataVisibilityEntry[]
  appAccess: AppAccessEntry[]
  devicePermissions: DevicePermissionEntry[]
}

// ---- Developer Mode ----

export type DiagnosticStatus = 'pass' | 'warn' | 'fail'

export interface DiagnosticItem {
  name: string
  status: DiagnosticStatus
  message: string
  detail?: string
}

export interface ConfigNode {
  key: string
  label: string
  type: 'folder' | 'file'
  children?: ConfigNode[]
  content?: string
}

export interface CLICommand {
  command: string
  description: string
}

export interface DeveloperInfo {
  modeEnabled: boolean
  readOnly: boolean
  diagnostics: DiagnosticItem[]
  configTree: ConfigNode[]
  cliHelpers: CLICommand[]
  logsAvailable: boolean
  lastLogExport: string | null
}

// ---- Store Snapshot ----

export interface SettingsStoreSnapshot {
  general: GeneralInfo
  session: SessionConfig
  cluster: ClusterInfo
  privacy: PrivacyInfo
  developer: DeveloperInfo
}
