export {}

declare global {
  type IconName =
    | 'desktop'
    | 'dashboard'
    | 'users'
    | 'storage'
    | 'apps'
    | 'settings'
    | 'bell'
    | 'signout'
    | 'alert'
    | 'spark'
    | 'cpu'
    | 'memory'
    | 'network'
    | 'package'
    | 'shield'
    | 'link'
    | 'activity'
    | 'drive'
    | 'chart'
    | 'server'

  type NavItem = {
    label: string
    icon: IconName
    path: string
    badge?: string
  }

  type EventItem = {
    title: string
    subtitle: string
    tone: 'success' | 'warning' | 'info'
  }

  type DappItem = {
    name: string
    status: 'running' | 'stopped'
    icon: IconName
  }

  type QuickAction = {
    label: string
    icon: IconName
    to: string
  }

  type ResourcePoint = {
    time: string
    cpu: number
    memory: number
  }

  type NetworkPoint = {
    time: string
    rx: number
    tx: number
  }

  type StorageSlice = {
    label: string
    value: number
    color: string
  }

  type PoolCategory = {
    label: string
    percent: number
    sizeGb: number
    color: string
  }

  type ActivityPoint = {
    time: string
    read: number
    write: number
  }

  type DeviceDisk = {
    label: string
    sizeTb: number
    usagePercent: number
    status: 'healthy' | 'warning' | 'offline'
  }

  type DeviceNode = {
    name: string
    role: string
    totalTb: number
    usedTb: number
    status: 'healthy' | 'degraded' | 'offline'
    disks: DeviceDisk[]
  }

  type PlaceholderPageProps = {
    title: string
    description: string
    ctaLabel?: string
  }

  type UserProfile = {
    name: string
    email: string
    avatar: string
  }

  type SystemStatus = {
    label: string
    state: 'online' | 'degraded' | 'offline'
    networkPeers: number
    activeSessions: number
  }

  type RootLayoutData = {
    primaryNav: NavItem[]
    secondaryNav: NavItem[]
    profile: UserProfile
    systemStatus: SystemStatus
  }

  type DashboardState = {
    recentEvents: EventItem[]
    dapps: DappItem[]
    quickActions: QuickAction[]
    resourceTimeline: ResourcePoint[]
    storageSlices: StorageSlice[]
    storageCapacityGb: number
    storageUsedGb: number
    devices?: DeviceInfo[]
    disks?: DiskInfo[]
    cpu?: DashboardCPU
    memory?: DashboardMemory
  }

  type DeviceInfo = {
    name: string
    role: string
    status: string
    uptimeHours: number
    cpu?: number
    memory?: number
  }

  type DashboardCPU = {
    usagePercent: number
    model?: string
    cores?: number
  }

  type DashboardMemory = {
    totalGb: number
    usedGb: number
    usagePercent: number
  }

  type DiskInfo = {
    label: string
    mount: string
    totalGb: number
    usedGb: number
    fs?: string
    usagePercent?: number
  }

  type SystemOverview = {
    name: string
    model: string
    os: string
    version: string
    uptime_seconds: number
  }

  type SystemMetricsDisk = {
    totalGb: number
    usedGb: number
    usagePercent: number
    disks: DiskInfo[]
  }

  type SystemMetricsNetwork = {
    rxBytes: number
    txBytes: number
    rxPerSec: number
    txPerSec: number
  }

  type SystemMetricsSwap = {
    totalGb: number
    usedGb: number
    usagePercent: number
  }

  type SystemLoadAverage = {
    one: number
    five: number
    fifteen: number
  }

  type SystemMetrics = {
    cpu: DashboardCPU
    memory: DashboardMemory
    disk: SystemMetricsDisk
    network: SystemMetricsNetwork
    resourceTimeline?: ResourcePoint[]
    networkTimeline?: NetworkPoint[]
    swap?: SystemMetricsSwap
    loadAverage?: SystemLoadAverage
    processCount?: number
    uptimeSeconds?: number
  }

  type SystemWarning = {
    label: string
    message: string
    severity: 'warning' | 'critical'
    value?: number
    unit?: string
  }

  type ServiceStatus = {
    name: string
    status: 'running' | 'stopped' | 'unknown'
  }

  type SystemStatusResponse = {
    state: 'online' | 'warning' | 'critical'
    warnings: SystemWarning[]
    services: ServiceStatus[]
  }

  type GatewayConfigFile = {
    name: string
    path: string
    exists: boolean
    sizeBytes: number
    modifiedAt: string
  }

  type GatewayConfigStack = {
    name: string
    id: string
    protocol: string
    bind: string
  }

  type GatewayConfigRoute = {
    kind: 'path' | 'host' | 'fallback' | 'logic'
    matcher: string
    action: string
    raw: string
  }

  type GatewayConfigOverride = {
    name: string
    preview: string
  }

  type GatewayOverview = {
    mode: 'sn' | 'direct' | string
    etcDir: string
    files: GatewayConfigFile[]
    includes: string[]
    stacks: GatewayConfigStack[]
    tlsDomains: string[]
    routes: GatewayConfigRoute[]
    routePreview: string
    customOverrides: GatewayConfigOverride[]
    notes: string[]
  }

  type GatewayFileContent = {
    name: string
    path: string
    sizeBytes: number
    modifiedAt: string
    content: string
  }

  type ZoneConfigFile = {
    name: string
    path: string
    exists: boolean
    sizeBytes: number
    modifiedAt: string
  }

  type ZoneOverview = {
    etcDir: string
    zone: {
      name: string
      domain: string
      did: string
      ownerDid: string
      userName: string
      zoneIat: number
    }
    device: {
      name: string
      did: string
      type: string
      netId: string
    }
    sn: {
      url: string
      username: string
    }
    files: ZoneConfigFile[]
    notes: string[]
  }

  type SystemLogLevel = 'info' | 'warning' | 'error' | 'unknown'

  type SystemLogEntry = {
    timestamp: string
    level: SystemLogLevel
    message: string
    raw: string
    service: string
    file: string
    line?: number
  }

  type SystemLogService = {
    id: string
    label: string
    path: string
  }

  type SystemLogQueryResponse = {
    entries: SystemLogEntry[]
    nextCursor?: string
    hasMore?: boolean
  }

  type SystemLogDownloadResponse = {
    url: string
    expiresInSec: number
    filename?: string
  }

  type SysConfigTreeResponse = {
    key: string
    depth: number
    tree: Record<string, unknown>
  }

  type UserSummary = {
    name: string
    email: string
    role: string
    status: 'active' | 'pending' | 'disabled'
    avatar: string
  }

  type DappCard = {
    name: string
    icon: IconName
    category: string
    status: 'installed' | 'available'
    version: string
    settings?: unknown
  }

  type AppsListResponse = {
    items: Array<DappCard | string>
    key?: string
  }

  type SettingBlock = {
    title: string
    description: string
    actions: string[]
    icon: IconName
  }
}
