import type { InstallerApprovalInput, SettingsInput } from '../schemas'
import type {
  AppServiceItem,
  AppServiceViewStatus,
  InstallAppInfo,
  InstallOptions,
  InstallPlan,
  InstallSourceResolution,
  InstallTask,
  InstallTaskStage,
  PickedPikgFile,
  SourceParseResult,
  TrustCheck,
} from './types'

const taskStorageKey = 'buckyos.app-service.install-task.v2'

const seedServices: AppServiceItem[] = [
  {
    id: 'app-nostr-relay',
    name: 'Nostr Relay',
    description: 'Decentralized social relay for this Zone.',
    iconKey: 'messagehub',
    version: '1.2.0',
    layer: 'app',
    status: 'running',
    docker: {
      engine: 'running',
      image: 'present',
      imageName: 'buckyos/nostr-relay:1.2.0',
      container: 'running',
    },
    diagnostics: [],
    spec: { port: '8080', protocol: 'wss', dataDir: '/data/nostr' },
    settings: { maxConnections: '1000', rateLimit: '100/min' },
    serviceInfo: { node: 'ood-primary', endpoint: 'https://relay.zone', uptime: '14d 8h' },
    logs: [
      '08:42:11 Relay listener ready on :8080',
      '08:42:13 Subscription cache restored',
      '08:44:02 128 active connections',
    ],
  },
  {
    id: 'app-filemanager',
    name: 'File Manager',
    description: 'Browse and manage files stored on your Personal Server.',
    iconKey: 'files',
    version: '0.9.3',
    layer: 'app',
    status: 'running',
    docker: {
      engine: 'running',
      image: 'present',
      imageName: 'buckyos/filemanager:0.9.3',
      container: 'running',
    },
    diagnostics: [],
    spec: { port: '8081', rootDir: '/data/files' },
    settings: { maxUploadSize: '512MB', thumbnails: 'enabled' },
    serviceInfo: { node: 'ood-primary', endpoint: 'https://files.zone', uptime: '6d 2h' },
    logs: ['10:14:33 Indexed 2,418 objects', '10:15:07 Thumbnail worker idle'],
  },
  {
    id: 'app-gitea',
    name: 'Gitea',
    description: 'Private Git hosting for development inside this Zone.',
    iconKey: 'codeassistant',
    version: '1.21.4',
    layer: 'app',
    status: 'error',
    docker: {
      engine: 'running',
      image: 'present',
      imageName: 'gitea/gitea:1.21.4',
      container: 'error',
    },
    diagnostics: [
      'The container could not start because port 3000 is already in use.',
      'Change the application port in Settings, then start the app again.',
    ],
    spec: { port: '3000', sshPort: '2222', dataDir: '/data/gitea' },
    settings: { registrationEnabled: 'false', lfsEnabled: 'true' },
    serviceInfo: { node: 'ood-primary', lastExit: '2 minutes ago', exitCode: '125' },
    logs: ['11:07:18 Binding HTTP listener to :3000', '11:07:18 Port is already allocated'],
  },
  {
    id: 'app-photoprism',
    name: 'PhotoPrism',
    description: 'Private photo library with local indexing and search.',
    iconKey: 'ai-center',
    version: '231128',
    layer: 'app',
    status: 'installing',
    docker: {
      engine: 'running',
      image: 'pulling',
      imageName: 'photoprism/photoprism:231128',
      container: 'not_created',
    },
    diagnostics: ['The application image is still downloading. It will start automatically when ready.'],
    spec: { port: '2342', dataDir: '/data/photoprism' },
    settings: {},
    serviceInfo: { node: 'ood-backup', source: 'registry.buckyos.ai' },
    logs: ['11:12:06 Pulling image layer 4 of 9'],
    installProgress: 45,
    installTaskId: 'install_demo_photoprism',
  },
  {
    id: 'app-home-assistant',
    name: 'Home Assistant',
    description: 'Local automation for devices and routines in your home.',
    iconKey: 'homestation',
    version: '2024.3.1',
    layer: 'app',
    status: 'stopped',
    docker: {
      engine: 'running',
      image: 'present',
      imageName: 'homeassistant/home-assistant:2024.3.1',
      container: 'stopped',
    },
    diagnostics: [],
    spec: { port: '8123', dataDir: '/data/homeassistant' },
    settings: { timezone: 'America/Los_Angeles', language: 'en' },
    serviceInfo: { node: 'ood-primary', lastStopped: 'Today, 09:32' },
    logs: ['09:31:58 Shutdown requested by administrator', '09:32:01 Container stopped cleanly'],
  },
  {
    id: 'app-jellyfin',
    name: 'Jellyfin',
    description: 'Stream media from your Zone to trusted devices.',
    iconKey: 'studio',
    version: '10.8.13',
    layer: 'app',
    status: 'activation_failed',
    docker: {
      engine: 'running',
      image: 'present',
      imageName: 'jellyfin/jellyfin:10.8.13',
      container: 'error',
    },
    diagnostics: [
      'Installation completed, but the application did not pass its startup health check.',
      'The installed files are intact. Review the runtime log before trying Start again.',
    ],
    spec: { port: '8096', mediaDir: '/data/media' },
    settings: { hardwareAcceleration: 'auto' },
    serviceInfo: { node: 'ood-backup', installed: 'Today, 08:18', healthCheck: 'failed' },
    logs: ['08:18:31 Container created', '08:18:36 Health check timed out after 30 seconds'],
  },
  {
    id: 'sys-gateway',
    name: 'Zone Gateway',
    description: 'HTTPS termination and Zone routing.',
    iconKey: 'diagnostics',
    version: '2.1.0',
    layer: 'system',
    status: 'running',
    docker: null,
    diagnostics: [],
    spec: { endpoint: '127.0.0.1:3180', protocol: 'https' },
    settings: {},
    serviceInfo: { node: 'ood-primary', pid: '4128', routes: '18' },
    logs: [],
  },
  {
    id: 'sys-scheduler',
    name: 'Scheduler',
    description: 'Derives deploy and runtime configuration for this Zone.',
    iconKey: 'task-center',
    version: '1.0.2',
    layer: 'system',
    status: 'running',
    docker: null,
    diagnostics: [],
    spec: { reconcileInterval: '30s', source: 'system-config' },
    settings: {},
    serviceInfo: { node: 'ood-primary', revision: '8,214', lastRun: '12 seconds ago' },
    logs: [],
  },
  {
    id: 'sys-verify-hub',
    name: 'Verify Hub',
    description: 'Session token issuance and unified sign-in.',
    iconKey: 'users-agents',
    version: '1.4.0',
    layer: 'system',
    status: 'running',
    docker: null,
    diagnostics: [],
    spec: { tokenExpiry: '3600s', provider: 'local' },
    settings: {},
    serviceInfo: { node: 'ood-primary', pid: '4216', sessions: '4' },
    logs: [],
  },
  {
    id: 'kernel-node-daemon',
    name: 'Node Daemon',
    description: 'Converges this node to its assigned configuration.',
    iconKey: 'settings',
    version: '0.7.0',
    layer: 'kernel',
    status: 'running',
    docker: null,
    diagnostics: [],
    spec: { arch: 'aarch64', platform: 'linux' },
    settings: {},
    serviceInfo: { node: 'ood-primary', pid: '3781', uptime: '14d 9h' },
    logs: [],
  },
  {
    id: 'kernel-kmsg',
    name: 'KMsg',
    description: 'Distributed kernel message queue.',
    iconKey: 'messagehub',
    version: '0.7.0',
    layer: 'kernel',
    status: 'running',
    docker: null,
    diagnostics: [],
    spec: { transport: 'local', queueDepth: '4096' },
    settings: {},
    serviceInfo: { node: 'ood-primary', pid: '3834', pending: '0' },
    logs: [],
  },
]

function createSource(
  kind: InstallSourceResolution['kind'],
  originalInput: string,
  displaySource: string,
  normalizedType: InstallSourceResolution['normalizedType'],
  normalizedValue: string,
  fileName?: string,
): InstallSourceResolution {
  return {
    kind,
    originalInput,
    displaySource,
    normalizedType,
    normalizedValue,
    fileName,
    warningCode: kind === 'unsigned-json' ? 'UNSIGNED_CANDIDATE' : undefined,
  }
}

function formatStagingHandle(file: PickedPikgFile) {
  return `staging://${file.location}/${file.name.replace(/[^a-z0-9.-]+/gi, '-').toLowerCase()}`
}

function parseStoredTask(): InstallTask | null {
  try {
    const value = window.localStorage.getItem(taskStorageKey)
    return value ? (JSON.parse(value) as InstallTask) : null
  } catch {
    return null
  }
}

export class AppServiceMockStore {
  services = structuredClone(seedServices)
  viewStatus: AppServiceViewStatus = 'loading'
  activeTask: InstallTask | null = parseStoredTask()
  private revision = 0
  private readonly listeners = new Set<() => void>()
  private readonly timers = new Set<number>()

  constructor() {
    const scenario = new URLSearchParams(window.location.search).get('appServiceScenario')
    if (scenario === 'empty') {
      this.services = this.services.filter((service) => service.layer !== 'app')
    }

    this.schedule(() => {
      this.viewStatus = scenario === 'error' ? 'error' : 'ready'
      this.ensureTaskService()
      this.emit()
      if (this.activeTask?.status === 'running') {
        this.runTaskTimeline(this.activeTask.taskId)
      }
    }, scenario === 'loading' ? 900 : 260)
  }

  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => this.listeners.delete(listener)
  }

  getRevision = () => this.revision

  retryLoad() {
    this.viewStatus = 'loading'
    this.emit()
    this.schedule(() => {
      this.viewStatus = 'ready'
      this.emit()
    }, 420)
  }

  getAllServices() {
    return this.services
  }

  getByLayer(layer: AppServiceItem['layer']) {
    return this.services.filter((service) => service.layer === layer)
  }

  getById(id: string) {
    return this.services.find((service) => service.id === id) ?? null
  }

  startService(id: string) {
    const service = this.getById(id)
    if (!service || !['stopped', 'activation_failed'].includes(service.status)) return

    service.status = 'starting'
    this.emit()
    this.schedule(() => {
      service.status = 'running'
      service.diagnostics = []
      if (service.docker) service.docker.container = 'running'
      service.logs.unshift(`${new Date().toLocaleTimeString()} Application started by administrator`)
      this.emit()
    }, 1100)
  }

  stopService(id: string) {
    const service = this.getById(id)
    if (!service || !['running', 'starting'].includes(service.status)) return

    service.status = 'stopped'
    if (service.docker) service.docker.container = 'stopped'
    service.logs.unshift(`${new Date().toLocaleTimeString()} Application stopped by administrator`)
    this.emit()
  }

  updateSettings(id: string, settings: SettingsInput) {
    const service = this.getById(id)
    if (!service) return
    service.settings = { ...settings }
    service.logs.unshift(`${new Date().toLocaleTimeString()} Settings updated`)
    this.emit()
  }

  async analyzeInstallSource(input: string | PickedPikgFile): Promise<SourceParseResult> {
    await new Promise<void>((resolve) => window.setTimeout(resolve, 360))

    if (typeof input !== 'string') {
      if (!input.name.toLowerCase().endsWith('.pikg')) {
        return { ok: false, code: 'INVALID_PIKG' }
      }
      const kind = input.location === 'device' ? 'local-pikg' : 'personal-server-pikg'
      return {
        ok: true,
        source: createSource(
          kind,
          input.name,
          input.location === 'device' ? input.name : `Personal Server / Apps / ${input.name}`,
          'staging_handle',
          formatStagingHandle(input),
          input.name,
        ),
      }
    }

    const value = input.trim()
    if (!value) return { ok: false, code: 'EMPTY_INPUT' }

    if (/^https?:\/\//i.test(value)) {
      let url: URL
      try {
        url = new URL(value)
      } catch {
        return { ok: false, code: 'INVALID_URL' }
      }

      const lowerPath = url.pathname.toLowerCase()
      if (lowerPath.endsWith('.pikg') || lowerPath.includes('/pikg/')) {
        return {
          ok: true,
          source: createSource('url-pikg', value, value, 'identifier', value),
        }
      }
      if (
        lowerPath.endsWith('.json') ||
        lowerPath.endsWith('.jwt') ||
        lowerPath.includes('app-meta') ||
        lowerPath.includes('appdoc')
      ) {
        return {
          ok: true,
          source: createSource('url-app-meta', value, value, 'identifier', value),
        }
      }
      return { ok: false, code: 'UNSUPPORTED_URL_CONTENT' }
    }

    if (/^did:[a-z0-9]+:[^\s]+$/i.test(value)) {
      return {
        ok: true,
        source: createSource('app-did', value, value, 'identifier', value),
      }
    }

    const jwtParts = value.split('.')
    if (jwtParts.length === 3 && jwtParts.every((part) => part.length >= 6)) {
      return {
        ok: true,
        source: createSource(
          'signed-jwt',
          value,
          'Pasted signed App Meta JWT',
          'identifier',
          'obj_appdoc_nextcloud_signed_7m3k',
        ),
      }
    }

    if (value.startsWith('{')) {
      try {
        const candidate = JSON.parse(value) as { id?: unknown; doc_type?: unknown }
        if (
          typeof candidate.id !== 'string' ||
          !candidate.id.startsWith('did:') ||
          !['app', 'app_doc', 'APPDOC'].includes(String(candidate.doc_type))
        ) {
          return { ok: false, code: 'INVALID_APP_META' }
        }
        return {
          ok: true,
          source: createSource(
            'unsigned-json',
            value,
            'Pasted unsigned App Meta JSON',
            'identifier',
            'obj_appdoc_nextcloud_unsigned_4q2d',
          ),
        }
      } catch {
        return { ok: false, code: 'INVALID_APP_META' }
      }
    }

    if (/^https?:/i.test(value)) return { ok: false, code: 'INVALID_URL' }
    return { ok: false, code: 'UNRECOGNIZED_INPUT' }
  }

  createInstallTask(source: InstallSourceResolution) {
    const app = this.createInstallAppInfo(source)
    const defaultOptions: InstallOptions = {
      targetNode: 'ood-primary',
      components: [...app.availableComponents],
      dataDir: app.defaultSettings.dataDir ?? '/data/nextcloud',
      networkMode: 'zone',
      autoStart: true,
    }
    const taskId = `install_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 7)}`
    this.activeTask = {
      taskId,
      app,
      plan: this.previewInstallPlan(app, defaultOptions),
      stage: 'inspect',
      status: 'waiting_for_approval',
      progress: null,
      summary: app.installReady ? 'Waiting for installation approval' : 'Installation cannot continue',
      history: [
        { stage: 'resolve', status: 'completed' },
        { stage: 'inspect', status: 'current' },
      ],
    }
    this.persistTask()
    this.emit()
    return taskId
  }

  getTask(taskId: string) {
    return this.activeTask?.taskId === taskId ? this.activeTask : null
  }

  previewInstallPlan(app: InstallAppInfo, options: InstallOptions): InstallPlan {
    const targetOverhead = options.targetNode === 'ood-backup' && !app.content.offlineReady
      ? 268_435_456
      : 0
    const missingBytes = app.content.missingBytes + targetOverhead
    return {
      options: structuredClone(options),
      permissions: app.permissions,
      impacts: ['container', 'persistent-data', 'network-route'],
      content: {
        ...app.content,
        offlineReady: missingBytes === 0,
        missingBytes,
      },
      ready: app.installReady && options.components.length > 0,
    }
  }

  approveTask(taskId: string, approval: InstallerApprovalInput) {
    const task = this.getTask(taskId)
    if (!task || !task.app.installReady) return

    const options: InstallOptions = {
      targetNode: approval.targetNode,
      components: approval.components,
      dataDir: approval.dataDir,
      networkMode: approval.networkMode,
      autoStart: approval.autoStart,
    }
    task.plan = this.previewInstallPlan(task.app, options)
    task.status = 'running'
    task.stage = task.plan.content.missingBytes > 0 ? 'acquire' : 'verify'
    task.progress = task.stage === 'acquire' ? 12 : 34
    task.summary = task.stage === 'acquire' ? 'Preparing application content' : 'Verifying application content'
    task.history = [
      { stage: 'resolve', status: 'completed' },
      { stage: 'inspect', status: 'completed' },
      ...(task.stage === 'acquire'
        ? [{ stage: 'acquire' as const, status: 'current' as const }]
        : [{ stage: 'acquire' as const, status: 'skipped' as const }, { stage: 'verify' as const, status: 'current' as const }]),
    ]
    task.failure = undefined
    task.result = undefined
    this.ensureTaskService()
    this.persistTask()
    this.emit()
    this.runTaskTimeline(taskId)
  }

  retryTask(taskId: string) {
    const task = this.getTask(taskId)
    if (!task) return
    task.status = 'running'
    task.failure = undefined
    task.stage = task.plan.content.missingBytes > 0 ? 'acquire' : 'verify'
    task.progress = task.stage === 'acquire' ? 12 : 34
    task.summary = task.stage === 'acquire' ? 'Retrying content download' : 'Retrying content verification'
    task.history = [
      { stage: 'resolve', status: 'completed' },
      { stage: 'inspect', status: 'completed' },
      { stage: 'acquire', status: task.stage === 'acquire' ? 'current' : 'skipped' },
    ]
    if (task.stage === 'verify') task.history.push({ stage: 'verify', status: 'current' })
    this.persistTask()
    this.emit()
    this.runTaskTimeline(taskId, true)
  }

  returnTaskToApproval(taskId: string) {
    const task = this.getTask(taskId)
    if (!task) return
    task.status = 'waiting_for_approval'
    task.stage = 'inspect'
    task.progress = null
    task.failure = undefined
    task.summary = 'Waiting for installation approval'
    task.history = [
      { stage: 'resolve', status: 'completed' },
      { stage: 'inspect', status: 'current' },
    ]
    this.persistTask()
    this.emit()
  }

  clearActiveTask() {
    this.activeTask = null
    window.localStorage.removeItem(taskStorageKey)
    this.emit()
  }

  private createInstallAppInfo(source: InstallSourceResolution): InstallAppInfo {
    const controlValue = source.originalInput.toLowerCase()
    const trustChecks: TrustCheck[] = [
      { code: 'document', status: 'verified', detail: 'App Document structure and object ID are valid.' },
      {
        code: 'signature',
        status: source.kind === 'unsigned-json' ? 'warning' : 'verified',
        detail: source.kind === 'unsigned-json'
          ? 'This candidate document is not signed. Authority matching is required.'
          : 'The document signature is valid.',
      },
      {
        code: 'owner',
        status: source.kind === 'unsigned-json' ? 'unknown' : 'verified',
        detail: source.kind === 'unsigned-json'
          ? 'No signed owner claim is present in the candidate.'
          : 'The signer satisfies the App DID owner constraint.',
      },
      { code: 'authority', status: 'verified', detail: 'The App DID currently publishes this document object.' },
    ]

    let blockingReason: InstallAppInfo['blockingReason']
    if (controlValue.includes('trust-pending')) {
      trustChecks[3] = {
        code: 'authority',
        status: 'pending',
        detail: 'The authoritative App DID record is not available offline yet.',
      }
      blockingReason = 'TRUST_RESOLUTION_REQUIRED'
    } else if (controlValue.includes('revoked')) {
      trustChecks[3] = {
        code: 'authority',
        status: 'failed',
        detail: 'The App DID identity has been revoked by its owner.',
      }
      blockingReason = 'IDENTITY_REVOKED'
    } else if (controlValue.includes('unsupported')) {
      blockingReason = 'TARGET_UNSUPPORTED'
    }

    const platformSupported = blockingReason !== 'TARGET_UNSUPPORTED'
    const offlineReady = ['local-pikg', 'personal-server-pikg'].includes(source.kind)
    const installReady = !blockingReason && trustChecks.every((check) => !['pending', 'failed'].includes(check.status))

    return {
      id: 'app-nextcloud',
      name: 'Nextcloud',
      version: '28.0.2',
      releaseVersion: '2026.07-stable',
      description: 'Private file sync, calendar, and collaboration for your Zone.',
      iconKey: 'files',
      appDid: 'did:cyfs:app-nextcloud-7w4k2n',
      documentObjectId: source.normalizedType === 'identifier' && source.normalizedValue.startsWith('obj_')
        ? source.normalizedValue
        : 'obj_appdoc_nextcloud_28_7m3k',
      publisher: 'Nextcloud Community Maintainers',
      referrer: 'App Service manual install',
      source,
      trustChecks,
      platformSupported,
      content: {
        offlineReady,
        missingBytes: offlineReady ? 0 : 1_879_048_192,
        availableSource: offlineReady ? 'Controlled staging area' : 'registry.buckyos.ai + repo-service',
      },
      permissions: [
        { kind: 'files', scope: '/data/nextcloud · read/write' },
        { kind: 'network', scope: 'Zone HTTPS route · inbound' },
        { kind: 'database', scope: 'Managed PostgreSQL database' },
        { kind: 'system', scope: 'Docker container runtime' },
      ],
      availableComponents: ['web', 'worker'],
      defaultSettings: { dataDir: '/data/nextcloud' },
      installReady,
      blockingReason,
    }
  }

  private runTaskTimeline(taskId: string, isRetry = false) {
    const task = this.getTask(taskId)
    if (!task || task.status !== 'running') return

    const stages: Array<{ stage: InstallTaskStage; progress: number; summary: string; resource?: string }> = task.plan.content.missingBytes > 0
      ? [
          { stage: 'acquire', progress: 28, summary: 'Downloading application content', resource: 'nextcloud-app-layer.tar.zst' },
          { stage: 'verify', progress: 48, summary: 'Verifying content hashes and signatures' },
          { stage: 'prepare', progress: 64, summary: 'Preparing the runtime environment' },
          { stage: 'deploy', progress: 82, summary: 'Registering services and applying settings' },
          { stage: 'activate', progress: 94, summary: 'Starting the app and checking health' },
        ]
      : [
          { stage: 'verify', progress: 42, summary: 'Verifying staged content' },
          { stage: 'prepare', progress: 62, summary: 'Preparing the runtime environment' },
          { stage: 'deploy', progress: 82, summary: 'Registering services and applying settings' },
          { stage: 'activate', progress: 94, summary: 'Starting the app and checking health' },
        ]

    const currentIndex = Math.max(0, stages.findIndex((item) => item.stage === task.stage))
    const advance = (index: number) => {
      const currentTask = this.getTask(taskId)
      if (!currentTask || currentTask.status !== 'running') return

      if (index >= stages.length) {
        this.completeTask(currentTask)
        return
      }

      const next = stages[index]
      if (
        next.stage === 'acquire' &&
        currentTask.app.source.originalInput.toLowerCase().includes('fail-download') &&
        !isRetry
      ) {
        this.failTask(currentTask)
        return
      }

      currentTask.stage = next.stage
      currentTask.progress = next.progress
      currentTask.summary = next.summary
      currentTask.currentResource = next.resource
      currentTask.history = this.buildHistory(currentTask, next.stage)
      const service = this.getById(currentTask.app.id)
      if (service) service.installProgress = next.progress
      this.persistTask()
      this.emit()
      this.schedule(() => advance(index + 1), 620)
    }

    this.schedule(() => advance(currentIndex), 420)
  }

  private buildHistory(task: InstallTask, currentStage: InstallTaskStage) {
    const ordered: InstallTaskStage[] = ['resolve', 'inspect', 'acquire', 'verify', 'prepare', 'deploy', 'activate']
    const currentIndex = ordered.indexOf(currentStage)
    return ordered.slice(0, currentIndex + 1).map((stage) => ({
      stage,
      status: stage === currentStage
        ? 'current' as const
        : stage === 'acquire' && task.plan.content.missingBytes === 0
          ? 'skipped' as const
          : 'completed' as const,
    }))
  }

  private completeTask(task: InstallTask) {
    const activationFailed = task.app.source.originalInput.toLowerCase().includes('activation-fail')
    task.stage = 'completed'
    task.status = 'completed'
    task.progress = 100
    task.currentResource = undefined
    task.summary = activationFailed ? 'Installed, but automatic startup failed' : 'Installed and running'
    task.history = [
      ...this.buildHistory(task, 'activate').map((item) => ({ ...item, status: item.status === 'current' ? 'completed' as const : item.status })),
      { stage: 'completed', status: 'completed' },
    ]
    task.result = {
      installed: true,
      installedVersion: task.app.version,
      targetNode: task.plan.options.targetNode,
      autoStart: task.plan.options.autoStart ? (activationFailed ? 'failed' : 'running') : 'skipped',
    }

    const service = this.getById(task.app.id)
    if (service) {
      service.status = activationFailed ? 'activation_failed' : task.plan.options.autoStart ? 'running' : 'stopped'
      service.installProgress = undefined
      service.diagnostics = activationFailed
        ? ['Installation completed, but the app did not pass its startup health check.']
        : []
      if (service.docker) {
        service.docker.image = 'present'
        service.docker.container = activationFailed ? 'error' : task.plan.options.autoStart ? 'running' : 'stopped'
      }
    }
    this.persistTask()
    this.emit()
  }

  private failTask(task: InstallTask) {
    task.status = 'failed'
    task.progress = null
    task.stage = 'acquire'
    task.summary = 'Application content could not be downloaded'
    task.failure = {
      stage: 'acquire',
      code: 'DOWNLOAD_FAILED',
      message: 'The content source stopped responding before the download completed.',
      technicalDetail: `task=${task.taskId}; stage=acquire; source=registry.buckyos.ai; error=connection_reset`,
    }
    const service = this.getById(task.app.id)
    if (service) {
      service.status = 'error'
      service.installProgress = undefined
      service.diagnostics = ['Installation stopped because the application content could not be downloaded.']
    }
    this.persistTask()
    this.emit()
  }

  private ensureTaskService() {
    const task = this.activeTask
    if (!task || task.status === 'waiting_for_approval' || this.getById(task.app.id)) return

    const completed = task.status === 'completed'
    const activationFailed = task.result?.autoStart === 'failed'
    this.services.unshift({
      id: task.app.id,
      name: task.app.name,
      description: task.app.description,
      iconKey: task.app.iconKey,
      version: task.app.version,
      layer: 'app',
      status: completed ? (activationFailed ? 'activation_failed' : task.result?.autoStart === 'running' ? 'running' : 'stopped') : task.status === 'failed' ? 'error' : 'installing',
      docker: {
        engine: 'running',
        image: completed ? 'present' : 'pulling',
        imageName: `nextcloud:${task.app.version}`,
        container: completed ? (activationFailed ? 'error' : task.result?.autoStart === 'running' ? 'running' : 'stopped') : 'not_created',
      },
      diagnostics: task.status === 'failed' ? [task.failure?.message ?? 'Installation failed.'] : [],
      spec: { dataDir: task.plan.options.dataDir, networkMode: task.plan.options.networkMode },
      settings: { dataDir: task.plan.options.dataDir },
      serviceInfo: { node: task.plan.options.targetNode, taskId: task.taskId },
      logs: [`Installation task ${task.taskId} created`],
      installProgress: task.progress ?? undefined,
      installTaskId: task.taskId,
    })
  }

  private persistTask() {
    if (this.activeTask) {
      window.localStorage.setItem(taskStorageKey, JSON.stringify(this.activeTask))
    }
  }

  private schedule(callback: () => void, delay: number) {
    const timer = window.setTimeout(() => {
      this.timers.delete(timer)
      callback()
    }, delay)
    this.timers.add(timer)
  }

  private emit() {
    this.revision += 1
    this.listeners.forEach((listener) => listener())
  }
}
