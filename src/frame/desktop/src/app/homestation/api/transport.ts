import { buckyos } from 'buckyos'
import { isMockRuntime } from '../../../runtime'
import type { FileObject, ObjId } from '../protocol/feed'

export const SERVICE_NAME = 'homestation'
export const DEV_OVERRIDE_KEY = 'homestation.dev'

export class TransportError extends Error {
  readonly kind: 'network' | 'protocol'
  constructor(kind: 'network' | 'protocol', message: string) {
    super(message)
    this.name = 'TransportError'
    this.kind = kind
  }
}

export interface UploadMeta {
  name: string
  mime: string
  width?: number
  height?: number
  duration_ms?: number
}

export interface Transport {
  readonly mode: 'zone' | 'dev-override'
  call<T>(method: string, params?: Record<string, unknown>): Promise<T>
  upload(body: Blob, meta: UploadMeta): Promise<{ objId: ObjId; file: FileObject }>
  contentUrl(objId: ObjId): string
}

// Media are read from the home that serves the page: the signed-in user's own home in the
// HomeStation app (it holds everything shown there), the portal's feed (`<user>` or `~zone`)
// on a portal page. One HomeStation view is mounted at a time, so a module value is enough.
let contentHome = ''

export function setContentHome(home: string) {
  contentHome = home
}

interface DevOverride { token: string; baseUrl?: string }

function readDevOverride(): DevOverride | null {
  try {
    const fromUrl = new URLSearchParams(window.location.search).get('hsDevToken')
    if (fromUrl) window.localStorage.setItem(DEV_OVERRIDE_KEY, JSON.stringify({ token: fromUrl }))
    const raw = window.localStorage.getItem(DEV_OVERRIDE_KEY)
    if (!raw) return null
    const parsed = JSON.parse(raw) as Partial<DevOverride>
    return typeof parsed.token === 'string' && parsed.token ? { token: parsed.token, baseUrl: parsed.baseUrl } : null
  } catch {
    return null
  }
}

async function httpError(response: Response): Promise<TransportError> {
  const text = await response.text().catch(() => '')
  return new TransportError('protocol', `HTTP ${response.status}${text ? `: ${text.slice(0, 300)}` : ''}`)
}

function uploadQuery(meta: UploadMeta) {
  const query = new URLSearchParams({ name: meta.name, mime: meta.mime })
  for (const key of ['width', 'height', 'duration_ms'] as const) {
    const value = meta[key]
    if (value !== undefined) query.set(key, String(Math.round(value)))
  }
  return query.toString()
}

async function putUpload(url: string, token: string, body: Blob): Promise<{ objId: ObjId; file: FileObject }> {
  let response: Response
  try {
    response = await fetch(url, { method: 'PUT', headers: { Authorization: `Bearer ${token}` }, body })
  } catch (error) {
    throw new TransportError('network', error instanceof Error ? error.message : String(error))
  }
  if (!response.ok) throw await httpError(response)
  return response.json() as Promise<{ objId: ObjId; file: FileObject }>
}

function contentPath(origin: string, objId: ObjId, token: string) {
  const path = `${origin}/home/${encodeURIComponent(contentHome)}/objects/${encodeURIComponent(objId)}/content`
  return token ? `${path}?access=${encodeURIComponent(token)}` : path
}

class DevOverrideTransport implements Transport {
  readonly mode = 'dev-override' as const
  private seq = 1
  private readonly base: string
  private readonly origin: string
  private readonly token: string

  constructor(override: DevOverride) {
    // `anonymous`: no session at all (portal pages as a visitor sees them).
    this.token = override.token === 'anonymous' ? '' : override.token
    this.base = (override.baseUrl ?? `/kapi/${SERVICE_NAME}`).replace(/\/+$/, '')
    this.origin = this.base.replace(/\/kapi\/[^/]+$/, '')
  }

  async call<T>(method: string, params: Record<string, unknown> = {}): Promise<T> {
    let response: Response
    try {
      response = await fetch(this.base, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ method, params, sys: this.token ? [this.seq++, this.token] : [this.seq++] }),
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

  upload(body: Blob, meta: UploadMeta) {
    return putUpload(`${this.base}/upload?${uploadQuery(meta)}`, this.token, body)
  }

  contentUrl(objId: ObjId) {
    return contentPath(this.origin, objId, this.token)
  }
}

class ZoneTransport implements Transport {
  readonly mode = 'zone' as const
  private knownToken: string | null = null

  private client() {
    return buckyos.getServiceRpcClient(SERVICE_NAME)
  }

  private async token(): Promise<string> {
    const fromClient = this.client().getSessionToken()
    if (fromClient) return (this.knownToken = fromClient)
    const token = (await buckyos.getAccountInfo())?.session_token
    if (!token) throw new TransportError('protocol', 'no session token')
    return (this.knownToken = token)
  }


  async call<T>(method: string, params: Record<string, unknown> = {}): Promise<T> {
    try {
      return await this.client().call<T, Record<string, unknown>>(method, params)
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error)
      throw new TransportError(/fetch|network|timeout|abort/i.test(message) ? 'network' : 'protocol', message)
    }
  }

  async upload(body: Blob, meta: UploadMeta) {
    const base = buckyos.getZoneServiceURL(SERVICE_NAME).replace(/\/+$/, '')
    return putUpload(`${base}/upload?${uploadQuery(meta)}`, await this.token(), body)
  }

  contentUrl(objId: ObjId) {
    return contentPath('', objId, this.client().getSessionToken() ?? this.knownToken ?? '')
  }
}

let resolved: { transport: Transport | null } | null = null

export function homeStationTransport(): Transport | null {
  if (!resolved) {
    const override = readDevOverride()
    resolved = { transport: override ? new DevOverrideTransport(override) : isMockRuntime() ? null : new ZoneTransport() }
  }
  return resolved.transport
}
