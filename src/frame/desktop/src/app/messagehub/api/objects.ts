/**
 * Attachment access through the msg-center object route
 * (`GET /kapi/msg-center/objects/{obj_id}[/content]?context_path=…`,
 * bearer session token; `context_path` names the message the object is read
 * through). Results are cached per viewer (`identityKey`), `context_path`
 * and object id; a new viewer drops them and revokes the Blob URLs
 * (`resetObjectCaches`). Collecting copies the attachment into the viewer's
 * `collected` bucket (`msg.collect_attachment`).
 */
import { collectAttachment, currentSessionToken, msgCenterServiceUrl } from '../datamodel/sessionApi'
import { onObjectCachesReset, resetObjectCaches, type ObjectAccess, type ObjectInfo } from '../conversation/history/objectAccess'

const infoCache = new Map<string, Promise<ObjectInfo>>()
const contentCache = new Map<string, Promise<string>>()
let viewer = ''

onObjectCachesReset(() => {
  for (const pending of contentCache.values()) void pending.then(url => URL.revokeObjectURL(url), () => undefined)
  infoCache.clear()
  contentCache.clear()
})

/** Binds the caches to the signed-in subject; a different one starts from nothing. */
export function setObjectViewer(identityKey: string) {
  if (identityKey === viewer) return
  resetObjectCaches()
  viewer = identityKey
}

/** `context` is the `context_path` of the message that shows the object (`objectContextOf`). */
export function objectUrl(objId: string, content = false, context?: string, innerPath?: string): string {
  const base = msgCenterServiceUrl().replace(/\/$/, '')
  const params = new URLSearchParams()
  if (context) params.set('context_path', context)
  if (innerPath) params.set('inner_path', innerPath)
  const query = params.size ? `?${params}` : ''
  return `${base}/objects/${encodeURIComponent(objId)}${content ? '/content' : ''}${query}`
}

export class ObjectRouteError extends Error {
  readonly status: number

  constructor(status: number, text: string) {
    super(`${status} ${text}`.trim())
    this.name = 'ObjectRouteError'
    this.status = status
  }
}

export async function authorizedFetch(url: string): Promise<Response> {
  const token = await currentSessionToken()
  const response = await fetch(url, { headers: token ? { authorization: `Bearer ${token}` } : {}, credentials: 'omit' })
  if (!response.ok) {
    const text = await response.text().catch(() => '')
    throw new ObjectRouteError(response.status, text || response.statusText)
  }
  return response
}

const cacheKey = (objId: string, context: string | undefined) => `${viewer}\n${context ?? ''}\n${objId}`

export const apiObjectAccess: ObjectAccess = {
  describe(objId, context, innerPath) {
    const key = `${cacheKey(objId, context)}\n${innerPath ?? ''}`
    let pending = infoCache.get(key)
    if (!pending) {
      pending = authorizedFetch(objectUrl(objId, false, context, innerPath)).then(async response => {
        const json = await response.json() as Record<string, unknown>
        const isFile = typeof json.content === 'string'
        const isDirectory = typeof json.body === 'object' && json.body !== null
        const meta = json
        return {
          objId,
          isFile,
          isDirectory,
          entries: isDirectory ? Object.keys(json.body as object).sort() : undefined,
          name: typeof meta.name === 'string' ? meta.name : undefined,
          size: typeof meta.size === 'number' ? meta.size : typeof meta.total_size === 'number' ? meta.total_size : undefined,
          mimeType: typeof meta.mime_type === 'string' ? meta.mime_type : undefined,
        } satisfies ObjectInfo
      })
      pending.catch(() => { if (infoCache.get(key) === pending) infoCache.delete(key) })
      infoCache.set(key, pending)
    }
    return pending
  },
  contentUrl(objId, context, innerPath) {
    const key = `${cacheKey(objId, context)}\n${innerPath ?? ''}`
    let pending = contentCache.get(key)
    if (!pending) {
      pending = authorizedFetch(objectUrl(objId, true, context, innerPath)).then(async response => URL.createObjectURL(await response.blob()))
      pending.catch(() => { if (contentCache.get(key) === pending) contentCache.delete(key) })
      contentCache.set(key, pending)
    }
    return pending
  },
  async collect(objId, context) {
    if (!context) throw new ObjectRouteError(404, 'not-found')
    const { bucket_id: bucketId, key } = await collectAttachment(objId, context)
    return { bucketId, key }
  },
}
