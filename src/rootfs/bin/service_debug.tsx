#!/usr/bin/env -S deno run --allow-env --allow-read --allow-net --allow-run

// service_debug.tsx 是一个 app_service 的 debug 工具。
// 目标是参考 node_daemon 的 app_loader，为手工调试 opendan/agent 服务
// 补齐 node_daemon 正常启动时会注入的关键环境变量，然后以前台方式启动。
//
// 当前实现优先覆盖 opendan(agent) 场景：
//   service_debug <app_service_name> <owner_user_id> [--port <port>] [--node-id <node_id>] [--detach]

type JsonValue =
  | string
  | number
  | boolean
  | null
  | JsonValue[]
  | { [key: string]: JsonValue }

type JsonObject = { [key: string]: JsonValue }

type StartupOptions = {
  appId: string
  ownerUserId: string
  nodeId?: string
  port?: number
  detach: boolean
  systemConfigUrl: string
}

const DEFAULT_BUCKYOS_ROOT = '/opt/buckyos'
const DEFAULT_OPENDAN_SERVICE_PORT = 4060
const OPENDAN_SERVICE_PORT_FALLBACK_KEYS = ['www', 'http', 'https', 'main']
const VERIFY_HUB_TOKEN_EXPIRE_TIME = 60 * 10

class RPCError extends Error {
  constructor(message: string) {
    super(message)
    this.name = 'RPCError'
  }
}

class KRPCClient {
  private seq = Date.now()

  constructor(
    private readonly serverUrl: string,
    private sessionToken: string,
  ) {}

  async call(method: string, params: JsonValue): Promise<JsonValue> {
    const seq = this.seq++
    const body = {
      method,
      params,
      sys: [seq, this.sessionToken],
    }
    const response = await fetch(this.serverUrl, {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
      },
      body: JSON.stringify(body),
    })
    if (!response.ok) {
      throw new RPCError(`RPC call ${method} failed with HTTP ${response.status}`)
    }

    const payload = await response.json()
    const sys = payload?.sys
    if (!Array.isArray(sys) || sys[0] !== seq) {
      throw new RPCError(`RPC response seq mismatch for ${method}`)
    }
    if (typeof sys[1] === 'string' && sys[1].length > 0) {
      this.sessionToken = sys[1]
    }
    if (payload?.error) {
      throw new RPCError(`RPC ${method} returned error: ${payload.error}`)
    }
    if (!('result' in payload)) {
      throw new RPCError(`RPC ${method} missing result field`)
    }
    return payload.result as JsonValue
  }
}

function printUsage(): never {
  console.error(
    [
      'Usage:',
      '  service_debug <app_service_name> <owner_user_id> [--port <port>] [--node-id <node_id>] [--detach]',
      '',
      'Example:',
      '  service_debug jarvis alice',
      '  service_debug jarvis alice --port 14060',
    ].join('\n'),
  )
  Deno.exit(1)
}

function parseArgs(args: string[]): StartupOptions {
  if (args.length < 2) {
    printUsage()
  }

  const appId = args[0]?.trim()
  const ownerUserId = args[1]?.trim()
  if (!appId || !ownerUserId) {
    printUsage()
  }

  let nodeId: string | undefined
  let port: number | undefined
  let detach = false
  let systemConfigUrl = 'http://127.0.0.1:3200/kapi/system_config'

  for (let index = 2; index < args.length; index += 1) {
    const arg = args[index]
    switch (arg) {
      case '--node-id': {
        nodeId = args[index + 1]?.trim()
        index += 1
        break
      }
      case '--port': {
        const raw = args[index + 1]?.trim()
        index += 1
        if (!raw) {
          throw new Error('missing value for --port')
        }
        const parsed = Number.parseInt(raw, 10)
        if (!Number.isInteger(parsed) || parsed <= 0 || parsed > 65535) {
          throw new Error(`invalid --port value: ${raw}`)
        }
        port = parsed
        break
      }
      case '--detach': {
        detach = true
        break
      }
      case '--system-config-url': {
        systemConfigUrl = args[index + 1]?.trim() || systemConfigUrl
        index += 1
        break
      }
      default: {
        throw new Error(`unknown argument: ${arg}`)
      }
    }
  }

  return {
    appId,
    ownerUserId,
    nodeId,
    port,
    detach,
    systemConfigUrl,
  }
}

function getBuckyosRoot(): string {
  return (Deno.env.get('BUCKYOS_ROOT') || DEFAULT_BUCKYOS_ROOT).trim() || DEFAULT_BUCKYOS_ROOT
}

function joinPath(...segments: string[]): string {
  return segments
    .filter((segment) => segment.length > 0)
    .map((segment, index) => {
      if (index === 0) {
        return segment.replace(/\/+$/g, '')
      }
      return segment.replace(/^\/+/g, '').replace(/\/+$/g, '')
    })
    .join('/')
}

async function fileExists(path: string): Promise<boolean> {
  try {
    await Deno.stat(path)
    return true
  } catch {
    return false
  }
}

async function readJsonFile(path: string): Promise<JsonObject> {
  const raw = await Deno.readTextFile(path)
  return JSON.parse(raw) as JsonObject
}

function uniquePkgName(pkgId: string): string {
  return pkgId.split('#', 1)[0].trim()
}

function getFullAppId(appId: string, ownerUserId: string): string {
  return `${ownerUserId}-${appId}`
}

function getSessionTokenEnvKey(appFullId: string, isAppService: boolean): string {
  const upper = appFullId.toUpperCase().replaceAll('-', '_')
  return isAppService ? `${upper}_TOKEN` : `${upper}_SESSION_TOKEN`
}

function getAppDataDir(buckyosRoot: string, appId: string, ownerUserId: string): string {
  return joinPath(buckyosRoot, 'data', 'home', ownerUserId, '.local', 'share', appId)
}

function normalizeServicePort(portValue: unknown): number | null {
  if (typeof portValue !== 'number' || !Number.isInteger(portValue)) {
    return null
  }
  if (portValue <= 0 || portValue > 65535) {
    return null
  }
  return portValue
}

function getNestedObject(root: JsonObject, path: string[]): JsonObject | undefined {
  let current: JsonValue = root
  for (const key of path) {
    if (!current || typeof current !== 'object' || Array.isArray(current)) {
      return undefined
    }
    current = (current as JsonObject)[key]
  }
  if (!current || typeof current !== 'object' || Array.isArray(current)) {
    return undefined
  }
  return current as JsonObject
}

function getNestedString(root: JsonObject, path: string[]): string | undefined {
  let current: JsonValue = root
  for (const key of path) {
    if (!current || typeof current !== 'object' || Array.isArray(current)) {
      return undefined
    }
    current = (current as JsonObject)[key]
  }
  return typeof current === 'string' && current.trim().length > 0 ? current.trim() : undefined
}

function getNestedNumber(root: JsonObject, path: string[]): number | undefined {
  let current: JsonValue = root
  for (const key of path) {
    if (!current || typeof current !== 'object' || Array.isArray(current)) {
      return undefined
    }
    current = (current as JsonObject)[key]
  }
  return typeof current === 'number' && Number.isFinite(current) ? current : undefined
}

function base64UrlDecode(input: string): Uint8Array {
  const normalized = input.replaceAll('-', '+').replaceAll('_', '/')
  const padded = normalized + '='.repeat((4 - (normalized.length % 4)) % 4)
  const raw = atob(padded)
  return Uint8Array.from(raw, (char) => char.charCodeAt(0))
}

function base64UrlEncode(input: Uint8Array): string {
  let raw = ''
  for (const byte of input) {
    raw += String.fromCharCode(byte)
  }
  return btoa(raw).replaceAll('+', '-').replaceAll('/', '_').replaceAll('=', '')
}

function decodeJwtPayload<T extends JsonObject>(jwt: string): T {
  const parts = jwt.split('.')
  if (parts.length < 2) {
    throw new Error('invalid jwt format')
  }
  const payloadBytes = base64UrlDecode(parts[1])
  const payloadText = new TextDecoder().decode(payloadBytes)
  return JSON.parse(payloadText) as T
}

function pemToDerBytes(pem: string): Uint8Array {
  const content = pem
    .replace(/-----BEGIN PRIVATE KEY-----/g, '')
    .replace(/-----END PRIVATE KEY-----/g, '')
    .replace(/\s+/g, '')
  return base64UrlDecode(content.replaceAll('+', '-').replaceAll('/', '_'))
}

async function importEd25519PrivateKeyFromPem(pem: string): Promise<CryptoKey> {
  const pkcs8 = pemToDerBytes(pem)
  const keyData = new Uint8Array(pkcs8.byteLength)
  keyData.set(pkcs8)
  return await crypto.subtle.importKey(
    'pkcs8',
    keyData,
    { name: 'Ed25519' },
    false,
    ['sign'],
  )
}

async function generateAppServiceToken(
  appId: string,
  subject: string,
  deviceName: string,
  privateKeyPem: string,
): Promise<string> {
  const now = Math.floor(Date.now() / 1000)
  const header = {
    alg: 'EdDSA',
    kid: deviceName,
    typ: 'JWT',
  }
  const payload = {
    token_type: 'Normal',
    appid: appId,
    jti: `${now}`,
    session: now,
    sub: subject,
    aud: null,
    exp: now + VERIFY_HUB_TOKEN_EXPIRE_TIME * 2,
    iss: deviceName,
    token: null,
    extra: {},
  }

  const encodedHeader = base64UrlEncode(new TextEncoder().encode(JSON.stringify(header)))
  const encodedPayload = base64UrlEncode(new TextEncoder().encode(JSON.stringify(payload)))
  const signingInput = new TextEncoder().encode(`${encodedHeader}.${encodedPayload}`)
  const privateKey = await importEd25519PrivateKeyFromPem(privateKeyPem)
  const signature = await crypto.subtle.sign('Ed25519', privateKey, signingInput)

  return `${encodedHeader}.${encodedPayload}.${base64UrlEncode(new Uint8Array(signature))}`
}

function selectAgentServicePort(
  appDoc: JsonObject,
  appInstanceConfig: JsonObject,
  portOverride?: number,
): number {
  if (portOverride) {
    return portOverride
  }

  const rawServicePorts = appInstanceConfig.service_ports_config
  const servicePorts =
    rawServicePorts && typeof rawServicePorts === 'object' && !Array.isArray(rawServicePorts)
      ? rawServicePorts as Record<string, number>
      : {}

  const preferredNames = new Set<string>()
  const configTips = getNestedObject(appDoc, ['install_config_tips', 'service_ports']) || {}
  for (const key of Object.keys(configTips)) {
    preferredNames.add(key)
  }
  for (const key of OPENDAN_SERVICE_PORT_FALLBACK_KEYS) {
    preferredNames.add(key)
  }

  for (const serviceName of preferredNames) {
    const port = normalizeServicePort(servicePorts[serviceName])
    if (port !== null) {
      return port
    }
  }

  const validPorts = Object.entries(servicePorts)
    .map(([serviceName, value]) => [serviceName, normalizeServicePort(value)] as const)
    .filter(([, value]) => value !== null)
    .map(([serviceName, value]) => [serviceName, value as number] as const)

  if (validPorts.length > 0) {
    validPorts.sort(([lhs], [rhs]) => lhs.localeCompare(rhs))
    return validPorts[0][1]
  }

  return DEFAULT_OPENDAN_SERVICE_PORT
}

function buildFallbackDeviceInfo(deviceConfig: JsonObject, nodeId: string): JsonObject {
  const name = getNestedString(deviceConfig, ['name']) || nodeId
  const deviceId = getNestedString(deviceConfig, ['id']) || ''
  const netId = getNestedString(deviceConfig, ['net_id']) || ''
  const supportContainer = typeof deviceConfig.support_container === 'boolean'
    ? deviceConfig.support_container
    : true

  return {
    name,
    id: deviceId,
    net_id: netId,
    support_container: supportContainer,
    cpu_mhz: 0,
    total_mem: 0,
    mem_usage: 0,
    gpu_tflops: 0,
    gpu_total_mem: 0,
    gpu_used_mem: 0,
    ips: [],
    all_ip: [],
    state: 'Running',
    device_doc: deviceConfig,
  }
}

async function resolveOpendanBinary(buckyosRoot: string): Promise<string> {
  const scriptDir = new URL('.', import.meta.url).pathname
  const candidates = [
    joinPath(buckyosRoot, 'bin', 'opendan', 'opendan'),
    joinPath(scriptDir, 'opendan', 'opendan'),
  ]

  for (const candidate of candidates) {
    if (await fileExists(candidate)) {
      return candidate
    }
  }

  throw new Error(`opendan binary not found, checked: ${candidates.join(', ')}`)
}

async function resolveAgentPackageRoot(
  buckyosRoot: string,
  appDoc: JsonObject,
): Promise<{ pkgId: string; fullPath: string }> {
  const pkgId = getNestedString(appDoc, ['pkg_list', 'agent', 'pkg_id'])
  if (!pkgId) {
    throw new Error('app_doc.pkg_list.agent.pkg_id is missing, only agent/opendan is supported')
  }

  const pkgName = uniquePkgName(pkgId)
  const candidates = [
    joinPath(buckyosRoot, 'bin', pkgName),
  ]

  for (const candidate of candidates) {
    if (await fileExists(candidate)) {
      return {
        pkgId,
        fullPath: candidate,
      }
    }
  }

  throw new Error(`agent package root not found for pkg ${pkgId}`)
}

async function sysConfigGet(client: KRPCClient, key: string): Promise<JsonObject | null> {
  const result = await client.call('sys_config_get', { key })
  if (!result || typeof result !== 'object' || Array.isArray(result)) {
    return null
  }

  const value = (result as JsonObject).value
  if (typeof value !== 'string' || value.length === 0) {
    return null
  }

  return JSON.parse(value) as JsonObject
}

async function loadAppSpec(
  client: KRPCClient,
  appId: string,
  ownerUserId: string,
): Promise<{ key: string; value: JsonObject }> {
  const candidateKeys = [
    `users/${ownerUserId}/agents/${appId}/spec`,
    `users/${ownerUserId}/apps/${appId}/spec`,
  ]

  for (const key of candidateKeys) {
    try {
      const value = await sysConfigGet(client, key)
      if (value) {
        return { key, value }
      }
    } catch {
      // try next key
    }
  }

  throw new Error(
    `app spec not found, checked: ${candidateKeys.join(', ')}`,
  )
}

async function loadAppInstanceConfig(
  client: KRPCClient,
  nodeId: string,
  appId: string,
  ownerUserId: string,
  spec: JsonObject,
): Promise<JsonObject> {
  const nodeConfig = await sysConfigGet(client, `nodes/${nodeId}/config`).catch(() => null)
  const instanceId = `${appId}@${ownerUserId}@${nodeId}`

  if (nodeConfig) {
    const apps = getNestedObject(nodeConfig, ['apps'])
    const instance = apps?.[instanceId]
    if (instance && typeof instance === 'object' && !Array.isArray(instance)) {
      return instance as JsonObject
    }
  }

  return {
    target_state: 'Started',
    node_id: nodeId,
    app_spec: spec,
    service_ports_config: {},
  }
}

async function buildLaunchContext(options: StartupOptions) {
  const buckyosRoot = getBuckyosRoot()
  const etcDir = joinPath(buckyosRoot, 'etc')
  const nodeIdentityPath = joinPath(etcDir, 'node_identity.json')
  const nodePrivateKeyPath = joinPath(etcDir, 'node_private_key.pem')
  const nodeDeviceConfigPath = joinPath(etcDir, 'node_device_config.json')

  const nodeIdentity = await readJsonFile(nodeIdentityPath)
  const nodeDeviceConfig = await readJsonFile(nodeDeviceConfigPath)
  const nodePrivateKeyPem = await Deno.readTextFile(nodePrivateKeyPath)
  const deviceConfig = decodeJwtPayload<JsonObject>(
    getNestedString(nodeIdentity, ['device_doc_jwt']) || '',
  )
  const deviceName =
    getNestedString(nodeDeviceConfig, ['name']) ||
    getNestedString(deviceConfig, ['name'])
  if (!deviceName) {
    throw new Error('device name not found in node_device_config.json/device_doc_jwt')
  }

  const nodeId = options.nodeId || deviceName
  const appFullId = getFullAppId(options.appId, options.ownerUserId)
  const serviceToken = await generateAppServiceToken(
    options.appId,
    options.ownerUserId,
    deviceName,
    nodePrivateKeyPem,
  )
  const nodeDaemonToken = await generateAppServiceToken(
    'node-daemon',
    deviceName,
    deviceName,
    nodePrivateKeyPem,
  )
  const systemConfigClient = new KRPCClient(options.systemConfigUrl, nodeDaemonToken)
  const zoneConfig = await sysConfigGet(systemConfigClient, 'boot/config')
  if (!zoneConfig) {
    throw new Error('failed to load boot/config from system_config')
  }
  const runtimeDeviceInfo =
    await sysConfigGet(systemConfigClient, `devices/${nodeId}/info`).catch(() => null) ||
    buildFallbackDeviceInfo(deviceConfig, nodeId)

  const { key: specKey, value: spec } = await loadAppSpec(
    systemConfigClient,
    options.appId,
    options.ownerUserId,
  )
  const appInstanceConfig = await loadAppInstanceConfig(
    systemConfigClient,
    nodeId,
    options.appId,
    options.ownerUserId,
    spec,
  )
  const appDoc = getNestedObject(appInstanceConfig, ['app_spec', 'app_doc']) ||
    getNestedObject(spec, ['app_doc'])
  if (!appDoc) {
    throw new Error('app_doc missing from app spec')
  }

  const agentPackage = await resolveAgentPackageRoot(buckyosRoot, appDoc)
  const opendanBinary = await resolveOpendanBinary(buckyosRoot)
  const agentEnvRoot = getAppDataDir(buckyosRoot, options.appId, options.ownerUserId)
  await Deno.mkdir(agentEnvRoot, { recursive: true })
  const servicePort = selectAgentServicePort(appDoc, appInstanceConfig, options.port)

  const env: Record<string, string> = {
    BUCKYOS_ROOT: buckyosRoot,
    BUCKYOS_ZONE_CONFIG: JSON.stringify(zoneConfig),
    BUCKYOS_THIS_DEVICE_INFO: JSON.stringify(runtimeDeviceInfo),
    BUCKYOS_THIS_DEVICE: JSON.stringify(deviceConfig),
    BUCKYOS_HOST_GATEWAY: '127.0.0.1',
    app_instance_config: JSON.stringify(appInstanceConfig),
    app_media_info: JSON.stringify({
      pkg_id: agentPackage.pkgId,
      full_path: agentPackage.fullPath,
    }),
    [getSessionTokenEnvKey(appFullId, true)]: serviceToken,
    OPENDAN_AGENT_ID: options.appId,
    OPENDAN_AGENT_ENV: agentEnvRoot,
    OPENDAN_AGENT_BIN: agentPackage.fullPath,
    OPENDAN_SERVICE_PORT: `${servicePort}`,
  }

  return {
    specKey,
    nodeId,
    buckyosRoot,
    opendanBinary,
    agentEnvRoot,
    agentPackageRoot: agentPackage.fullPath,
    servicePort,
    env,
  }
}

async function runForeground(
  opendanBinary: string,
  appId: string,
  agentEnvRoot: string,
  agentPackageRoot: string,
  servicePort: number,
  env: Record<string, string>,
): Promise<number> {
  const child = new Deno.Command(opendanBinary, {
    args: [
      '--agent-id',
      appId,
      '--agent-env',
      agentEnvRoot,
      '--agent-bin',
      agentPackageRoot,
      '--service-port',
      `${servicePort}`,
    ],
    env,
    stdin: 'inherit',
    stdout: 'inherit',
    stderr: 'inherit',
  }).spawn()

  const status = await child.status
  return status.code
}

async function runDetached(
  opendanBinary: string,
  appId: string,
  agentEnvRoot: string,
  agentPackageRoot: string,
  servicePort: number,
  env: Record<string, string>,
): Promise<void> {
  const child = new Deno.Command(opendanBinary, {
    args: [
      '--agent-id',
      appId,
      '--agent-env',
      agentEnvRoot,
      '--agent-bin',
      agentPackageRoot,
      '--service-port',
      `${servicePort}`,
    ],
    env,
    stdin: 'null',
    stdout: 'inherit',
    stderr: 'inherit',
  }).spawn()

  console.log(`started detached opendan pid=${child.pid}`)
}

async function main() {
  try {
    const options = parseArgs(Deno.args)
    const launch = await buildLaunchContext(options)

    console.log(`app spec key: ${launch.specKey}`)
    console.log(`node id: ${launch.nodeId}`)
    console.log(`agent env: ${launch.agentEnvRoot}`)
    console.log(`agent package: ${launch.agentPackageRoot}`)
    console.log(`service port: ${launch.servicePort}`)
    console.log(`opendan binary: ${launch.opendanBinary}`)

    if (options.detach) {
      await runDetached(
        launch.opendanBinary,
        options.appId,
        launch.agentEnvRoot,
        launch.agentPackageRoot,
        launch.servicePort,
        launch.env,
      )
      return
    }

    const code = await runForeground(
      launch.opendanBinary,
      options.appId,
      launch.agentEnvRoot,
      launch.agentPackageRoot,
      launch.servicePort,
      launch.env,
    )
    Deno.exit(code)
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error))
    Deno.exit(1)
  }
}

if (import.meta.main) {
  await main()
}
