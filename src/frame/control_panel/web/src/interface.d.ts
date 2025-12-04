export {}

declare global {
  type NavItem = {
    label: string
    icon: string
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
    icon: string
  }

  type QuickAction = {
    label: string
    icon: string
    to: string
  }

  type ResourcePoint = {
    time: string
    cpu: number
    memory: number
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
    icon: string
    category: string
    status: 'installed' | 'available'
    version: string
  }

  type SettingBlock = {
    title: string
    description: string
    actions: string[]
    icon: string
  }
}
