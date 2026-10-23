import { buckyos, ndm } from 'buckyos'
import { isMockRuntime } from '../../../runtime'
import type { ObjId } from '../protocol/feed'

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

export interface UploadOptions {
  signal?: AbortSignal
  onProgress?: (loaded: number, total: number) => void
}

export interface Transport {
  readonly mode: 'zone' | 'dev-override'
  call<T>(method: string, params?: Record<string, unknown>): Promise<T>
  upload(body: File | readonly File[], meta: UploadMeta, options?: UploadOptions): Promise<{ objId: ObjId }>
  contentUrl(objId: ObjId): string
  objectUrl(objId: ObjId): string
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

const imports = new WeakMap<File, { sessionId: string; objId: string }>()

async function importAttachment(transport: Transport, endpoint: string, token: string, body: File | readonly File[], meta: UploadMeta, options: UploadOptions = {}): Promise<{ objId: ObjId }> {
  const files = Array.isArray(body) ? body : [body as File]
  if (!files.length) throw new Error('This browser cannot select an empty folder; use a directory drag-and-drop capable browser.')
  if (files.some(file => file.size > 64 * 1024 * 1024)) throw new Error('Each file must be at most 64 MiB.')
  const first = files[0]
  let imported = imports.get(first)
  if (!imported) {
    const snapshot = await ndm.importFiles(files, { mode: meta.mime === 'application/x-directory' ? 'single_dir' : 'single_file', endpoint, sessionToken: token, signal: options.signal, autoStartUpload: false })
    const object = snapshot.items[0]
    if (!object) throw new Error('No file or folder was selected.')
    imported = { sessionId: snapshot.sessionId, objId: object.objectId }
    imports.set(first, imported)
  }
  const total = files.reduce((sum, file) => sum + file.size, 0)
  const status = await ndm.ensureUploaded(imported.sessionId, { endpoint, sessionToken: token, signal: options.signal, onProgress: progress => options.onProgress?.(progress.uploadedBytes, total) })
  if (status.objectStatus !== 'ready') throw new Error('Attachment object is not ready.')
  options.signal?.throwIfAborted()
  const result = await transport.call<{ objId: ObjId }>('object.register', { objId: imported.objId, metadata: meta })
  options.onProgress?.(total, total)
  ndm.releaseImportSession(imported.sessionId)
  imports.delete(first)
  return result
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

  upload(body: File | readonly File[], meta: UploadMeta, options?: UploadOptions) {
    return importAttachment(this, this.origin, this.token, body, meta, options)
  }

  objectUrl(objId: ObjId) { return this.contentUrl(objId).replace('/content', '') }

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

  async upload(body: File | readonly File[], meta: UploadMeta, options?: UploadOptions) {
    const base = buckyos.getZoneServiceURL(SERVICE_NAME).replace(/\/kapi\/[^/]+\/?$/, '')
    return importAttachment(this, base, await this.token(), body, meta, options)
  }

  objectUrl(objId: ObjId) { return this.contentUrl(objId).replace('/content', '') }

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

export function releaseAttachmentImport(selected: File | readonly File[]) {
  const file = Array.isArray(selected) ? selected[0] : selected as File
  if (!file) return
  const session = imports.get(file)
  if (session) ndm.releaseImportSession(session.sessionId)
  imports.delete(file)
}
