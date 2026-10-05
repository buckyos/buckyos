/* The only module that knows how the Desktop reaches the aiworkspace service.
 *
 * Production: the Desktop's normal service path — `buckyos.getServiceRpcClient('aiworkspace')`
 * (kRPC through the zone gateway at /kapi/aiworkspace, session token managed by the SDK); the
 * upload/download routes get `Authorization: Bearer <session token>`.
 *
 * Development / e2e against the standalone backend (`aiworkspace --data-dir … --auth-file …`):
 * an explicit override, never active by accident:
 *   localStorage['aiworkspace.dev'] = '{"token":"tok-alice"}'      (optional "baseUrl", "principal")
 *   or the URL parameter  ?aiwsDevToken=tok-alice                   (stored into the same key)
 * With the override the module talks plain kRPC to `baseUrl` (default `/kapi/aiworkspace`, which the
 * Vite dev server forwards to `AIWS_BACKEND`). There is no mock backend. */

import { buckyos } from 'buckyos'
import { isMockRuntime } from '../../../runtime'

export const SERVICE_NAME = 'aiworkspace'
export const DEV_OVERRIDE_KEY = 'aiworkspace.dev'

/** The request did not produce an answer from the service (network, proxy, timeout, kRPC-level error). */
export class TransportError extends Error {
  readonly kind: 'network' | 'protocol' | 'unconfigured'
  constructor(kind: 'network' | 'protocol' | 'unconfigured', message: string) {
    super(message)
    this.name = 'TransportError'
    this.kind = kind
  }
}

export interface Transport {
  readonly mode: 'zone' | 'dev-override'
  /** Display name of the caller when the transport knows it (the service never trusts it). */
  readonly principalHint: string | null
  /** kRPC call; resolves with `result` (business results live there), rejects with TransportError. */
  call<T>(method: string, params: Record<string, unknown>, signal?: AbortSignal): Promise<T>
  /** `PUT /upload/<upload_id>`. */
  upload(uploadId: string, body: Blob | Uint8Array): Promise<void>
  /** Authenticated `GET <base>/<path>` (asset, export, replica routes). */
  download(path: string): Promise<Blob>
}

interface DevOverride { token: string; baseUrl?: string; principal?: string }

function readDevOverride(): DevOverride | null {
  try {
    const fromUrl = new URLSearchParams(window.location.search).get('aiwsDevToken')
    if (fromUrl) window.localStorage.setItem(DEV_OVERRIDE_KEY, JSON.stringify({ token: fromUrl }))
    const raw = window.localStorage.getItem(DEV_OVERRIDE_KEY)
    if (!raw) return null
    const parsed = JSON.parse(raw) as Partial<DevOverride>
    return typeof parsed.token === 'string' && parsed.token ? { token: parsed.token, baseUrl: parsed.baseUrl, principal: parsed.principal } : null
  } catch {
    return null
  }
}

async function httpError(response: Response): Promise<TransportError> {
  const text = await response.text().catch(() => '')
  return new TransportError('protocol', `HTTP ${response.status}${text ? `: ${text.slice(0, 300)}` : ''}`)
}

class DevOverrideTransport implements Transport {
  readonly mode = 'dev-override' as const
  readonly principalHint: string | null
  private seq = 1
  private readonly base: string
  private readonly token: string

  constructor(override: DevOverride) {
    this.token = override.token
    this.base = (override.baseUrl ?? `/kapi/${SERVICE_NAME}`).replace(/\/+$/, '')
    this.principalHint = override.principal ?? override.token.replace(/^tok-/, '')
  }

  async call<T>(method: string, params: Record<string, unknown>, signal?: AbortSignal): Promise<T> {
    let response: Response
    try {
      response = await fetch(this.base, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ method, params, sys: [this.seq++, this.token] }),
        signal,
      })
    } catch (error) {
      throw new TransportError('network', error instanceof Error ? error.message : String(error))
    }
    if (!response.ok) throw await httpError(response)
    let body: { result?: T; error?: string }
    try {
      body = await response.json() as { result?: T; error?: string }
    } catch {
      throw new TransportError('network', 'unparsable response')
    }
    if (typeof body.error === 'string' && body.error) throw new TransportError('protocol', body.error)
    return body.result as T
  }

  async upload(uploadId: string, body: Blob | Uint8Array): Promise<void> {
    let response: Response
    try {
      response = await fetch(`${this.base}/upload/${uploadId}`, { method: 'PUT', headers: { Authorization: `Bearer ${this.token}` }, body: body as BodyInit })
    } catch (error) {
      throw new TransportError('network', error instanceof Error ? error.message : String(error))
    }
    if (!response.ok) throw await httpError(response)
  }

  async download(path: string): Promise<Blob> {
    let response: Response
    try {
      response = await fetch(`${this.base}/${path}`, { headers: { Authorization: `Bearer ${this.token}` } })
    } catch (error) {
      throw new TransportError('network', error instanceof Error ? error.message : String(error))
    }
    if (!response.ok) throw await httpError(response)
    return response.blob()
  }
}

class ZoneTransport implements Transport {
  readonly mode = 'zone' as const
  principalHint: string | null = null

  private client() {
    return buckyos.getServiceRpcClient(SERVICE_NAME)
  }

  private async token(): Promise<string> {
    const fromClient = this.client().getSessionToken()
    if (fromClient) return fromClient
    const account = await buckyos.getAccountInfo()
    if (account?.user_id) this.principalHint = account.user_id
    const token = account?.session_token
    if (!token) throw new TransportError('protocol', 'no session token')
    return token
  }

  async call<T>(method: string, params: Record<string, unknown>): Promise<T> {
    try {
      return await this.client().call<T, Record<string, unknown>>(method, params)
    } catch (error) {
      // The SDK reports kRPC-level errors and network failures through the same channel; for a
      // commit both mean "no business result", which is all the callers need to know.
      const message = error instanceof Error ? error.message : String(error)
      throw new TransportError(/fetch|network|timeout|abort/i.test(message) ? 'network' : 'protocol', message)
    }
  }

  private base(): string {
    return buckyos.getZoneServiceURL(SERVICE_NAME).replace(/\/+$/, '')
  }

  async upload(uploadId: string, body: Blob | Uint8Array): Promise<void> {
    const token = await this.token()
    let response: Response
    try {
      response = await fetch(`${this.base()}/upload/${uploadId}`, { method: 'PUT', headers: { Authorization: `Bearer ${token}` }, body: body as BodyInit })
    } catch (error) {
      throw new TransportError('network', error instanceof Error ? error.message : String(error))
    }
    if (!response.ok) throw await httpError(response)
  }

  async download(path: string): Promise<Blob> {
    const token = await this.token()
    let response: Response
    try {
      response = await fetch(`${this.base()}/${path}`, { headers: { Authorization: `Bearer ${token}` } })
    } catch (error) {
      throw new TransportError('network', error instanceof Error ? error.message : String(error))
    }
    if (!response.ok) throw await httpError(response)
    return response.blob()
  }
}

async function zonePrincipal(transport: ZoneTransport): Promise<void> {
  try {
    const account = await buckyos.getAccountInfo()
    if (account?.user_id) transport.principalHint = account.user_id
  } catch { /* the hint is cosmetic */ }
}

/** Why no transport is available, for the UI to state plainly. */
export type TransportAvailability = { ok: true; transport: Transport } | { ok: false; reason: string }

export function resolveTransport(): TransportAvailability {
  const override = readDevOverride()
  if (override) return { ok: true, transport: new DevOverrideTransport(override) }
  if (isMockRuntime()) {
    return {
      ok: false,
      reason: `Desktop 正在 Mock 运行时中运行，没有可用的 aiworkspace 服务。AI 工作区没有 Mock 后台：请启动独立后台并设置 localStorage['${DEV_OVERRIDE_KEY}'] = {"token":"<测试令牌>"}（见应用 README），或在真实 Zone 中打开。`,
    }
  }
  const transport = new ZoneTransport()
  void zonePrincipal(transport)
  return { ok: true, transport }
}
