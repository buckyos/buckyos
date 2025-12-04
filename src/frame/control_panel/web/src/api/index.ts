import {buckyos} from 'buckyos'

const mockLayoutData: RootLayoutData = {
  primaryNav: [
    { label: 'Dashboard', icon: '📊', path: '/' },
    { label: 'User Management', icon: '👥', path: '/users' },
    { label: 'Storage', icon: '🗄️', path: '/storage' },
    { label: 'dApp Store', icon: '🛒', path: '/dapps' },
    { label: 'Settings', icon: '⚙️', path: '/settings' },
  ],
  secondaryNav: [
    { label: 'Notifications', icon: '🔔', path: '/notifications', badge: '3' },
    { label: 'Sign Out', icon: '↪️', path: '/sign-out' },
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
    { name: 'FileSync', icon: '🗂️', status: 'running' },
    { name: 'SecureChat', icon: '💬', status: 'stopped' },
    { name: 'CloudBridge', icon: '🌉', status: 'stopped' },
    { name: 'PhotoVault', icon: '📷', status: 'running' },
    { name: 'DataAnalyzer', icon: '📊', status: 'running' },
    { name: 'WebPortal', icon: '🌐', status: 'running' },
  ],
  quickActions: [
    { label: 'Manage Users', icon: '👤', to: '/users' },
    { label: 'Storage Settings', icon: '💾', to: '/storage' },
    { label: 'Network Config', icon: '🛰️', to: '/settings' },
    { label: 'System Logs', icon: '📈', to: '/notifications' },
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
    { label: '/dev/sda1', mount: '/', totalGb: 512, usedGb: 310, fs: 'ext4' },
    { label: '/dev/sdb1', mount: '/data', totalGb: 1024, usedGb: 640, fs: 'ext4' },
  ],
}

export const fetchLayout = async (): Promise<{ data: RootLayoutData | null; error: unknown }> => {
  try {
    const rpcClient = new buckyos.kRPCClient('/kapi/control-panel')
    const result = await rpcClient.call('layout', {})
    if (!result || typeof result !== 'object') {
      throw new Error('Invalid layout response')
    }
    const merged: RootLayoutData = {
      ...mockLayoutData,
      ...(result as Record<string, unknown>),
      primaryNav: mockLayoutData.primaryNav,
      secondaryNav: mockLayoutData.secondaryNav,
    }
    console.log('fetchLayout', merged)
    return { data: merged, error: null }
  } catch (error) {
    return { data: null, error }
  }
}

export const fetchDashboard = async (): Promise<{ data: DashboardState | null; error: unknown }> => {
  try {
    const rpcClient = new buckyos.kRPCClient('/kapi/control-panel')
    const result = await rpcClient.call('dashboard', {})
    if (!result || typeof result !== 'object') {
      throw new Error('Invalid dashboard response')
    }
    const merged: DashboardState = {
      ...mockDashboardData,
      ...(result as Record<string, unknown>),
      quickActions: mockDashboardData.quickActions,
    }
    return { data: merged, error: null }
  } catch (error) {
    return { data: null, error }
  }
}

export { mockLayoutData, mockDashboardData }
