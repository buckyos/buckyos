export {}

declare global {
  type IconName =
    | 'desktop'
    | 'container'
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
    | 'agent'
    | 'loop'
    | 'todo'
    | 'branch'
    | 'play'
    | 'pause'
    | 'chevron-down'
    | 'chevron-right'
    | 'close'
    | 'copy'
    | 'message'
    | 'function'
    | 'action'
    | 'search'
    | 'external'

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
    errors?: number
    drops?: number
  }

  type NetworkInterfacePoint = {
    name: string
    rxBytes: number
    txBytes: number
    rxPerSec: number
    txPerSec: number
    rxErrors: number
    txErrors: number
    rxDrops: number
    txDrops: number
  }

  type NetworkOverviewSummary = {
    rxBytes: number
    txBytes: number
    rxPerSec: number
    txPerSec: number
    rxErrors: number
    txErrors: number
    rxDrops: number
    txDrops: number
    interfaceCount: number
  }

  type NetworkOverview = {
    summary: NetworkOverviewSummary
    timeline: NetworkPoint[]
    perInterface: NetworkInterfacePoint[]
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
    rxErrors?: number
    txErrors?: number
    rxDrops?: number
    txDrops?: number
    interfaceCount?: number
    perInterface?: NetworkInterfacePoint[]
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

  type ContainerServerInfo = {
    name: string
    version: string
    apiVersion: string
    os: string
    kernel: string
    driver: string
    cgroupDriver: string
    cpuCount: number
    memTotalBytes: number
  }

  type ContainerSummary = {
    total: number
    running: number
    paused: number
    exited: number
    restarting: number
    dead: number
  }

  type ContainerItem = {
    id: string
    name: string
    image: string
    state: string
    status: string
    ports: string
    networks: string
    createdAt: string
    runningFor: string
    command: string
  }

  type ContainerOverview = {
    available: boolean
    daemonRunning: boolean
    server: ContainerServerInfo
    summary: ContainerSummary
    containers: ContainerItem[]
    notes: string[]
  }

  type ContainerActionResponse = {
    id: string
    action: 'start' | 'stop' | 'restart' | string
    ok: boolean
    stdout?: string
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

  // --- Agent Workspace Types ---

  type AgentStatus = 'idle' | 'running' | 'sleeping' | 'error' | 'offline'
  type AgentType = 'main' | 'sub'

  type WsAgent = {
    agent_id: string
    agent_name: string
    agent_type: AgentType
    status: AgentStatus
    parent_agent_id?: string
    current_run_id?: string
    last_active_at: string
  }

  type WsAgentSession = {
    session_id: string
    owner_agent: string
    title: string
    summary?: string
    status: string
    created_at: string
    updated_at: string
    last_activity_at: string
  }

  type LoopRunStatus = 'running' | 'success' | 'failed' | 'cancelled'

  type LoopRunSummary = {
    step_count: number
    task_count: number
    log_count: number
    todo_count: number
    sub_agent_count: number
  }

  type LoopRun = {
    run_id: string
    agent_id: string
    trigger_event: string
    status: LoopRunStatus
    started_at: string
    ended_at?: string
    duration?: number
    current_step_index: number
    summary: LoopRunSummary
  }

  type StepStatus = 'running' | 'success' | 'failed' | 'skipped'

  type StepLogCounts = {
    message: number
    function_call: number
    action: number
    sub_agent: number
  }

  type WsStep = {
    step_id: string
    step_index: number
    title?: string
    status: StepStatus
    started_at: string
    ended_at?: string
    duration?: number
    task_count: number
    log_counts: StepLogCounts
    output_snapshot?: string
  }

  type WsTaskStatus = 'queued' | 'running' | 'success' | 'failed'

  type WsTask = {
    task_id: string
    step_id: string
    behavior_id?: string
    status: WsTaskStatus
    model: string
    tokens_in?: number
    tokens_out?: number
    prompt_preview: string
    result_preview: string
    raw_input?: string
    raw_output?: string
    created_at: string
    duration?: number
  }

  type WorkLogType =
    | 'message_sent'
    | 'message_reply'
    | 'function_call'
    | 'action'
    | 'sub_agent_created'
    | 'sub_agent_sleep'
    | 'sub_agent_wake'
    | 'sub_agent_destroyed'

  type WorkLogStatus = 'info' | 'success' | 'failed' | 'partial'

  type WsWorkLog = {
    log_id: string
    type: WorkLogType
    agent_id: string
    related_agent_id?: string
    step_id?: string
    status: WorkLogStatus
    timestamp: string
    duration?: number
    summary: string
    payload?: Record<string, unknown>
  }

  type WsTodoStatus = 'open' | 'done'

  type WsTodo = {
    todo_id: string
    agent_id: string
    title: string
    description?: string
    status: WsTodoStatus
    created_at: string
    completed_at?: string
    created_in_step_id?: string
    completed_in_step_id?: string
  }

  type WsTabId = 'overview' | 'worklog' | 'tasks' | 'todos' | 'sub-agents'

  type InspectorTarget =
    | { kind: 'step'; data: WsStep }
    | { kind: 'task'; data: WsTask }
    | { kind: 'worklog'; data: WsWorkLog }
    | { kind: 'todo'; data: WsTodo }
    | { kind: 'sub-agent'; data: WsAgent }

  type WsWorkLogFilters = {
    stepId?: string
    type?: WorkLogType
    status?: WorkLogStatus
    keyword?: string
  }

  type WsTaskFilters = {
    stepId?: string
    status?: WsTaskStatus
  }
}
