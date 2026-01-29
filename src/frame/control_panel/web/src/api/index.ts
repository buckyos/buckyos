import {buckyos} from 'buckyos'

const rpcClient = new buckyos.kRPCClient('/kapi/control-panel')

const callRpc = async <T>(
  method: string,
  params: Record<string, unknown> = {},
): Promise<{ data: T | null; error: unknown }> => {
  try {
    const result = await rpcClient.call(method, params)
    if (!result || typeof result !== 'object') {
      throw new Error(`Invalid ${method} response`)
    }
    return { data: result as T, error: null }
  } catch (error) {
    return { data: null, error }
  }
}

const mockLayoutData: RootLayoutData = {
  primaryNav: [
    { label: 'Dashboard', icon: 'dashboard', path: '/' },
    { label: 'User Management', icon: 'users', path: '/users' },
    { label: 'Storage', icon: 'storage', path: '/storage' },
    { label: 'dApp Store', icon: 'apps', path: '/dapps' },
    { label: 'Settings', icon: 'settings', path: '/settings' },
  ],
  secondaryNav: [
    { label: 'Notifications', icon: 'bell', path: '/notifications', badge: '3' },
    { label: 'Sign Out', icon: 'signout', path: '/sign-out' },
  ],
  profile: {
    name: 'Admin User',
    email: 'admin@buckyos.io',
    avatar: 'https://i.pravatar.cc/64?img=12',
  },
  systemStatus: {
    label: 'System Online',
    state: 'online',
    networkPeers: 847,
    activeSessions: 23,
  },
}

const mockDashboardData: DashboardState = {
  recentEvents: [
    { title: 'System backup completed', subtitle: '2 mins ago', tone: 'success' },
    { title: 'High memory usage detected', subtitle: '15 mins ago', tone: 'warning' },
    { title: 'New device connected: iPhone 15', subtitle: '1 hour ago', tone: 'info' },
    { title: 'dApp \"FileSync\" updated successfully', subtitle: '2 hours ago', tone: 'success' },
    { title: 'New admin policy applied', subtitle: 'Yesterday', tone: 'info' },
  ],
  dapps: [
    { name: 'FileSync', icon: 'package', status: 'running' },
    { name: 'SecureChat', icon: 'package', status: 'stopped' },
    { name: 'CloudBridge', icon: 'package', status: 'stopped' },
    { name: 'PhotoVault', icon: 'package', status: 'running' },
    { name: 'DataAnalyzer', icon: 'package', status: 'running' },
    { name: 'WebPortal', icon: 'package', status: 'running' },
  ],
  quickActions: [
    { label: 'Manage Users', icon: 'users', to: '/users' },
    { label: 'Storage Settings', icon: 'storage', to: '/storage' },
    { label: 'Network Config', icon: 'network', to: '/settings' },
    { label: 'System Logs', icon: 'chart', to: '/notifications' },
  ],
  resourceTimeline: [
    { time: '00:00', cpu: 52, memory: 68 },
    { time: '00:05', cpu: 62, memory: 70 },
    { time: '00:10', cpu: 58, memory: 72 },
    { time: '00:15', cpu: 54, memory: 74 },
    { time: '00:20', cpu: 57, memory: 75 },
    { time: '00:25', cpu: 60, memory: 76 },
  ],
  storageSlices: [
    { label: 'Apps', value: 28, color: '#1d4ed8' },
    { label: 'System', value: 22, color: '#6b7280' },
    { label: 'Photos', value: 18, color: '#22c55e' },
    { label: 'Documents', value: 12, color: '#facc15' },
    { label: 'Other', value: 20, color: '#38bdf8' },
  ],
  storageCapacityGb: 4000,
  storageUsedGb: 2400,
  devices: [
    { name: 'Mock Node', role: 'server', status: 'online', uptimeHours: 120, cpu: 45, memory: 62 },
  ],
  cpu: {
    usagePercent: 58,
    model: 'Mock CPU 8-Core',
    cores: 8,
  },
  memory: {
    totalGb: 32,
    usedGb: 19,
    usagePercent: 59,
  },
  disks: [
    { label: '/dev/sda1', mount: '/', totalGb: 512, usedGb: 310, fs: 'ext4', usagePercent: 60 },
    { label: '/dev/sdb1', mount: '/data', totalGb: 1024, usedGb: 640, fs: 'ext4', usagePercent: 63 },
  ],
}

const mockSystemMetrics: SystemMetrics = {
  cpu: {
    usagePercent: 58,
    model: 'Mock CPU 8-Core',
    cores: 8,
  },
  memory: {
    totalGb: 32,
    usedGb: 19,
    usagePercent: 59,
  },
  disk: {
    totalGb: 1536,
    usedGb: 950,
    usagePercent: 62,
    disks: [
      { label: '/dev/sda1', mount: '/', totalGb: 512, usedGb: 310, fs: 'ext4', usagePercent: 60 },
      { label: '/dev/sdb1', mount: '/data', totalGb: 1024, usedGb: 640, fs: 'ext4', usagePercent: 63 },
    ],
  },
  network: {
    rxBytes: 580_245_000,
    txBytes: 240_900_000,
    rxPerSec: 2_200_000,
    txPerSec: 1_200_000,
  },
  swap: {
    totalGb: 4,
    usedGb: 1.2,
    usagePercent: 30,
  },
  loadAverage: {
    one: 0.62,
    five: 0.55,
    fifteen: 0.51,
  },
  processCount: 186,
  uptimeSeconds: 345678,
}

const mockSystemStatus: SystemStatusResponse = {
  state: 'online',
  warnings: [],
  services: [
    { name: 'control-panel', status: 'running' },
    { name: 'repo-service', status: 'running' },
    { name: 'cyfs-gateway', status: 'running' },
  ],
}

const mockDappStoreData: DappCard[] = [
  { name: 'FileSync', icon: 'package', category: 'Storage', status: 'installed', version: '1.4.2' },
  { name: 'SecureChat', icon: 'package', category: 'Communication', status: 'available', version: '2.1.0' },
  { name: 'CloudBridge', icon: 'package', category: 'Networking', status: 'available', version: '0.9.5' },
  { name: 'PhotoVault', icon: 'package', category: 'Media', status: 'installed', version: '1.2.0' },
  { name: 'DataAnalyzer', icon: 'package', category: 'Analytics', status: 'installed', version: '3.0.1' },
  { name: 'WebPortal', icon: 'package', category: 'Web', status: 'available', version: '1.0.0' },
]

const defaultDappCard = (name: string): DappCard => ({
  name,
  icon: 'package',
  category: 'Service',
  status: 'installed',
  version: '0.0.0',
})

const normalizeAppStatus = (value: unknown): DappCard['status'] =>
  value === 'available' ? 'available' : 'installed'

const normalizeAppItem = (item: DappCard | string): DappCard => {
  if (typeof item === 'string') {
    return defaultDappCard(item)
  }
  const app = item as Partial<DappCard>
  return {
    name: typeof app.name === 'string' ? app.name : 'Unknown',
    icon: app.icon ?? 'package',
    category: typeof app.category === 'string' ? app.category : 'Service',
    status: normalizeAppStatus(app.status),
    version: typeof app.version === 'string' ? app.version : '0.0.0',
    settings: app.settings,
  }
}

export const fetchLayout = async (): Promise<{ data: RootLayoutData | null; error: unknown }> => {
  try {
    const { data, error } = await callRpc<RootLayoutData>('ui.layout', {})
    if (!data) {
      throw new Error('Invalid layout response')
    }
    const merged: RootLayoutData = {
      ...mockLayoutData,
      ...(data as Record<string, unknown>),
      primaryNav: mockLayoutData.primaryNav,
      secondaryNav: mockLayoutData.secondaryNav,
    }
    console.log('fetchLayout', merged)
    return { data: merged, error }
  } catch (error) {
    return { data: null, error }
  }
}

export const fetchDashboard = async (): Promise<{ data: DashboardState | null; error: unknown }> => {
  try {
    const { data, error } = await callRpc<DashboardState>('ui.dashboard', {})
    if (!data) {
      throw new Error('Invalid dashboard response')
    }
    const merged: DashboardState = {
      ...mockDashboardData,
      ...(data as Record<string, unknown>),
      quickActions: mockDashboardData.quickActions,
    }
    return { data: merged, error }
  } catch (error) {
    return { data: null, error }
  }
}

export const fetchAppsList = async (): Promise<{ data: DappCard[] | null; error: unknown }> => {
  const { data, error } = await callRpc<AppsListResponse>('apps.list', {})
  if (!data || !Array.isArray(data.items)) {
    return { data: null, error }
  }
  return { data: data.items.map((item) => normalizeAppItem(item)), error }
}

export const fetchSystemOverview = async (): Promise<{
  data: SystemOverview | null
  error: unknown
}> => callRpc<SystemOverview>('system.overview', {})

export const fetchSystemMetrics = async (
  options: { lite?: boolean } = {},
): Promise<{
  data: SystemMetrics | null
  error: unknown
}> => {
  const { data, error } = await callRpc<SystemMetrics>(
    'system.metrics',
    options.lite ? { lite: true } : {},
  )
  if (!data) {
    return { data: mockSystemMetrics, error }
  }
  const merged: SystemMetrics = {
    ...mockSystemMetrics,
    ...(data as Record<string, unknown>),
    cpu: { ...mockSystemMetrics.cpu, ...(data.cpu ?? {}) },
    memory: { ...mockSystemMetrics.memory, ...(data.memory ?? {}) },
    disk: { ...mockSystemMetrics.disk, ...(data.disk ?? {}) },
    network: { ...mockSystemMetrics.network, ...(data.network ?? {}) },
    swap: data.swap ?? mockSystemMetrics.swap,
    loadAverage: data.loadAverage ?? mockSystemMetrics.loadAverage,
    processCount: data.processCount ?? mockSystemMetrics.processCount,
    uptimeSeconds: data.uptimeSeconds ?? mockSystemMetrics.uptimeSeconds,
  }
  return { data: merged, error }
}

export const fetchSystemStatus = async (): Promise<{
  data: SystemStatusResponse | null
  error: unknown
}> => {
  const { data, error } = await callRpc<SystemStatusResponse>('system.status', {})
  if (!data) {
    return { data: mockSystemStatus, error }
  }
  const merged: SystemStatusResponse = {
    ...mockSystemStatus,
    ...(data as Record<string, unknown>),
    warnings: Array.isArray(data.warnings) ? data.warnings : mockSystemStatus.warnings,
    services: Array.isArray(data.services) ? data.services : mockSystemStatus.services,
  }
  return { data: merged, error }
}

export const fetchSysConfigTree = async (
  key: string,
  depth = 2,
): Promise<{ data: SysConfigTreeResponse | null; error: unknown }> =>
  callRpc<SysConfigTreeResponse>('sys_config.tree', { key, depth })

export { mockLayoutData, mockDashboardData, mockDappStoreData, mockSystemMetrics, mockSystemStatus }
