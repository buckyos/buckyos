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
    { label: 'Desktop', icon: 'desktop', path: '/' },
    { label: 'Monitor', icon: 'dashboard', path: '/monitor' },
    { label: 'Network', icon: 'network', path: '/network' },
    { label: 'Containers', icon: 'container', path: '/containers' },
    { label: 'User Management', icon: 'users', path: '/users' },
    { label: 'Storage', icon: 'storage', path: '/storage' },
    { label: 'dApp Store', icon: 'apps', path: '/dapps' },
    { label: 'Settings', icon: 'settings', path: '/settings' },
  ],
  secondaryNav: [
    { label: 'Recent Events', icon: 'bell', path: '/notifications', badge: '3' },
    { label: 'System Logs', icon: 'chart', path: '/system-logs' },
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
    { title: 'dApp "FileSync" updated successfully', subtitle: '2 hours ago', tone: 'success' },
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
    { label: 'Network Config', icon: 'network', to: '/network' },
    { label: 'System Logs', icon: 'chart', to: '/system-logs' },
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

const mockNetworkOverview: NetworkOverview = {
  summary: {
    rxBytes: mockSystemMetrics.network.rxBytes,
    txBytes: mockSystemMetrics.network.txBytes,
    rxPerSec: mockSystemMetrics.network.rxPerSec,
    txPerSec: mockSystemMetrics.network.txPerSec,
    rxErrors: 0,
    txErrors: 0,
    rxDrops: 0,
    txDrops: 0,
    interfaceCount: 2,
  },
  timeline: [
    { time: '00:00:00', rx: 1800000, tx: 1000000, errors: 0, drops: 0 },
    { time: '00:00:01', rx: 2200000, tx: 1200000, errors: 0, drops: 0 },
    { time: '00:00:02', rx: 2100000, tx: 1180000, errors: 0, drops: 0 },
    { time: '00:00:03', rx: 2600000, tx: 1410000, errors: 0, drops: 0 },
    { time: '00:00:04', rx: 2300000, tx: 1300000, errors: 0, drops: 0 },
    { time: '00:00:05', rx: 2400000, tx: 1350000, errors: 0, drops: 0 },
  ],
  perInterface: [
    {
      name: 'eth0',
      rxBytes: 480000000,
      txBytes: 220000000,
      rxPerSec: 2000000,
      txPerSec: 1100000,
      rxErrors: 0,
      txErrors: 0,
      rxDrops: 0,
      txDrops: 0,
    },
    {
      name: 'wlan0',
      rxBytes: 100000000,
      txBytes: 20900000,
      rxPerSec: 200000,
      txPerSec: 100000,
      rxErrors: 0,
      txErrors: 0,
      rxDrops: 0,
      txDrops: 0,
    },
  ],
}

const mockGatewayOverview: GatewayOverview = {
  mode: 'sn',
  etcDir: '/opt/buckyos/etc',
  files: [
    {
      name: 'cyfs_gateway.json',
      path: '/opt/buckyos/etc/cyfs_gateway.json',
      exists: true,
      sizeBytes: 186,
      modifiedAt: '',
    },
    {
      name: 'boot_gateway.yaml',
      path: '/opt/buckyos/etc/boot_gateway.yaml',
      exists: true,
      sizeBytes: 2048,
      modifiedAt: '',
    },
    {
      name: 'node_gateway.json',
      path: '/opt/buckyos/etc/node_gateway.json',
      exists: true,
      sizeBytes: 4096,
      modifiedAt: '',
    },
  ],
  includes: ['user_gateway.yaml', 'boot_gateway.yaml', 'node_gateway.json', 'post_gateway.yaml'],
  stacks: [
    { name: 'zone_gateway_http', id: 'zone_gateway_http', protocol: 'tcp', bind: '0.0.0.0:80' },
    { name: 'node_gateway_http', id: 'node_gateway_http', protocol: 'tcp', bind: '0.0.0.0:3180' },
  ],
  tlsDomains: ['*.meteor101.web3.buckyos.ai', 'meteor101.web3.buckyos.ai'],
  routes: [
    {
      kind: 'path',
      matcher: '/kapi/control-panel/*',
      action: 'forward http://127.0.0.1:4020',
      raw: 'match ${REQ.path} "/kapi/control-panel/*" && return "forward http://127.0.0.1:4020"',
    },
    {
      kind: 'host',
      matcher: 'sys-*',
      action: 'forward http://127.0.0.1:4020/',
      raw: 'match ${REQ.host} "sys-*" && return "forward http://127.0.0.1:4020/"',
    },
  ],
  routePreview:
    'match ${REQ.path} "/kapi/control-panel/*" && return "forward http://127.0.0.1:4020"\nmatch ${REQ.host} "sys-*" && return "forward http://127.0.0.1:4020/"',
  customOverrides: [],
  notes: [
    'Gateway config loaded from /opt/buckyos/etc.',
    'No user override rules detected in user_gateway.yaml/post_gateway.yaml.',
  ],
}

const mockZoneOverview: ZoneOverview = {
  etcDir: '/opt/buckyos/etc',
  zone: {
    name: 'meteor101',
    domain: 'meteor101.web3.buckyos.ai',
    did: 'did:bns:meteor101',
    ownerDid: 'did:bns:meteor101',
    userName: 'meteor101',
    zoneIat: 1770361152,
  },
  device: {
    name: 'ood1',
    did: 'did:dev:jocxyR8Ceskn6rjgDfDYmMQ5HXJDhw_TEyJj7sqCPZA',
    type: 'ood',
    netId: 'nat',
  },
  sn: {
    url: 'http://sn.buckyos.ai/kapi/sn',
    username: 'meteor101',
  },
  files: [
    {
      name: 'start_config.json',
      path: '/opt/buckyos/etc/start_config.json',
      exists: true,
      sizeBytes: 1024,
      modifiedAt: '',
    },
    {
      name: 'node_device_config.json',
      path: '/opt/buckyos/etc/node_device_config.json',
      exists: true,
      sizeBytes: 1024,
      modifiedAt: '',
    },
    {
      name: 'node_identity.json',
      path: '/opt/buckyos/etc/node_identity.json',
      exists: true,
      sizeBytes: 1024,
      modifiedAt: '',
    },
  ],
  notes: [],
}

const mockContainerOverview: ContainerOverview = {
  available: true,
  daemonRunning: true,
  server: {
    name: 'docker-host',
    version: '25.0.3',
    apiVersion: '1.44',
    os: 'Ubuntu 24.04 LTS',
    kernel: '6.8.0',
    driver: 'overlay2',
    cgroupDriver: 'systemd',
    cpuCount: 8,
    memTotalBytes: 34_359_738_368,
  },
  summary: {
    total: 4,
    running: 2,
    paused: 0,
    exited: 2,
    restarting: 0,
    dead: 0,
  },
  containers: [
    {
      id: 'd8b7f2c9f4aa',
      name: 'control-panel-dev',
      image: 'buckyos/control-panel:nightly',
      state: 'running',
      status: 'Up 3 hours',
      ports: '0.0.0.0:4020->4020/tcp',
      networks: 'bridge',
      createdAt: '2026-02-11 09:12:10 +0800 CST',
      runningFor: '3 hours ago',
      command: '"/bin/control_panel"',
    },
    {
      id: '9ac721bc10f4',
      name: 'repo-service-dev',
      image: 'buckyos/repo-service:nightly',
      state: 'running',
      status: 'Up 3 hours',
      ports: '0.0.0.0:3000->3000/tcp',
      networks: 'bridge',
      createdAt: '2026-02-11 09:12:10 +0800 CST',
      runningFor: '3 hours ago',
      command: '"/bin/repo_service"',
    },
  ],
  notes: [],
}

const mockLogServices: SystemLogService[] = [
  { id: 'control-panel', label: 'Control Panel', path: '/opt/buckyos/logs/control-panel' },
  { id: 'cyfs_gateway', label: 'Cyfs Gateway', path: '/opt/buckyos/logs/cyfs_gateway' },
  { id: 'node_daemon', label: 'Node Daemon', path: '/opt/buckyos/logs/node_daemon' },
  { id: 'repo_service', label: 'Repo Service', path: '/opt/buckyos/logs/repo_service' },
  { id: 'scheduler', label: 'Scheduler', path: '/opt/buckyos/logs/scheduler' },
  {
    id: 'system_config_service',
    label: 'System Config',
    path: '/opt/buckyos/logs/system_config_service',
  },
  { id: 'verify_hub', label: 'Verify Hub', path: '/opt/buckyos/logs/verify_hub' },
]

const mockLogEntries: SystemLogEntry[] = [
  {
    timestamp: '01-29 08:34:18.943',
    level: 'info',
    message: 'server-runner listening on 0.0.0.0:4020',
    raw: '01-29 08:34:18.943 [INFO] server-runner listening on 0.0.0.0:4020',
    service: 'control-panel',
    file: 'control-panel_217501.log',
  },
  {
    timestamp: '01-29 08:34:29.142',
    level: 'info',
    message: 'recv http request:remote 127.0.0.1:54438 method POST host sys.meteor002.web3.buckyos.ai path /kapi/control-panel',
    raw: '01-29 08:34:29.142 [INFO] recv http request:remote 127.0.0.1:54438 method POST host sys.meteor002.web3.buckyos.ai path /kapi/control-panel',
    service: 'control-panel',
    file: 'control-panel_217501.log',
  },
  {
    timestamp: '01-29 08:32:50.642',
    level: 'info',
    message: 'update ood1 info to system_config success!',
    raw: '01-29 08:32:50.642 [INFO] update ood1 info to system_config success!',
    service: 'node_daemon',
    file: 'node_daemon_215894.log',
  },
  {
    timestamp: '01-29 08:32:50.555',
    level: 'warning',
    message: 'system config client is created,service_url:http://127.0.0.1:3200/kapi/system_config',
    raw: '01-29 08:32:50.555 [WARN] system config client is created,service_url:http://127.0.0.1:3200/kapi/system_config',
    service: 'system_config_service',
    file: 'system_config_service_215996.log',
  },
]

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

export const fetchNetworkOverview = async (): Promise<{
  data: NetworkOverview | null
  error: unknown
}> => {
  const { data, error } = await callRpc<NetworkOverview>('network.overview', {})
  if (!data) {
    return { data: mockNetworkOverview, error }
  }

  const merged: NetworkOverview = {
    ...mockNetworkOverview,
    ...(data as Record<string, unknown>),
    summary: { ...mockNetworkOverview.summary, ...(data.summary ?? {}) },
    timeline: Array.isArray(data.timeline) ? data.timeline : mockNetworkOverview.timeline,
    perInterface: Array.isArray(data.perInterface)
      ? data.perInterface
      : mockNetworkOverview.perInterface,
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

export const fetchGatewayOverview = async (): Promise<{
  data: GatewayOverview | null
  error: unknown
}> => {
  const { data, error } = await callRpc<GatewayOverview>('gateway.overview', {})
  if (!data) {
    return { data: mockGatewayOverview, error }
  }

  const merged: GatewayOverview = {
    ...mockGatewayOverview,
    ...(data as Record<string, unknown>),
    files: Array.isArray(data.files) ? data.files : mockGatewayOverview.files,
    includes: Array.isArray(data.includes) ? data.includes : mockGatewayOverview.includes,
    stacks: Array.isArray(data.stacks) ? data.stacks : mockGatewayOverview.stacks,
    tlsDomains: Array.isArray(data.tlsDomains) ? data.tlsDomains : mockGatewayOverview.tlsDomains,
    routes: Array.isArray(data.routes) ? data.routes : mockGatewayOverview.routes,
    customOverrides: Array.isArray(data.customOverrides)
      ? data.customOverrides
      : mockGatewayOverview.customOverrides,
    notes: Array.isArray(data.notes) ? data.notes : mockGatewayOverview.notes,
  }
  return { data: merged, error }
}

export const fetchGatewayFile = async (
  name: string,
): Promise<{ data: GatewayFileContent | null; error: unknown }> => {
  const { data, error } = await callRpc<GatewayFileContent>('gateway.file.get', { name })
  if (!data || typeof data.content !== 'string') {
    return { data: null, error }
  }
  return { data, error }
}

export const fetchZoneOverview = async (): Promise<{
  data: ZoneOverview | null
  error: unknown
}> => {
  const { data, error } = await callRpc<ZoneOverview>('zone.overview', {})
  if (!data) {
    return { data: mockZoneOverview, error }
  }

  const merged: ZoneOverview = {
    ...mockZoneOverview,
    ...(data as Record<string, unknown>),
    zone: { ...mockZoneOverview.zone, ...(data.zone ?? {}) },
    device: { ...mockZoneOverview.device, ...(data.device ?? {}) },
    sn: { ...mockZoneOverview.sn, ...(data.sn ?? {}) },
    files: Array.isArray(data.files) ? data.files : mockZoneOverview.files,
    notes: Array.isArray(data.notes) ? data.notes : mockZoneOverview.notes,
  }

  return { data: merged, error }
}

export const fetchContainerOverview = async (): Promise<{
  data: ContainerOverview | null
  error: unknown
}> => {
  const { data, error } = await callRpc<ContainerOverview>('container.overview', {})
  if (!data) {
    return { data: mockContainerOverview, error }
  }

  const merged: ContainerOverview = {
    ...mockContainerOverview,
    ...(data as Record<string, unknown>),
    server: { ...mockContainerOverview.server, ...(data.server ?? {}) },
    summary: { ...mockContainerOverview.summary, ...(data.summary ?? {}) },
    containers: Array.isArray(data.containers) ? data.containers : mockContainerOverview.containers,
    notes: Array.isArray(data.notes) ? data.notes : mockContainerOverview.notes,
  }

  return { data: merged, error }
}

export const runContainerAction = async (
  id: string,
  action: 'start' | 'stop' | 'restart',
): Promise<{ data: ContainerActionResponse | null; error: unknown }> =>
  callRpc<ContainerActionResponse>('container.action', { id, action })

type LogQueryParams = {
  services?: string[]
  service?: string
  file?: string
  level?: SystemLogLevel
  keyword?: string
  since?: string
  until?: string
  limit?: number
  cursor?: string
  direction?: 'forward' | 'backward'
}

type LogTailParams = {
  services?: string[]
  service?: string
  file?: string
  level?: SystemLogLevel
  keyword?: string
  limit?: number
  cursor?: string
  from?: 'start' | 'end'
}

type LogDownloadParams = {
  services: string[]
  mode: 'filtered' | 'full'
  level?: SystemLogLevel
  keyword?: string
  since?: string
  until?: string
  file?: string
}

export const fetchLogServices = async (): Promise<{
  data: SystemLogService[] | null
  error: unknown
}> => {
  const { data, error } = await callRpc<{ services: SystemLogService[] }>('system.logs.list', {})
  if (!data?.services?.length) {
    return { data: mockLogServices, error }
  }
  return { data: data.services, error }
}

export const querySystemLogs = async (
  params: LogQueryParams,
): Promise<{ data: SystemLogQueryResponse | null; error: unknown }> => {
  const { data, error } = await callRpc<SystemLogQueryResponse>('system.logs.query', params)
  if (!data) {
    return { data: { entries: mockLogEntries }, error }
  }
  return { data, error }
}

export const tailSystemLogs = async (
  params: LogTailParams,
): Promise<{ data: SystemLogQueryResponse | null; error: unknown }> => {
  const { data, error } = await callRpc<SystemLogQueryResponse>('system.logs.tail', params)
  if (!data) {
    return { data: { entries: [] }, error }
  }
  return { data, error }
}

export const downloadSystemLogs = async (
  params: LogDownloadParams,
): Promise<{ data: SystemLogDownloadResponse | null; error: unknown }> =>
  callRpc<SystemLogDownloadResponse>('system.logs.download', params)

export const fetchSysConfigTree = async (
  key: string,
  depth = 2,
): Promise<{ data: SysConfigTreeResponse | null; error: unknown }> =>
  callRpc<SysConfigTreeResponse>('sys_config.tree', { key, depth })

export {
  mockLayoutData,
  mockDashboardData,
  mockDappStoreData,
  mockSystemMetrics,
  mockSystemStatus,
  mockNetworkOverview,
  mockGatewayOverview,
  mockZoneOverview,
  mockContainerOverview,
  mockLogServices,
  mockLogEntries,
}
