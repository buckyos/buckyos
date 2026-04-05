/* ── App Service mock store ── */

import type { AppServiceItem, InstallAppInfo } from './types'

const seedServices: AppServiceItem[] = [
  // ── App Layer ──
  {
    id: 'app-nostr-relay',
    name: 'Nostr Relay',
    description: 'Decentralized social relay service',
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
  },
  {
    id: 'app-filemanager',
    name: 'File Manager',
    description: 'Web-based file management service',
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
  },
  {
    id: 'app-gitea',
    name: 'Gitea',
    description: 'Self-hosted Git service',
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
      'Container startup failed: port 3000 is already in use.',
      'Check if another service is using port 3000, or change the port in settings.',
    ],
    spec: { port: '3000', sshPort: '2222', dataDir: '/data/gitea' },
    settings: { registrationEnabled: 'false', lfsEnabled: 'true' },
  },
  {
    id: 'app-photoprism',
    name: 'PhotoPrism',
    description: 'AI-powered photo management',
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
    diagnostics: ['Image is being pulled. Please wait for download to complete.'],
    spec: { port: '2342', dataDir: '/data/photoprism' },
    settings: {},
    installProgress: 45,
  },
  {
    id: 'app-home-assistant',
    name: 'Home Assistant',
    description: 'Smart home automation platform',
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
    settings: { timezone: 'Asia/Shanghai', language: 'zh-CN' },
  },
  {
    id: 'app-jellyfin',
    name: 'Jellyfin',
    description: 'Media streaming server',
    iconKey: 'studio',
    version: '10.8.13',
    layer: 'app',
    status: 'error',
    docker: {
      engine: 'running',
      image: 'missing',
      imageName: 'jellyfin/jellyfin:10.8.13',
      container: 'not_created',
    },
    diagnostics: [
      'Docker image not found: jellyfin/jellyfin:10.8.13',
      'The application image has not been downloaded. Try reinstalling or check network connectivity.',
    ],
    spec: { port: '8096', mediaDir: '/data/media' },
    settings: {},
  },
  // ── System Services Layer ──
  {
    id: 'sys-gateway',
    name: 'API Gateway',
    description: 'System API routing gateway',
    iconKey: 'diagnostics',
    version: '2.1.0',
    layer: 'system',
    status: 'running',
    docker: null,
    diagnostics: [],
    spec: { port: '443', protocol: 'https' },
    settings: {},
  },
  {
    id: 'sys-scheduler',
    name: 'Task Scheduler',
    description: 'System task scheduling service',
    iconKey: 'task-center',
    version: '1.0.2',
    layer: 'system',
    status: 'running',
    docker: null,
    diagnostics: [],
    spec: { maxConcurrent: '8', retryPolicy: 'exponential' },
    settings: {},
  },
  {
    id: 'sys-auth',
    name: 'Auth Service',
    description: 'Authentication and authorization',
    iconKey: 'users-agents',
    version: '1.4.0',
    layer: 'system',
    status: 'running',
    docker: null,
    diagnostics: [],
    spec: { tokenExpiry: '3600s', provider: 'local' },
    settings: {},
  },
  {
    id: 'sys-storage',
    name: 'Storage Manager',
    description: 'Distributed storage management',
    iconKey: 'files',
    version: '0.8.1',
    layer: 'system',
    status: 'running',
    docker: null,
    diagnostics: [],
    spec: { backend: 'local-fs', replication: '1' },
    settings: {},
  },
  // ── Kernel Layer ──
  {
    id: 'kernel-runtime',
    name: 'BuckyOS Runtime',
    description: 'Core system runtime',
    iconKey: 'settings',
    version: '0.5.0',
    layer: 'kernel',
    status: 'running',
    docker: null,
    diagnostics: [],
    spec: { arch: 'aarch64', platform: 'linux' },
    settings: {},
  },
  {
    id: 'kernel-network',
    name: 'Network Stack',
    description: 'System network management',
    iconKey: 'diagnostics',
    version: '0.5.0',
    layer: 'kernel',
    status: 'running',
    docker: null,
    diagnostics: [],
    spec: { interfaces: 'eth0, wlan0', dns: '1.1.1.1' },
    settings: {},
  },
]

export class AppServiceMockStore {
  services: AppServiceItem[] = [...seedServices]

  getAllServices(): AppServiceItem[] {
    return this.services
  }

  getByLayer(layer: AppServiceItem['layer']): AppServiceItem[] {
    return this.services.filter((s) => s.layer === layer)
  }

  getById(id: string): AppServiceItem | null {
    return this.services.find((s) => s.id === id) ?? null
  }

  startService(id: string): void {
    const svc = this.services.find((s) => s.id === id)
    if (svc && svc.status === 'stopped') {
      svc.status = 'starting'
      // Simulate startup
      setTimeout(() => {
        svc.status = 'running'
        if (svc.docker) {
          svc.docker.container = 'running'
        }
        svc.diagnostics = []
      }, 1500)
    }
  }

  stopService(id: string): void {
    const svc = this.services.find((s) => s.id === id)
    if (svc && (svc.status === 'running' || svc.status === 'starting')) {
      svc.status = 'stopped'
      if (svc.docker) {
        svc.docker.container = 'stopped'
      }
    }
  }

  parseInstallSource(_source: string): InstallAppInfo | null {
    // Mock: always returns a valid app info
    return {
      name: 'Nextcloud',
      version: '28.0.2',
      description: 'A self-hosted productivity platform with file sync, calendar, contacts, and more.',
      iconKey: 'files',
      permissions: [
        { label: 'File Access', description: 'Read/write access to /data/nextcloud' },
        { label: 'Network Access', description: 'Inbound connections on port 8443' },
        { label: 'Database', description: 'Create and manage PostgreSQL database' },
      ],
    }
  }

  installApp(info: InstallAppInfo): string {
    const id = `app-${info.name.toLowerCase().replace(/\s+/g, '-')}`
    const newService: AppServiceItem = {
      id,
      name: info.name,
      description: info.description,
      iconKey: info.iconKey,
      version: info.version,
      layer: 'app',
      status: 'installing',
      docker: {
        engine: 'running',
        image: 'pulling',
        imageName: `buckyos/${info.name.toLowerCase()}:${info.version}`,
        container: 'not_created',
      },
      diagnostics: ['Image is being pulled. Please wait for download to complete.'],
      spec: {},
      settings: {},
      installProgress: 0,
    }
    this.services.unshift(newService)

    // Simulate install progress
    let progress = 0
    const timer = setInterval(() => {
      progress += 20
      newService.installProgress = Math.min(progress, 100)
      if (progress >= 100) {
        clearInterval(timer)
        newService.status = 'running'
        newService.installProgress = undefined
        newService.diagnostics = []
        if (newService.docker) {
          newService.docker.image = 'present'
          newService.docker.container = 'running'
        }
      }
    }, 1000)

    return id
  }
}
