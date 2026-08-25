import { buckyos, BuckyOSSDK, namelib, RuntimeType } from 'buckyos'
import type { SessionController } from './auth.ts'
import type { ResolvedConfig } from './config.ts'
import type { ResolvedConnection } from './context.ts'
import { ToolError, UsageError } from './errors.ts'

export interface RuntimeAdapter {
  initialize(config: ResolvedConfig, session: SessionController): Promise<ResolvedConnection>
}

export interface RpcCallOptions {
  traceId: string
  timeoutMs: number
  signal?: AbortSignal
}

export interface ServiceClientRegistry {
  call<T>(
    service: string,
    method: string,
    params: Record<string, unknown>,
    options: RpcCallOptions,
  ): Promise<T>
  createEventReader?(pattern: string, signal?: AbortSignal): Promise<EventReader>
}

export interface EventReader {
  pullEvent(timeoutMs?: number): Promise<unknown | null>
  close(): Promise<void>
}

interface RpcClient {
  call<T>(
    method: string,
    params: Record<string, unknown>,
    options: { traceId: string },
  ): Promise<T>
}

type RpcClientConstructor = new (
  url: string,
  token?: string | null,
  sequence?: number | null,
  options?: { sessionTokenProvider?: () => Promise<string | null> },
) => RpcClient

export class BuckyOSRuntimeAdapter implements RuntimeAdapter {
  #sdk = new BuckyOSSDK('node')

  async initialize(
    config: ResolvedConfig,
    session: SessionController,
  ): Promise<ResolvedConnection> {
    const authenticated = session.current()
    const zoneHost = resolveZoneHost(config)
    const initialize = this.#sdk.initBuckyOS as unknown as (
      appId: string,
      config: Record<string, unknown>,
    ) => Promise<void>
    await initialize.call(this.#sdk, authenticated.principal.appId, {
      appId: authenticated.principal.appId,
      ownerUserId: authenticated.principal.id,
      runtimeType: RuntimeType.AppClient,
      zoneHost,
      defaultProtocol: config.defaultProtocol,
      sessionToken: authenticated.token,
      privateKeySearchPaths: [],
      autoRenew: false,
      verifyHubServiceUrl: resolveServiceUrl(config, 'verify-hub'),
    })
    return {
      zone: config.zone ?? zoneHost,
      endpoint: config.endpoint ?? `${config.defaultProtocol}${zoneHost}`,
      defaultProtocol: config.defaultProtocol,
    }
  }
}

export class BuckyOSServiceClientRegistry implements ServiceClientRegistry {
  readonly #config: ResolvedConfig
  readonly #session: SessionController
  readonly #clients = new Map<string, RpcClient>()

  constructor(config: ResolvedConfig, session: SessionController) {
    this.#config = config
    this.#session = session
  }

  async call<T>(
    service: string,
    method: string,
    params: Record<string, unknown>,
    options: RpcCallOptions,
  ): Promise<T> {
    let client = this.#clients.get(service)
    if (!client) {
      const RpcClient = buckyos.kRPCClient as unknown as RpcClientConstructor
      client = new RpcClient(resolveServiceUrl(this.#config, service), null, null, {
        sessionTokenProvider: async () => (await this.#session.ensureValid()).token,
      })
      this.#clients.set(service, client)
    }
    const request = client.call(method, params, { traceId: options.traceId }) as Promise<T>
    return await withDeadline(request, options.timeoutMs, options.signal)
  }

  async createEventReader(pattern: string, signal?: AbortSignal): Promise<EventReader> {
    return await buckyos.createEventReader(pattern, { keepaliveMs: 5_000, signal })
  }
}

export class InteractiveSession {
  readonly authentication: SessionController
  readonly clients: ServiceClientRegistry
  readonly connection: ResolvedConnection
  readonly startedAt: number

  private constructor(
    authentication: SessionController,
    clients: ServiceClientRegistry,
    connection: ResolvedConnection,
  ) {
    this.authentication = authentication
    this.clients = clients
    this.connection = connection
    this.startedAt = Date.now()
  }

  static async create(
    config: ResolvedConfig,
    authentication: SessionController,
    runtime: RuntimeAdapter = new BuckyOSRuntimeAdapter(),
    clients?: ServiceClientRegistry,
  ): Promise<InteractiveSession> {
    await authentication.connect()
    const connection = await runtime.initialize(config, authentication)
    return new InteractiveSession(
      authentication,
      clients ?? new BuckyOSServiceClientRegistry(config, authentication),
      connection,
    )
  }

  async reconnect(): Promise<void> {
    await this.authentication.reconnect()
  }
}

export function resolveServiceUrl(config: ResolvedConfig, service: string): string {
  if (config.endpoint) {
    const endpoint = new URL(config.endpoint)
    const marker = endpoint.pathname.indexOf('/kapi/')
    if (marker >= 0) endpoint.pathname = `${endpoint.pathname.slice(0, marker)}/kapi/${service}`
    else if (endpoint.pathname.endsWith('/kapi')) {
      endpoint.pathname = `${endpoint.pathname}/${service}`
    } else endpoint.pathname = `${endpoint.pathname.replace(/\/$/, '')}/kapi/${service}`
    endpoint.search = ''
    endpoint.hash = ''
    return endpoint.toString().replace(/\/$/, '')
  }
  const host = resolveZoneHost(config)
  return `${config.defaultProtocol}${host}/kapi/${service}`
}

export function resolveZoneHost(config: ResolvedConfig): string {
  if (config.zone) {
    try {
      return namelib.DID.fromStr(config.zone).toRawHostName()
    } catch {
      throw new UsageError('INVALID_ZONE', `invalid zone host or DID: ${config.zone}`)
    }
  }
  if (config.endpoint) return new URL(config.endpoint).host
  throw new UsageError('CONNECTION_REQUIRED', 'an endpoint or zone is required for online commands')
}

export async function withDeadline<T>(
  promise: Promise<T>,
  timeoutMs: number,
  signal?: AbortSignal,
): Promise<T> {
  if (signal?.aborted) throw new ToolError('CANCELED', 'operation canceled', 8)
  let timer: ReturnType<typeof setTimeout> | undefined
  let abortHandler: (() => void) | undefined
  const guards: Promise<never>[] = [
    new Promise((_, reject) => {
      timer = setTimeout(
        () => reject(new ToolError('TIMEOUT', 'operation timed out', 8, true)),
        timeoutMs,
      )
    }),
  ]
  if (signal) {
    guards.push(
      new Promise((_, reject) => {
        abortHandler = () => reject(new ToolError('CANCELED', 'operation canceled', 8))
        signal.addEventListener('abort', abortHandler, { once: true })
      }),
    )
  }
  try {
    return await Promise.race([promise, ...guards])
  } finally {
    if (timer !== undefined) clearTimeout(timer)
    if (signal && abortHandler) signal.removeEventListener('abort', abortHandler)
  }
}
