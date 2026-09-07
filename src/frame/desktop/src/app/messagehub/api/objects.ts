/**
 * Attachment access through the msg-center object route
 * (`GET /kapi/msg-center/objects/{obj_id}[/content]`, bearer session token).
 * Blob URLs are cached per object id for the page lifetime.
 */
import { currentSessionToken, msgCenterServiceUrl } from '../datamodel/sessionApi'
import type { ObjectAccess, ObjectInfo } from '../conversation/history/objectAccess'

const infoCache = new Map<string, Promise<ObjectInfo>>()
const contentCache = new Map<string, Promise<string>>()

export function objectUrl(objId: string, content = false): string {
  const base = msgCenterServiceUrl().replace(/\/$/, '')
  return `${base}/objects/${encodeURIComponent(objId)}${content ? '/content' : ''}`
}

async function authorizedFetch(url: string): Promise<Response> {
  const token = await currentSessionToken()
  const response = await fetch(url, { headers: token ? { authorization: `Bearer ${token}` } : {}, credentials: 'omit' })
  if (!response.ok) {
    const text = await response.text().catch(() => '')
    throw new Error(`${response.status} ${text || response.statusText}`.trim())
  }
  return response
}

export const apiObjectAccess: ObjectAccess = {
  describe(objId) {
    let pending = infoCache.get(objId)
    if (!pending) {
      pending = authorizedFetch(objectUrl(objId)).then(async response => {
        const json = await response.json() as Record<string, unknown>
        const isFile = objId.startsWith('cyfile:')
        const meta = json
        return {
          objId,
          isFile,
          name: typeof meta.name === 'string' ? meta.name : undefined,
          size: typeof meta.size === 'number' ? meta.size : undefined,
          mimeType: typeof meta.mime_type === 'string' ? meta.mime_type : undefined,
        } satisfies ObjectInfo
      })
      pending.catch(() => infoCache.delete(objId))
      infoCache.set(objId, pending)
    }
    return pending
  },
  contentUrl(objId) {
    let pending = contentCache.get(objId)
    if (!pending) {
      pending = authorizedFetch(objectUrl(objId, true)).then(async response => URL.createObjectURL(await response.blob()))
      pending.catch(() => contentCache.delete(objId))
      contentCache.set(objId, pending)
    }
    return pending
  },
}
