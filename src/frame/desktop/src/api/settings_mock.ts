/* Mock-runtime answers for the Settings app's control-panel reads (`?scenario=empty` gives a
 * freshly activated single-node Zone). Shapes follow the Rust handlers in control_panel. */

import type {
  BuckyOSDevConfig,
  BuckyOSInfo,
  GatewayFileResponse,
  GatewayOverviewResponse,
  ZoneCertificatesResponse,
  ZoneDevice,
  ZoneDevicesResponse,
  ZoneOverviewResponse,
} from './settings.ts'

const GB = 1024 ** 3
const HOST = 'alice.web3.buckyos.ai'

function scenario(): 'empty' | 'populated' {
  return new URLSearchParams(window.location.search).get('scenario') === 'empty' ? 'empty' : 'populated'
}

const nowSecs = () => Math.floor(Date.now() / 1000)

let devConfig: BuckyOSDevConfig = {
  schema_version: 1,
  enabled: true,
  enabled_at: nowSecs() - 3 * 86_400,
  enabled_by: 'alice',
}

function buckyosInfo(): BuckyOSInfo {
  return {
    schema_version: 1,
    version: '0.7.0',
    build_version: '0.7.0+20261012.a3f8c2d',
    release_channel: 'beta',
    target: 'x86_64-unknown-linux-gnu',
    installed_at: nowSecs() - 40 * 86_400,
    updated_at: nowSecs() - 2 * 86_400,
  }
}

function devices(): ZoneDevice[] {
  const ood1: ZoneDevice = {
    name: 'ood1',
    did: 'did:dev:T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8',
    device_type: 'ood',
    role: 'ood',
    is_ood: true,
    is_local: true,
    online: true,
    state: 'Ready',
    last_seen: nowSecs() - 12,
    hostname: 'alice-nas',
    arch: 'amd64',
    os: 'linux',
    os_version: 'Ubuntu 24.04.1 LTS',
    net_id: 'wan',
    ips: ['192.168.1.20', '2408:8207:1850:2a0::20'],
    endpoints: [
      { ip: '192.168.1.20', scope: 'private' },
      { ip: '2408:8207:1850:2a0::20', scope: 'global' },
    ],
    ipv6_state: 'available',
    rtcp_port: 2980,
    cpu: { model: 'AMD Ryzen 7 5825U with Radeon Graphics', cores: 16, mhz: 4546, usage_percent: 18.4 },
    memory: { total_bytes: 32 * GB, used_bytes: 13.4 * GB },
    storage: { total_bytes: 1863 * GB, used_bytes: 721 * GB },
    gpu: null,
    info_error: null,
  }
  if (scenario() === 'empty') return [ood1]
  return [
    ood1,
    {
      name: 'ood2',
      did: 'did:dev:qJ0c4n0yS1Yw8M5bT2r7xZkLpQe3HfVd9uGa6WmNs1E',
      device_type: 'ood',
      role: 'ood',
      is_ood: true,
      is_local: false,
      online: true,
      state: 'Ready',
      last_seen: nowSecs() - 25,
      hostname: 'alice-mini',
      arch: 'aarch64',
      os: 'linux',
      os_version: 'Debian GNU/Linux 12',
      net_id: 'lan1',
      ips: ['192.168.1.31'],
      endpoints: [{ ip: '192.168.1.31', scope: 'private' }],
      ipv6_state: 'unavailable',
      rtcp_port: 2980,
      cpu: { model: 'Rockchip RK3588', cores: 8, mhz: 2400, usage_percent: 6.2 },
      memory: { total_bytes: 16 * GB, used_bytes: 4.1 * GB },
      storage: { total_bytes: 3726 * GB, used_bytes: 1290 * GB },
      gpu: { model: 'Mali-G610', total_mem_bytes: null, used_mem_bytes: null, load_percent: 0, tflops: 0.45 },
      info_error: null,
    },
    {
      name: 'laptop',
      did: 'did:dev:Zr8x1QwLm2Nc5Vb7Kp3Ty6Ud4Hs9Ge0Af1Jo2Wi3Eq',
      device_type: 'node',
      role: 'node',
      is_ood: false,
      is_local: false,
      online: false,
      state: 'Ready',
      last_seen: nowSecs() - 5 * 3600,
      hostname: 'alice-mbp',
      arch: 'aarch64',
      os: 'apple',
      os_version: 'macOS 15.4',
      net_id: null,
      ips: ['192.168.1.52'],
      endpoints: [{ ip: '192.168.1.52', scope: 'private' }],
      ipv6_state: 'unavailable',
      rtcp_port: 2980,
      cpu: { model: 'Apple M3 Pro', cores: 12, mhz: 4050, usage_percent: null },
      memory: { total_bytes: 36 * GB, used_bytes: null },
      storage: { total_bytes: 1000 * GB, used_bytes: 412 * GB },
      gpu: { model: 'Apple M3 Pro GPU', total_mem_bytes: null, used_mem_bytes: null, load_percent: null, tflops: 7.4 },
      info_error: null,
    },
  ]
}

function devicesResponse(): ZoneDevicesResponse {
  return {
    zone_did: 'did:bns:alice',
    hostname: HOST,
    owner_did: 'did:bns:alice',
    sn: 'sn.buckyos.ai',
    local_device: 'ood1',
    online_window_secs: 120,
    devices: devices(),
  }
}

const MOCK_PEM = `-----BEGIN CERTIFICATE-----
MIIDmjCCAyCgAwIBAgISBk2mock0nly0for0display0purposes0AKBggqhkjOPQQD
AzAyMQswCQYDVQQGEwJVUzEWMBQGA1UEChMNTGV0J3MgRW5jcnlwdDELMAkGA1UE
AxMCRTYwHhcNMjYwOTE0MDAwMDAwWhcNMjYxMjEzMjM1OTU5WjAdMRswGQYDVQQD
-----END CERTIFICATE-----
`

function certificatesResponse(): ZoneCertificatesResponse {
  const issued = new Date(Date.now() - 29 * 86_400_000)
  const expires = new Date(issued.getTime() + 90 * 86_400_000)
  const days = Math.floor((expires.getTime() - Date.now()) / 86_400_000)
  const entry = (host: string, match: string) => ({
    host,
    match_type: match,
    source: 'acme',
    issuer: 'C=US, O=Let\'s Encrypt, CN=E6',
    subject: `CN=${host}`,
    serial_number: '064d9a6b3c1f2e0a7b8c9d0e1f2a3b4c5d6e',
    not_before: issued.toISOString(),
    not_after: expires.toISOString(),
    days_remaining: days,
    dns_names: [host],
    fingerprint_sha256: '3F:9A:1C:7E:52:B0:4D:88:21:6C:E3:0F:9B:47:A2:5D:1E:C8:73:06:BB:94:2F:60:D5:18:7A:E9:43:0C:6B:21',
    last_renewed_at: issued.toISOString(),
    state: 'valid' as const,
    cert_pem: MOCK_PEM,
  })
  return { hostname: HOST, node: 'ood1', certificates: [entry(HOST, 'exact'), entry(`*.${HOST}`, 'exact')] }
}

function zoneOverview(): ZoneOverviewResponse {
  return {
    etcDir: '/opt/buckyos/etc',
    zone: { name: 'alice', domain: HOST, did: 'did:bns:alice', ownerDid: 'did:bns:alice', userName: 'alice', zoneIat: nowSecs() - 40 * 86_400 },
    device: { name: 'ood1', did: devices()[0].did ?? '', type: 'ood', netId: 'wan' },
    sn: {
      url: 'https://sn.buckyos.ai/kapi/sn',
      username: 'alice',
      host: 'sn.buckyos.ai',
      ip: '203.0.113.10',
      dnsARecords: ['203.0.113.10'],
      dnsTxtRecords: ['"DID=did:bns:alice;"', '"PKX=T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8;"'],
      digError: '',
      selfCertState: true,
      selfCertStateSource: '/opt/buckyos/data/cyfs_gateway/sn_dns/self_cert_state.json',
    },
    notes: [],
  }
}

const GATEWAY_FILES: Record<string, string> = {
  'cyfs_gateway.json': JSON.stringify({ includes: [{ path: 'boot_gateway.yaml' }, { path: 'node_gateway.json' }, { path: 'user_gateway.yaml' }, { path: 'post_gateway.yaml' }] }, null, 2),
  'boot_gateway.yaml': 'stacks:\n  zone_gateway_http:\n    bind: 0.0.0.0:80\n    protocol: tcp\n  node_rtcp:\n    bind: 0.0.0.0:2980\n    protocol: rtcp\n',
  'node_gateway.json': JSON.stringify({ stacks: { zone_tls: { bind: '[::]:443', protocol: 'tls', hosts: [`*.${HOST}`, HOST] } } }, null, 2),
  'node_gateway_params.json': JSON.stringify({ zone_hostname: HOST, node_id: 'ood1' }, null, 2),
  'user_gateway.yaml': '--- {}\n',
  'post_gateway.yaml': '--- {}\n',
}

function gatewayOverview(): GatewayOverviewResponse {
  const modifiedAt = new Date(Date.now() - 86_400_000).toISOString()
  return {
    mode: 'sn',
    etcDir: '/opt/buckyos/etc',
    files: Object.entries(GATEWAY_FILES).map(([name, content]) => ({
      name,
      path: `/opt/buckyos/etc/${name}`,
      exists: true,
      sizeBytes: content.length,
      modifiedAt,
    })),
    tlsDomains: [HOST, `*.${HOST}`],
    notes: [],
  }
}

function gatewayFile(name: unknown): GatewayFileResponse {
  const key = String(name ?? '')
  const content = GATEWAY_FILES[key]
  if (content === undefined) throw new Error(`Unsupported gateway config file: ${key}`)
  return { name: key, path: `/opt/buckyos/etc/${key}`, sizeBytes: content.length, modifiedAt: new Date().toISOString(), content }
}

const executorDevices = new Map<string, Record<string, unknown>>()

const LOG_SERVICES = ['control-panel', 'cyfs-gateway', 'msg-center', 'node-daemon', 'scheduler', 'system-config', 'task-manager']

function handle(method: string, params: Record<string, unknown>): unknown {
  switch (method) {
    case 'executor.device.list':
      return { devices: [...executorDevices.values()].map((device) => ({ device, online: null })) }
    case 'executor.device.create':
    case 'executor.device.issue': {
      const id = method === 'executor.device.create' ? crypto.randomUUID() : String(params.executor_id)
      const device = method === 'executor.device.create'
        ? { ...params, executor_id: id, owner_user_id: 'alice', enabled: true, generation: 1, created_at_ms: Date.now() }
        : { ...executorDevices.get(id), enabled: true, expires_at_ms: params.expires_at_ms ?? null }
      executorDevices.set(id, device)
      const access_url = `wss://${HOST}/kapi/control-panel/executor/manage?executor_id=${id}#mock-credential-for-desktop-preview-only`
      return { device, access_url, start_command: `./start-buckyos-agent-daemon.sh '${access_url}'` }
    }
    case 'executor.device.revoke': {
      const id = String(params.executor_id)
      const device = executorDevices.get(id)
      if (device) executorDevices.set(id, { ...device, enabled: false })
      return {}
    }
    case 'executor.device.delete':
      executorDevices.delete(String(params.executor_id))
      return {}
    case 'system.buckyos_info.get':
      return buckyosInfo()
    case 'system.dev_mode.get':
      return { ...devConfig }
    case 'system.dev_mode.set':
      devConfig = { ...devConfig, enabled: params.enabled === true, ...(params.enabled === true ? { enabled_at: nowSecs(), enabled_by: 'alice' } : {}) }
      return { ...devConfig }
    case 'zone.devices.list':
      return devicesResponse()
    case 'zone.certificates.list':
      return certificatesResponse()
    case 'zone.overview':
      return zoneOverview()
    case 'gateway.overview':
      return gatewayOverview()
    case 'gateway.file.get':
      return gatewayFile(params.name)
    case 'system.logs.list':
      return { services: LOG_SERVICES.map((id) => ({ id, label: id })) }
    case 'system.logs.download': {
      const services = Array.isArray(params.services) ? params.services.map(String) : []
      const body = services.map((service) => `[${service}] mock log since ${String(params.since)}`).join('\n')
      return { url: URL.createObjectURL(new Blob([body], { type: 'text/plain' })), expiresInSec: 600, filename: 'buckyos-logs-mock.txt' }
    }
    default:
      throw new Error(`Unknown method: ${method}`)
  }
}

export async function callMockSettings<T>(
  method: string,
  params: Record<string, unknown>,
): Promise<{ data: T | null; error: unknown }> {
  await new Promise((resolve) => window.setTimeout(resolve, 80))
  try {
    return { data: handle(method, params) as T, error: null }
  } catch (error) {
    return { data: null, error }
  }
}
