/* The only module that knows how the Desktop reaches the aiworkspace service.
 *
 * Production: the Desktop's normal service path — `buckyos.getServiceRpcClient('aiworkspace')`
 * (kRPC through the zone gateway at /kapi/aiworkspace, session token managed by the SDK); the
 * asset route gets `Authorization: Bearer <session token>`. Uploads go through NDM into the caller's
 * own buckets of the service; downloads (exports, replicas) come from `/bucket/v1/...` with a token.
 *
 * Development / e2e against the standalone backend (`aiworkspace --data-dir … --auth-file …`):
 * an explicit override, never active by accident:
 *   localStorage['aiworkspace.dev'] = '{"token":"tok-alice"}'      (optional "baseUrl", "principal")
 *   or the URL parameter  ?aiwsDevToken=tok-alice                   (stored into the same key)
 * With the override the module talks plain kRPC to `baseUrl` (default `/kapi/aiworkspace`, which the
 * Vite dev server forwards to `AIWS_BACKEND`). There is no mock backend.
 *
 * Guests (a public link, UI improvement §8.3): without a signed-in account the zone transport sends a
 * guest token instead of a session — `gt_` + base64url of 32 random bytes (the zone's bearer token form) this browser made up once and keeps in
 * localStorage['aiworkspace.guest']. The service derives the guest principal from it, so the same
 * browser stays the same guest. `{"guest":true}` in the dev override does the same against the
 * standalone backend.
 *
 * Hybrid SSO: a Zone session may be restricted (a Limited account or an External DID; contract §2, §3) — known from
 * `buckyos.getCurrentIdentity()`, never from the shape of the principal. A refusal of the session itself comes back
 * as a kRPC failure with `code: access_denied`; TransportError carries its denial. `{"restricted":true}` in the dev
 * override stands for such a session against the standalone backend. */

import { accessDenialOf, bucket, buckyos, parseAccessDenial, ACCESS_DENIED_ERROR_CODE, type AccessDenial } from 'buckyos'
import { bucketObjectUrl } from '../../../components/preview/bucketSource'
import { isMockRuntime } from '../../../runtime'
import { aiwsT } from '../i18n'
import type { BucketDownload } from './types'

export const SERVICE_NAME = 'aiworkspace'
export const DEV_OVERRIDE_KEY = 'aiworkspace.dev'
export const GUEST_TOKEN_KEY = 'aiworkspace.guest'
export const GUEST_PREFIX = 'guest:'

export function isGuestPrincipal(principal: string | null | undefined): boolean {
  return typeof principal === 'string' && principal.startsWith(GUEST_PREFIX)
}

/** This browser's guest token (made once, kept for good): the service turns it into the guest principal. */
export function guestToken(): string {
  try {
    const kept = window.localStorage.getItem(GUEST_TOKEN_KEY)
    if (kept && /^gt_[A-Za-z0-9_-]{42}[AEIMQUYcgkosw048]$/.test(kept)) return kept
  } catch { /* storage unavailable: a fresh guest each time */ }
  const bytes = crypto.getRandomValues(new Uint8Array(32))
  const token = `gt_${btoa(String.fromCharCode(...bytes)).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '')}`
  try { window.localStorage.setItem(GUEST_TOKEN_KEY, token) } catch { /* see above */ }
  return token
}

/** The request did not produce an answer from the service (network, proxy, timeout, kRPC-level error). */
export class TransportError extends Error {
  readonly kind: 'network' | 'protocol' | 'unconfigured'
  /** The service refused the session itself (D-4), e.g. a restricted session without the `aiworkspace` scope. */
  readonly denial: AccessDenial | null
  constructor(kind: 'network' | 'protocol' | 'unconfigured', message: string, denial: AccessDenial | null = null) {
    super(message)
    this.name = 'TransportError'
    this.kind = kind
    this.denial = denial
  }
}

/** A failed kRPC body (`{ error, code?, data? }`) as a TransportError. */
function rpcFailure(body: { error: string; code?: unknown; data?: unknown }): TransportError {
  return new TransportError('protocol', body.error, body.code === ACCESS_DENIED_ERROR_CODE ? parseAccessDenial(body.data) : null)
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
  /** True for a restricted session (Limited account, External DID): it owns no workspace and never counts as
   * "any signed-in user". A guest is not counted here. */
  restricted(): Promise<boolean>
  /** The service told who the caller is (`ws.get_info.principal`): the only way a guest learns its principal. */
  learnPrincipal(principal: string): void
  importOptions(): Promise<{ endpoint: string; sessionToken: string | null }>
  /** kRPC call; resolves with `result` (business results live there), rejects with TransportError. */
  call<T>(method: string, params: Record<string, unknown>, signal?: AbortSignal): Promise<T>
  /** Authenticated `GET <base>/<path>` (the asset route). */
  download(path: string): Promise<Blob>
  /** The caller's own bucket `name` of this service (`user/aiworkspace/<user>/<name>`); null for a guest. */
  ownBucket(name: string): Promise<string | null>
  /** A file the service left in a bucket (an export, a replica), read with its token. */
  fetchDownload(download: BucketDownload): Promise<Blob>
}

function userBucket(principal: string | null, name: string): string | null {
  if (!principal || isGuestPrincipal(principal)) return null
  try { return bucket.userBucketId(SERVICE_NAME, principal, name) } catch { return null }
}

async function fetchBlob(url: string): Promise<Blob> {
  let response: Response
  try {
    response = await fetch(url, { credentials: 'same-origin' })
  } catch (error) {
    throw new TransportError('network', error instanceof Error ? error.message : String(error))
  }
  if (!response.ok) throw await httpError(response)
  return response.blob()
}

interface DevOverride { token: string; guest: boolean; baseUrl?: string; principal?: string; restricted?: boolean; llm?: DevModelEndpoint }

/** Development only: the local executor talks OpenAI-format chat to this endpoint instead of the Zone's AICC. */
export interface DevModelEndpoint { baseUrl: string; apiKey?: string }

export function devModelEndpoint(): DevModelEndpoint | null {
  return readDevOverride()?.llm ?? null
}

function readDevOverride(): DevOverride | null {
  try {
    const fromUrl = new URLSearchParams(window.location.search).get('aiwsDevToken')
    if (fromUrl) window.localStorage.setItem(DEV_OVERRIDE_KEY, JSON.stringify({ token: fromUrl }))
    const raw = window.localStorage.getItem(DEV_OVERRIDE_KEY)
    if (!raw) return null
    const parsed = JSON.parse(raw) as Partial<DevOverride>
    if (parsed.guest === true) return { token: guestToken(), guest: true, baseUrl: parsed.baseUrl }
    return typeof parsed.token === 'string' && parsed.token
      ? { token: parsed.token, guest: false, baseUrl: parsed.baseUrl, principal: parsed.principal, restricted: parsed.restricted === true, llm: parsed.llm && typeof parsed.llm.baseUrl === 'string' ? parsed.llm : undefined }
      : null
  } catch {
    return null
  }
}

/** A failed asset or bucket download: its body is `{ ok: false, error }`, a refusal's `error.denial` included. */
async function httpError(response: Response): Promise<TransportError> {
  const text = await response.text().catch(() => '')
  let denial: AccessDenial | null = null
  try {
    denial = parseAccessDenial((JSON.parse(text) as { error?: { denial?: unknown } } | null)?.error?.denial)
  } catch { /* not JSON */ }
  return new TransportError('protocol', `HTTP ${response.status}${text ? `: ${text.slice(0, 300)}` : ''}`, denial)
}

class DevOverrideTransport implements Transport {
  readonly mode = 'dev-override' as const
  principalHint: string | null
  private seq = 1
  private readonly base: string
  private readonly token: string
  private readonly guest: boolean
  private readonly limited: boolean

  constructor(override: DevOverride) {
    this.token = override.token
    this.guest = override.guest
    this.limited = override.restricted === true
    this.base = (override.baseUrl ?? `/kapi/${SERVICE_NAME}`).replace(/\/+$/, '')
    this.principalHint = override.guest ? null : override.principal ?? override.token.replace(/^tok-/, '')
  }

  get target(): string { return `${window.location.host}${this.base}` }
  async principal(): Promise<string | null> { return this.principalHint }
  async isGuest(): Promise<boolean> { return this.guest }
  async restricted(): Promise<boolean> { return this.limited }
  learnPrincipal(principal: string) { if (this.guest) this.principalHint = principal }
  async importOptions() { return { endpoint: new URL(this.base, window.location.origin).origin, sessionToken: this.token } }

  call<T>(method: string, params: Record<string, unknown>, signal?: AbortSignal): Promise<T> {
    return guestCall<T>(this.base, method, params, this.token, this.seq++, signal)
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

  async ownBucket(name: string): Promise<string | null> { return this.guest ? null : userBucket(this.principalHint, name) }

  // the dev server forwards `/bucket` to the standalone nfs-server, as the zone gateway does
  fetchDownload(download: BucketDownload): Promise<Blob> {
    return fetchBlob(`${bucket.bucketObjectPath(download.bucket_id, download.key)}?token=${encodeURIComponent(download.token)}`)
  }
}

class ZoneTransport implements Transport {
  readonly mode = 'zone' as const
  principalHint: string | null = null
  readonly target = window.location.host
  private principalLookup: Promise<void> | null = null
  /** Decided once: a signed-in account uses the SDK's session, anyone else is a guest. */
  private guestLookup: Promise<boolean> | null = null
  private restrictedLookup: Promise<boolean> | null = null
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

  restricted(): Promise<boolean> {
    this.restrictedLookup ??= buckyos.getCurrentIdentity()
      .then((identity) => identity.kind !== 'anonymous' && identity.restricted)
      .catch(() => false)
    return this.restrictedLookup
  }

  learnPrincipal(principal: string) { if (isGuestPrincipal(principal)) this.principalHint = principal }
  async importOptions() { return { endpoint: new URL(this.base(), window.location.origin).origin, sessionToken: await this.token() } }

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
      const denial = accessDenialOf(error)
      throw new TransportError(denial || !/fetch|network|timeout|abort/i.test(message) ? 'protocol' : 'network', message, denial)
    }
  }

  private base(): string {
    return buckyos.getZoneServiceURL(SERVICE_NAME).replace(/\/+$/, '')
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

  async ownBucket(name: string): Promise<string | null> {
    if (await this.isGuest()) return null
    return userBucket(await this.principal(), name)
  }

  fetchDownload(download: BucketDownload): Promise<Blob> {
    return fetchBlob(bucketObjectUrl({ bucketId: download.bucket_id, key: download.key }, { token: download.token }))
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
  let body: { result?: T; error?: string; code?: unknown; data?: unknown }
  try {
    body = await response.json() as { result?: T; error?: string; code?: unknown; data?: unknown }
  } catch {
    throw new TransportError('network', 'unparsable response')
  }
  if (typeof body.error === 'string' && body.error) throw rpcFailure({ ...body, error: body.error })
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
      reason: aiwsT('aiworkspace.core.transport.mockRuntime', { key: DEV_OVERRIDE_KEY }),
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
  const body = await response.json().catch(() => null) as { result?: T; error?: string; code?: unknown; data?: unknown } | null
  if (!body) throw new TransportError('network', 'unparsable response')
  if (typeof body.error === 'string' && body.error) throw rpcFailure({ ...body, error: body.error })
  return body.result as T
}
