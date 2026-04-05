import type {
  SettingsStoreSnapshot,
  GeneralInfo,
  SessionConfig,
  ClusterInfo,
  PrivacyInfo,
  DeveloperInfo,
} from './types'

/* ------------------------------------------------------------------ */
/*  General                                                            */
/* ------------------------------------------------------------------ */

const generalInfo: GeneralInfo = {
  software: {
    version: '1.0.0-rc.3',
    buildVersion: 'build-20260401-a3f8c2d',
    releaseChannel: 'beta',
    lastUpdateTime: '2026-03-28T14:30:00Z',
    updateAvailable: true,
    latestVersion: '1.0.0-rc.4',
    autoUpdate: false,
  },
  device: {
    osType: 'macOS',
    osVersion: 'macOS 15.4',
    cpuModel: 'Apple M3 Pro',
    cpuCores: 12,
    totalMemory: '36 GB',
    totalStorage: '1 TB',
  },
  snapshot: {
    installMode: 'desktop',
    nodeCount: 1,
    storageUsed: '256 GB',
    storageTotal: '1 TB',
    enabledModules: ['File Manager', 'AI Center', 'Message Hub', 'Code Assistant'],
  },
}

/* ------------------------------------------------------------------ */
/*  Session / Appearance                                               */
/* ------------------------------------------------------------------ */

const sessionConfig: SessionConfig = {
  session: {
    sessionId: 'session_d8a3f2c1',
    name: "Leo's MacBook Pro",
    type: 'shared',
    deviceId: null,
    environment: 'desktop',
  },
  appearance: {
    theme: 'dark',
    language: 'en',
    fontSize: 'medium',
    wallpaper: 'wallpaper_01',
  },
  desktop: {
    layout: {},
    windowState: {},
  },
}

/* ------------------------------------------------------------------ */
/*  Cluster Manager                                                    */
/* ------------------------------------------------------------------ */

const clusterInfo: ClusterInfo = {
  overview: {
    clusterMode: 'single_node',
    nodeCount: 1,
    zoneCount: 1,
    activeZone: 'did:bns:leo.buckyos',
  },
  nodes: [
    {
      deviceName: "Leo's MacBook Pro",
      deviceId: 'node-001-m3pro',
      status: 'online',
      zone: 'did:bns:leo.buckyos',
    },
  ],
  zones: [
    {
      zoneDID: 'did:bns:leo.buckyos',
      didMethod: 'did:bns',
      ownerDID: 'did:bns:leo',
      name: "Leo's Zone",
      description: 'Primary personal zone',
    },
  ],
  connectivity: {
    domainType: 'bns_subdomain',
    domain: 'leo.buckyos.io',
    snRelay: true,
    snRegion: 'Asia Pacific (Hong Kong)',
    snTrafficUsed: '12.4 GB',
    snTrafficTotal: '100 GB',
    dnsInfo: 'A: 203.0.113.42 | AAAA: —',
    ipv4: true,
    ipv6: false,
    directConnect: false,
    portMapping: false,
    portMappingDetails: null,
  },
  certificates: [
    {
      source: 'auto',
      domain: 'leo.buckyos.io',
      issuer: "Let's Encrypt Authority X3",
      expiryDate: '2026-06-28T00:00:00Z',
      valid: true,
      x509Raw: '-----BEGIN CERTIFICATE-----\nMIIFjTCCA3WgAwIBAgISA0...(truncated for display)\n-----END CERTIFICATE-----',
    },
  ],
}

/* ------------------------------------------------------------------ */
/*  Privacy                                                            */
/* ------------------------------------------------------------------ */

const privacyInfo: PrivacyInfo = {
  publicAccess: [
    {
      domain: 'public.leo.buckyos.io',
      label: 'Public Site',
      description: 'Accessible without authentication. Anyone on the internet can visit this address.',
      isPublic: true,
      enabled: true,
    },
    {
      domain: 'home.leo.buckyos.io',
      label: 'Home (Coming Soon)',
      description: 'A private portal for friends and family. Not yet available.',
      isPublic: false,
      enabled: false,
    },
  ],
  messagingAccess: {
    enabled: true,
    canToggle: false,
    description: 'Allows others to send you messages directly. This is required for core messaging features to work.',
  },
  dataVisibility: [
    {
      folderName: 'Public Folder',
      visibility: 'public',
      description: 'Files here are visible to anyone on the internet.',
      icon: 'Globe',
    },
    {
      folderName: 'Shared Folder',
      visibility: 'shared',
      description: 'Files here are visible to other users within your system.',
      icon: 'Users',
    },
    {
      folderName: 'Private Data',
      visibility: 'private',
      description: 'Only you can access these files. Not shared with anyone.',
      icon: 'Lock',
    },
  ],
  appAccess: [
    {
      id: 'app-file-browser',
      name: 'File Browser',
      type: 'system',
      riskLevel: 'trusted',
      accessScope: 'Full file system access',
      multiUser: false,
      extraPermissions: ['Home partition access'],
      description: 'System file browser with extended access to manage all your files.',
    },
    {
      id: 'app-ai-center',
      name: 'AI Center',
      type: 'system',
      riskLevel: 'trusted',
      accessScope: 'User data (current user only)',
      multiUser: false,
      extraPermissions: [],
      description: 'System AI management. Accesses your data to provide AI features.',
    },
    {
      id: 'app-message-hub',
      name: 'Message Hub',
      type: 'system',
      riskLevel: 'trusted',
      accessScope: 'User messages and contacts',
      multiUser: false,
      extraPermissions: [],
      description: 'Core messaging application.',
    },
    {
      id: 'app-code-assistant',
      name: 'Code Assistant',
      type: 'third_party',
      riskLevel: 'elevated',
      accessScope: 'Project files and workspace',
      multiUser: true,
      extraPermissions: ['Workspace file read/write'],
      description: 'Third-party code assistance tool. Shared installation with multi-user access.',
    },
    {
      id: 'agent-research',
      name: 'Research Agent',
      type: 'agent',
      riskLevel: 'elevated',
      accessScope: 'User documents and web access',
      multiUser: false,
      extraPermissions: ['Internet access', 'Document read'],
      description: 'AI agent that can browse the web and read your documents to complete research tasks.',
    },
  ],
  devicePermissions: [
    {
      id: 'perm-camera-1',
      appName: 'Message Hub',
      capability: 'camera',
      accessType: 'direct',
      granted: true,
      description: 'Camera access for video calls',
    },
    {
      id: 'perm-mic-1',
      appName: 'Message Hub',
      capability: 'microphone',
      accessType: 'direct',
      granted: true,
      description: 'Microphone access for voice and video calls',
    },
    {
      id: 'perm-iot-1',
      appName: 'Home Assistant',
      capability: 'iot_camera',
      accessType: 'data_routed',
      granted: true,
      description: 'IoT camera data routed through shared directory',
    },
    {
      id: 'perm-docker-1',
      appName: 'Home Assistant',
      capability: 'docker',
      accessType: 'direct',
      granted: true,
      description: 'Docker container runtime access',
    },
  ],
}

/* ------------------------------------------------------------------ */
/*  Developer Mode                                                     */
/* ------------------------------------------------------------------ */

const developerInfo: DeveloperInfo = {
  modeEnabled: true,
  readOnly: true,
  diagnostics: [
    { name: 'System Services', status: 'pass', message: 'All 12 services running', detail: 'gateway, auth, storage, message-hub, ai-router, scheduler, dns-manager, cert-manager, sn-proxy, file-service, app-manager, monitor' },
    { name: 'Network Connectivity', status: 'pass', message: 'SN relay active, latency 42ms' },
    { name: 'Certificate Status', status: 'pass', message: 'Valid, expires in 85 days' },
    { name: 'Storage Health', status: 'pass', message: '256 GB used of 1 TB (25.6%)' },
    { name: 'DNS Resolution', status: 'warn', message: 'IPv6 AAAA record not configured', detail: 'IPv6 is available on this device but no AAAA record exists. Consider adding one for better connectivity.' },
    { name: 'Port Mapping', status: 'warn', message: 'UPnP not available, using SN relay', detail: 'Direct connections are not possible. All traffic is routed through the SN relay node.' },
    { name: 'Backup Status', status: 'fail', message: 'No backup configured', detail: 'No backup target has been configured. Data exists only on this device.' },
  ],
  configTree: [
    {
      key: 'system_config',
      label: 'SystemConfig',
      type: 'folder',
      children: [
        {
          key: 'system_config.identity',
          label: 'Identity',
          type: 'file',
          content: JSON.stringify({
            zone_did: 'did:bns:leo.buckyos',
            owner_did: 'did:bns:leo',
            did_method: 'did:bns',
            display_name: "Leo's BuckyOS",
          }, null, 2),
        },
        {
          key: 'system_config.network',
          label: 'Network',
          type: 'file',
          content: JSON.stringify({
            domain: 'leo.buckyos.io',
            sn_relay: true,
            sn_region: 'ap-hk',
            ipv4: '203.0.113.42',
            ipv6: null,
            port_mapping: false,
          }, null, 2),
        },
        {
          key: 'system_config.services',
          label: 'Services',
          type: 'file',
          content: JSON.stringify({
            enabled: ['gateway', 'auth', 'storage', 'message-hub', 'ai-router', 'scheduler', 'dns-manager', 'cert-manager', 'sn-proxy', 'file-service', 'app-manager', 'monitor'],
            auto_start: true,
          }, null, 2),
        },
        {
          key: 'system_config.storage',
          label: 'Storage',
          type: 'file',
          content: JSON.stringify({
            data_dir: '/var/buckyos/data',
            cache_dir: '/var/buckyos/cache',
            temp_dir: '/tmp/buckyos',
            max_cache_size: '10GB',
          }, null, 2),
        },
      ],
    },
    {
      key: 'gateway_config',
      label: 'Gateway Config',
      type: 'folder',
      children: [
        {
          key: 'gateway_config.main',
          label: 'Main',
          type: 'file',
          content: JSON.stringify({
            listen_port: 443,
            http_redirect: true,
            cors_origins: ['*'],
            rate_limit: { enabled: true, requests_per_minute: 60 },
            tls: { cert_path: '/etc/buckyos/certs/fullchain.pem', key_path: '*** Hidden for security ***' },
          }, null, 2),
        },
        {
          key: 'gateway_config.routes',
          label: 'Routes',
          type: 'file',
          content: JSON.stringify({
            routes: [
              { path: '/api/*', upstream: 'localhost:8080' },
              { path: '/public/*', upstream: 'localhost:8081', auth: false },
              { path: '/ws/*', upstream: 'localhost:8082', websocket: true },
            ],
          }, null, 2),
        },
      ],
    },
    {
      key: 'local_config',
      label: 'Local Config',
      type: 'file',
      content: JSON.stringify({
        auto_start: true,
        startup_delay: 0,
        log_level: 'info',
        log_retention_days: 30,
        dev_mode: false,
      }, null, 2),
    },
  ],
  cliHelpers: [
    { command: 'buckyos status', description: 'Show current system status and running services' },
    { command: 'buckyos restart', description: 'Restart all BuckyOS services' },
    { command: 'buckyos logs', description: 'View recent system logs' },
    { command: 'buckyos logs --export', description: 'Export logs as a compressed archive' },
    { command: 'buckyos doctor', description: 'Run system diagnostics and health checks' },
    { command: 'buckyos config show', description: 'Display current system configuration' },
    { command: 'buckyos network test', description: 'Test network connectivity and SN relay' },
    { command: 'buckyos cert renew', description: 'Force certificate renewal' },
  ],
  logsAvailable: true,
  lastLogExport: '2026-03-25T10:00:00Z',
}

/* ------------------------------------------------------------------ */
/*  Seed builders                                                      */
/* ------------------------------------------------------------------ */

export function getPopulatedSeed(): SettingsStoreSnapshot {
  return {
    general: generalInfo,
    session: sessionConfig,
    cluster: clusterInfo,
    privacy: privacyInfo,
    developer: developerInfo,
  }
}

export function getEmptySeed(): SettingsStoreSnapshot {
  return {
    general: {
      ...generalInfo,
      software: { ...generalInfo.software, updateAvailable: false, latestVersion: null },
      snapshot: { ...generalInfo.snapshot, enabledModules: [] },
    },
    session: sessionConfig,
    cluster: {
      ...clusterInfo,
      nodes: [clusterInfo.nodes[0]],
      zones: [clusterInfo.zones[0]],
    },
    privacy: {
      ...privacyInfo,
      appAccess: privacyInfo.appAccess.filter((a) => a.type === 'system'),
      devicePermissions: [],
    },
    developer: {
      ...developerInfo,
      diagnostics: developerInfo.diagnostics.filter((d) => d.status === 'pass'),
    },
  }
}
