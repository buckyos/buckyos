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
 * Vite dev server forwards to `AIWS_BACKEND`). There is no mock backend.
 *
 * Guests (a public link, UI improvement §8.3): without a signed-in account the zone transport sends a
 * guest token instead of a session — `gt_` + 26 base32 chars this browser made up once and keeps in
 * localStorage['aiworkspace.guest']. The service derives the guest principal from it, so the same
 * browser stays the same guest. `{"guest":true}` in the dev override does the same against the
 * standalone backend. */

import { buckyos } from 'buckyos'
import { isMockRuntime } from '../../../runtime'

export const SERVICE_NAME = 'aiworkspace'
export const DEV_OVERRIDE_KEY = 'aiworkspace.dev'
export const GUEST_TOKEN_KEY = 'aiworkspace.guest'
export const GUEST_PREFIX = 'guest:'

export function isGuestPrincipal(principal: string | null | undefined): boolean {
  return typeof principal === 'string' && principal.startsWith(GUEST_PREFIX)
}

const BASE32 = 'abcdefghijklmnopqrstuvwxyz234567'

/** This browser's guest token (made once, kept for good): the service turns it into the guest principal. */
export function guestToken(): string {
  try {
    const kept = window.localStorage.getItem(GUEST_TOKEN_KEY)
    if (kept && /^gt_[a-z2-7]{26}$/.test(kept)) return kept
  } catch { /* storage unavailable: a fresh guest each time */ }
  const bytes = crypto.getRandomValues(new Uint8Array(26))
  const token = `gt_${Array.from(bytes, (b) => BASE32[b % 32]).join('')}`
  try { window.localStorage.setItem(GUEST_TOKEN_KEY, token) } catch { /* see above */ }
  return token
}

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
  /** The service this transport reaches (zone host or dev backend): app-level local state is kept per target. */
  readonly target: string
  /** The caller once known (the Zone account is looked up asynchronously). */
  principal(): Promise<string | null>
  /** True when the caller is a guest of a public link (no signed-in account). */
  isGuest(): Promise<boolean>
  /** The service told who the caller is (`ws.get_info.principal`): the only way a guest learns its principal. */
  learnPrincipal(principal: string): void
  /** kRPC call; resolves with `result` (business results live there), rejects with TransportError. */
  call<T>(method: string, params: Record<string, unknown>, signal?: AbortSignal): Promise<T>
  /** `PUT /upload/<upload_id>`. */
  upload(uploadId: string, body: Blob | Uint8Array): Promise<void>
  /** Authenticated `GET <base>/<path>` (asset, export, replica routes). */
  download(path: string): Promise<Blob>
}

interface DevOverride { token: string; guest: boolean; baseUrl?: string; principal?: string }

function readDevOverride(): DevOverride | null {
  try {
    const fromUrl = new URLSearchParams(window.location.search).get('aiwsDevToken')
    if (fromUrl) window.localStorage.setItem(DEV_OVERRIDE_KEY, JSON.stringify({ token: fromUrl }))
    const raw = window.localStorage.getItem(DEV_OVERRIDE_KEY)
    if (!raw) return null
    const parsed = JSON.parse(raw) as Partial<DevOverride>
    if (parsed.guest === true) return { token: guestToken(), guest: true, baseUrl: parsed.baseUrl }
    return typeof parsed.token === 'string' && parsed.token ? { token: parsed.token, guest: false, baseUrl: parsed.baseUrl, principal: parsed.principal } : null
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
  principalHint: string | null
  private seq = 1
  private readonly base: string
  private readonly token: string
  private readonly guest: boolean

  constructor(override: DevOverride) {
    this.token = override.token
    this.guest = override.guest
    this.base = (override.baseUrl ?? `/kapi/${SERVICE_NAME}`).replace(/\/+$/, '')
    this.principalHint = override.guest ? null : override.principal ?? override.token.replace(/^tok-/, '')
  }

  get target(): string { return `${window.location.host}${this.base}` }
  async principal(): Promise<string | null> { return this.principalHint }
  async isGuest(): Promise<boolean> { return this.guest }
  learnPrincipal(principal: string) { if (this.guest) this.principalHint = principal }

  call<T>(method: string, params: Record<string, unknown>, signal?: AbortSignal): Promise<T> {
    return guestCall<T>(this.base, method, params, this.token, this.seq++, signal)
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
  readonly target = window.location.host
  private principalLookup: Promise<void> | null = null
  /** Decided once: a signed-in account uses the SDK's session, anyone else is a guest. */
  private guestLookup: Promise<boolean> | null = null
  private seq = 1

  async principal(): Promise<string | null> {
    this.principalLookup ??= zonePrincipal(this)
    await this.principalLookup
    return this.principalHint
  }

  isGuest(): Promise<boolean> {
    this.guestLookup ??= this.token().then((token) => token === null)
    return this.guestLookup
  }

  learnPrincipal(principal: string) { if (isGuestPrincipal(principal)) this.principalHint = principal }

  private client() {
    return buckyos.getServiceRpcClient(SERVICE_NAME)
  }

  /** The session token, or null without a signed-in account. */
  private async token(): Promise<string | null> {
    let fromClient: string | null | undefined = null
    try { fromClient = this.client().getSessionToken() } catch { /* no session at all: a guest */ }
    if (fromClient) return fromClient
    const account = await buckyos.getAccountInfo().catch(() => null)
    if (account?.user_id) this.principalHint = account.user_id
    return account?.session_token || null
  }

  private async bearer(): Promise<string> {
    return (await this.isGuest()) ? guestToken() : (await this.token()) ?? guestToken()
  }

  async call<T>(method: string, params: Record<string, unknown>, signal?: AbortSignal): Promise<T> {
    if (await this.isGuest()) return guestCall<T>(this.base(), method, params, guestToken(), this.seq++, signal)
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
    const token = await this.bearer()
    let response: Response
    try {
      response = await fetch(`${this.base()}/upload/${uploadId}`, { method: 'PUT', headers: { Authorization: `Bearer ${token}` }, body: body as BodyInit })
    } catch (error) {
      throw new TransportError('network', error instanceof Error ? error.message : String(error))
    }
    if (!response.ok) throw await httpError(response)
  }

  async download(path: string): Promise<Blob> {
    const token = await this.bearer()
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

/** Plain kRPC with a guest token in the envelope (no SDK session). */
async function guestCall<T>(base: string, method: string, params: Record<string, unknown>, token: string, seq: number, signal?: AbortSignal): Promise<T> {
  let response: Response
  try {
    response = await fetch(base, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ method, params, sys: [seq, token] }), signal })
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
  void transport.principal()
  return { ok: true, transport }
}

/** A prompter link's call (第三期规划 §11.1): plain kRPC to the service with the show's token instead of a session —
 * the page runs before (and without) the Desktop's login. Resolves with `result`, rejects with TransportError. */
export async function prompterCall<T>(method: string, params: Record<string, unknown>, token: string, signal?: AbortSignal): Promise<T> {
  const base = (readDevOverride()?.baseUrl ?? `/kapi/${SERVICE_NAME}`).replace(/\/+$/, '')
  let response: Response
  try {
    response = await fetch(base, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ method, params: { ...params, prompter_token: token }, sys: [Date.now()] }),
      signal,
    })
  } catch (error) {
    throw new TransportError('network', error instanceof Error ? error.message : String(error))
  }
  if (!response.ok) throw await httpError(response)
  const body = await response.json().catch(() => null) as { result?: T; error?: string } | null
  if (!body) throw new TransportError('network', 'unparsable response')
  if (typeof body.error === 'string' && body.error) throw new TransportError('protocol', body.error)
  return body.result as T
}
