/* Mock-runtime answers for the Settings app's control-panel reads (`?scenario=empty` gives a
 * freshly activated single-node Zone). Shapes follow the Rust handlers in control_panel. */

import type { bucket } from 'buckyos'
import type {
  BuckyOSDevConfig,
  BuckyOSInfo,
  ExtToolsCatalogEntry,
  ExtToolsNodeStatus,
  ExtToolsOnlineStatus,
  ExtToolsStatusResponse,
  GatewayFileResponse,
  GatewayOverviewResponse,
  ZoneCertificatesResponse,
  ZoneDevice,
  ZoneDevicesResponse,
  ZoneOverviewResponse,
  GarbageReport,
  GcRecord,
  GcStatus,
  StorageOverview,
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
    mode: 'default',
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

const MB = 1024 ** 2

function mockBucket(
  appid: string,
  name: string,
  data_policy: bucket.DataPolicy,
  unique_bytes: number,
  key_count: number,
  orphaned = false,
): bucket.BucketInfo {
  return {
    bucket_id: `user/${appid}/alice/${name}`,
    kind: 'data',
    data_policy,
    scope: 'user',
    appid,
    owner: 'alice',
    name,
    versioning: data_policy === 'documents' || data_policy === 'collected',
    unique_bytes: Math.round(unique_bytes),
    key_bytes: Math.round(unique_bytes),
    key_count,
    version_count: key_count,
    snapshot_only_bytes: 0,
    ...(orphaned ? { orphaned_at: Date.now() - 6 * 86_400_000 } : {}),
    created_at: Date.now() - 30 * 86_400_000,
  }
}

let storageBuckets: bucket.BucketInfo[] | null = null

function mockBuckets(): bucket.BucketInfo[] {
  storageBuckets ??=
    scenario() === 'empty'
      ? []
      : [
          mockBucket('control-panel', 'wallpapers', 'app_data', 9.4 * MB, 2),
          mockBucket('jarvis.alice.web3.buckyos.ai', 'artifacts', 'documents', 412 * MB, 38),
          mockBucket('jarvis.alice.web3.buckyos.ai', 'scratch', 'temp', 56 * MB, 11),
          mockBucket('aicc', 'outputs', 'temp', 23 * MB, 7),
          mockBucket('msg-center', 'collected', 'collected', 1.2 * GB, 214),
          mockBucket('notes.alice.bns.did', 'notes', 'documents', 3.1 * MB, 64, true),
          mockBucket('notes.alice.bns.did', 'index', 'app_data', 18 * MB, 1, true),
        ]
  return storageBuckets
}

export function mockStorageOverview(): StorageOverview {
  return { user: 'alice', buckets: mockBuckets().map((item) => ({ ...item })) }
}

export function deleteMockStorageBucket(bucketId: string): { deleted: boolean } {
  storageBuckets = mockBuckets().filter((item) => item.bucket_id !== bucketId)
  return { deleted: true }
}

let garbageBytes = 184 * MB
let gcRecords: GcRecord[] | null = null

function mockGcRecords(): GcRecord[] {
  gcRecords ??= [
    { at: Date.now() - 42 * 60_000, trigger: 'background', freed_bytes: 12 * MB, evicted_objects: 9, evicted_chunks: 14, pins_expired: 0 },
    { at: Date.now() - 26 * 3_600_000, trigger: 'background', freed_bytes: 0, evicted_objects: 0, evicted_chunks: 0, pins_expired: 2 },
  ]
  return gcRecords
}

export function mockStorageGcStatus(): GcStatus {
  return { evict_enabled: true, last_round_at: Date.now() - 12_000, rounds: 3120, last_error: null, recent: mockGcRecords().map((item) => ({ ...item })) }
}

/** The first run frees what is there, the next finds nothing. */
export function collectMockStorageGarbage(): GarbageReport {
  const freed = garbageBytes
  garbageBytes = 0
  const report = { freed_bytes: freed, evicted_objects: freed ? 37 : 0, evicted_chunks: freed ? 52 : 0, pins_expired: 0 }
  gcRecords = [{ at: Date.now(), trigger: 'manual' as const, ...report }, ...mockGcRecords()].slice(0, 3)
  return { ...report, rounds: freed ? 2 : 1, complete: true }
}

const LOG_SERVICES = ['control-panel', 'cyfs-gateway', 'msg-center', 'node-daemon', 'scheduler', 'system-config', 'task-manager']

function handle(method: string, params: Record<string, unknown>): unknown {
  switch (method) {
    case 'system.buckyos_info.get':
      return buckyosInfo()
    case 'system.dev_mode.get':
      return { ...devConfig }
    case 'system.dev_mode.set':
      devConfig = { ...devConfig, enabled: params.enabled === true, ...(params.enabled === true ? { enabled_at: nowSecs(), enabled_by: 'alice' } : {}) }
      return { ...devConfig }
    case 'ext_tools.status':
      return extToolsStatus()
    case 'ext_tools.plan':
      return extToolsPlan(String(params.id), params.enabled === true)
    case 'ext_tools.set_desired':
      extToolsDesired[String(params.id)] = { version: extToolsCatalog[String(params.id)]?.manifest.version ?? '', enabled: params.enabled === true }
      return { ok: true, plan: extToolsPlan(String(params.id), params.enabled === true) }
    case 'ext_tools.download':
      return extToolsDownload(String(params.id), String(params.item), typeof params.node === 'string' ? params.node : undefined)
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

const CHROMIUM_SIZE = 208_987_836

const extToolsCatalog: Record<string, ExtToolsCatalogEntry> = {
  'ext-core': {
    manifest: {
      id: 'ext-core',
      version: '1.3.0',
      desc: 'The shared local tools of every Agent: Python, Node, ffmpeg, pandoc and the Agent browser',
      provides: [{ command: 'python3' }, { command: 'ffmpeg' }, { command: 'pandoc' }, { command: 'agent-browser' }, { command: 'chromium', needs: ['chromium'] }, { command: 'page-render', needs: ['chromium'] }],
      platforms: ['linux-x86_64', 'linux-aarch64'],
      size_bytes: { 'linux-x86_64': 720_800_000, 'linux-aarch64': 702_000_000 },
      online: [{
        id: 'chromium',
        version: '159.0.8154.6',
        desc: 'Chrome for Testing: the browser of the Agent Browser, of page tests and of page-render',
        license: 'Chrome for Testing, downloaded by each node from Google\'s official address (storage.googleapis.com/chrome-for-testing-public) and used under its own terms; BuckyOS does not distribute it.',
        auto: true,
        platforms: { 'linux-x86_64': { size_bytes: CHROMIUM_SIZE }, 'linux-aarch64': { size_bytes: 199_000_000 } },
      }],
    },
    pikg_digest: 'sha256:1',
    content_id: 'file:1',
  },
  'ext-media': {
    manifest: { id: 'ext-media', version: '1.2.0', desc: 'Audio and video editing', requires: [{ id: 'ext-core', version: '>=1.0' }], provides: [{ command: 'ffmpeg' }, { command: 'ffprobe' }], platforms: ['linux-x86_64'] },
    pikg_digest: 'sha256:0',
    content_id: 'file:0',
  },
}
const extToolsDesired: Record<string, { version: string; enabled: boolean }> = {}

function extToolsPlan(id: string, enabled: boolean) {
  if (!enabled) return { enable: [], disable: [id], errors: [] }
  return { enable: extToolsCatalog[id] ? [extToolsCatalog[id]] : [], disable: [], errors: extToolsCatalog[id] ? [] : [`${id} is not in the Repo`] }
}

// ── Online items: what ext-core only declares and the node downloads (chromium), with a download that takes a few seconds ──

/** The node takes a request on its next round; the download then runs this long. */
const MOCK_PICKUP_MS = 700
const MOCK_DOWNLOAD_MS = 4000

interface MockOnline {
  status: ExtToolsOnlineStatus
  /** A download asked for: under way from `start`, ending with `failure` or ready. */
  job?: { start: number; failure: string | null }
}

const readyChromium = (at: number): ExtToolsOnlineStatus => ({ state: 'ready', version: '159.0.8154.6', size_bytes: CHROMIUM_SIZE, downloaded_bytes: CHROMIUM_SIZE, updated_at_ms: at })

const extToolsOnline: Record<string, Record<string, MockOnline>> = {
  ood1: { chromium: { status: readyChromium(Date.now() - 3 * 86_400_000) } },
}
/** The next download of an item fails with this reason. */
const failNext: Record<string, string> = {}

function onlineNow(entry: MockOnline): ExtToolsOnlineStatus {
  const { job, status } = entry
  if (!job || Date.now() < job.start) return status
  const fraction = Math.min(1, (Date.now() - job.start) / MOCK_DOWNLOAD_MS)
  if (job.failure && fraction >= 0.4) {
    entry.job = undefined
    // Fetching again what is in place and failing keeps it in place.
    entry.status = status.state === 'ready' ? { ...status, error: job.failure } : { ...status, state: 'failed', downloaded_bytes: 0, error: job.failure, updated_at_ms: Date.now() }
    return entry.status
  }
  if (fraction >= 1) {
    entry.job = undefined
    entry.status = { state: 'ready', version: status.version, size_bytes: status.size_bytes, downloaded_bytes: status.size_bytes, updated_at_ms: Date.now() }
    return entry.status
  }
  const step = Math.max(1, Math.floor(status.size_bytes / 100))
  return { state: 'downloading', version: status.version, size_bytes: status.size_bytes, downloaded_bytes: Math.floor((status.size_bytes * fraction) / step) * step, updated_at_ms: Date.now() }
}

/** Where online item `item` stands on `node` now; the mock Agent Service reads it to tell whether its browser is there. */
export function mockOnlineStatus(node: string, item: string): ExtToolsOnlineStatus | null {
  const entry = extToolsOnline[node]?.[item]
  return entry ? onlineNow(entry) : null
}

/** Whether the node has a copy of the item in place: downloading it again leaves the one in use there until that ends. */
export function mockOnlineInPlace(node: string, item: string): boolean {
  const entry = extToolsOnline[node]?.[item]
  if (!entry) return false
  onlineNow(entry)
  return entry.status.state === 'ready'
}

/** Puts an online item in a state, as if the node had got there (`window.__extToolsMock.setOnline`). */
export function setMockOnline(node: string, item: string, state: 'missing' | 'failed' | 'ready', error = 'connection to storage.googleapis.com timed out') {
  const base = extToolsOnline[node]?.[item]?.status ?? readyChromium(0)
  const status: ExtToolsOnlineStatus = state === 'ready'
    ? { ...readyChromium(Date.now()), version: base.version, size_bytes: base.size_bytes, downloaded_bytes: base.size_bytes }
    : state === 'failed'
      ? { state, version: base.version, size_bytes: base.size_bytes, downloaded_bytes: 0, error, updated_at_ms: Date.now() - 60_000 }
      : { state, version: base.version, size_bytes: base.size_bytes, downloaded_bytes: 0, updated_at_ms: 0 }
  extToolsOnline[node] = { ...extToolsOnline[node], [item]: { status } }
}

function extToolsDownload(id: string, item: string, node?: string) {
  const having = Object.keys(extToolsOnline).filter((name) => extToolsOnline[name][item])
  if (node && !having.includes(node)) throw new Error(`node ${node} has no online item ${item} of ${id}`)
  if (having.length === 0) throw new Error(`no node has an online item ${item} of ${id}`)
  const nodes = node ? [node] : having
  for (const name of nodes) {
    const entry = extToolsOnline[name][item]
    // A node in the middle of that download answers with the one under way.
    if (onlineNow(entry).state === 'downloading') continue
    entry.job = { start: Date.now() + MOCK_PICKUP_MS, failure: failNext[item] ?? null }
    delete failNext[item]
  }
  return { ok: true, id, item, request_ms: Date.now(), nodes }
}

function coreOnNode(node: string): ExtToolsNodeStatus {
  const online = Object.fromEntries(Object.entries(extToolsOnline[node] ?? {}).map(([item, entry]) => [item, onlineNow(entry)]))
  const waiting = Object.fromEntries(extToolsCatalog['ext-core'].manifest.provides
    .map((p) => [p.command, (p.needs ?? []).filter((item) => online[item] && online[item].state !== 'ready')] as const)
    .filter(([, items]) => items.length > 0))
  return { version: '1.3.0', stage: 'active', targets: { native: { stage: 'active', version: '1.3.0', online, waiting } }, online }
}

function extToolsStatus(): ExtToolsStatusResponse {
  const active = (version: string) => ({ version, stage: 'active' as const })
  return {
    nodes: [{ id: 'ood1', platform: 'linux-x86_64', containers: true }],
    packages: [
      { id: 'ext-core', latest: extToolsCatalog['ext-core'], versions: ['1.3.0'], desired: null, nodes: { ood1: coreOnNode('ood1') }, skills: { ffmpeg: ['aigc_video'], chromium: ['browser'] } },
      {
        id: 'ext-media',
        latest: extToolsCatalog['ext-media'],
        versions: ['1.2.0'],
        desired: extToolsDesired['ext-media'] ?? null,
        nodes: extToolsDesired['ext-media']?.enabled ? { ood1: active('1.2.0') } : null,
      },
    ],
  }
}

Object.assign(window, {
  __extToolsMock: {
    setOnline: setMockOnline,
    failNextDownload(item: string, error: string) {
      failNext[item] = error
    },
  },
})
