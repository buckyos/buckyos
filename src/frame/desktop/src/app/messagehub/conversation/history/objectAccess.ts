/**
 * Registry for attachment object access. The api store registers a handler
 * that resolves `refs[].target.obj_id` through the msg-center object route;
 * mock mode leaves it unregistered so renderers fall back to `uri_hint`.
 *
 * `context` is msg-center's `context_path` of the object: the message that
 * carries it, prefixed with the session in a hosted group session
 * (`[<session MailboxAddress>/]<msg ObjId>`, see `objectContextOf`).
 * msg-center serves an attachment only through the message it is read with.
 */
export interface ObjectInfo {
  objId: string
  name?: string
  size?: number
  mimeType?: string
  isFile: boolean
  isDirectory?: boolean
  entries?: string[]
  /** Described by another Zone; the bytes come only when asked for (a click, not a card). */
  remote?: boolean
}

/** A refused read of the object route: its status, its body and when to ask again. */
export class ObjectRouteError extends Error {
  readonly status: number
  readonly reason: string
  readonly retryAfterSeconds?: number

  constructor(status: number, text: string, retryAfterSeconds?: number) {
    super(`${status} ${text}`.trim())
    this.name = 'ObjectRouteError'
    this.status = status
    this.reason = text
    this.retryAfterSeconds = retryAfterSeconds
  }
}

export type ObjectFailure = 'awaiting_acceptance' | 'denied' | 'not_found' | 'too_large' | 'directory' | 'unsupported' | 'corrupt' | 'fetching' | 'unreachable' | 'other'

export function objectFailure(error: unknown): ObjectFailure {
  const text = error instanceof Error ? error.message : String(error)
  const reason = error instanceof ObjectRouteError ? error.reason : text.slice(text.indexOf(' ') + 1)
  const status = error instanceof ObjectRouteError ? error.status : Number(text.split(' ')[0])
  if (reason === 'attachment-awaiting-acceptance') return 'awaiting_acceptance'
  if (reason === 'attachment-directory-unsupported') return 'directory'
  if (reason === 'attachment-verification-failed') return 'corrupt'
  if (reason === 'attachment-fetching') return 'fetching'
  if (reason === 'attachment-source-unreachable') return 'unreachable'
  if (status === 401 || status === 403) return 'denied'
  if (status === 404) return 'not_found'
  if (status === 413) return 'too_large'
  if (status === 415) return 'unsupported'
  return 'other'
}

/** Attempts after the first: a pull underway gets minutes, a Zone not answering does not. */
const RETRIES: Partial<Record<ObjectFailure, number>> = { fetching: 12, unreachable: 2 }

/**
 * A read that honours the route's `Retry-After`, a bounded number of times:
 * the bytes may still be on their way from another Zone. `wanted` ends the
 * wait when nobody is left to see the answer.
 */
export async function retryingObjectRead<T>(read: () => Promise<T>, wanted: () => boolean = () => true): Promise<T> {
  const used = new Map<ObjectFailure, number>()
  for (;;) {
    try {
      return await read()
    } catch (error) {
      const failure = objectFailure(error)
      const seconds = error instanceof ObjectRouteError ? error.retryAfterSeconds : undefined
      const count = used.get(failure) ?? 0
      if (seconds === undefined || count >= (RETRIES[failure] ?? 0) || !wanted()) throw error
      used.set(failure, count + 1)
      await new Promise(resolve => setTimeout(resolve, seconds * 1000))
    }
  }
}

/** Where a collected attachment is kept: a key of the viewer's `collected` bucket. */
export interface CollectedObject {
  bucketId: string
  key: string
}

export interface ObjectAccess {
  describe(objId: string, context?: string, innerPath?: string): Promise<ObjectInfo>
  /** Blob URL of the file content (cached per viewer, context and object id). */
  contentUrl(objId: string, context?: string, innerPath?: string): Promise<string>
  /** Copies the attachment into the viewer's collection; absent where the viewer has none. */
  collect?(objId: string, context?: string): Promise<CollectedObject>
}

export interface LocalObject {
  info: ObjectInfo
  url: string
}

let handler: ObjectAccess | null = null
let wrapped: ObjectAccess | null = null
const localObjects = new Map<string, LocalObject>()
let localIds = new WeakMap<Blob, string>()
const cacheResets = new Set<() => void>()

export function registerObjectAccess(access: ObjectAccess | null) {
  handler = access
  wrapped = access ? {
    describe: (objId, context, innerPath) => {
      const local = innerPath ? undefined : localObjects.get(objId)
      return local ? Promise.resolve(local.info) : access.describe(objId, context, innerPath)
    },
    contentUrl: (objId, context, innerPath) => {
      const local = innerPath ? undefined : localObjects.get(objId)
      return local ? Promise.resolve(local.url) : access.contentUrl(objId, context, innerPath)
    },
    ...(access.collect ? { collect: access.collect.bind(access) } : {}),
  } : null
}

export function getObjectAccess(): ObjectAccess | null {
  return handler ? wrapped : null
}

/** A cache of resolved objects to drop with `resetObjectCaches`. */
export function onObjectCachesReset(reset: () => void) {
  cacheResets.add(reset)
}

/**
 * Drops every resolved object and revokes its Blob URL: another subject
 * (identity change, sign-out) must never see the previous one's attachments.
 */
export function resetObjectCaches() {
  for (const local of localObjects.values()) URL.revokeObjectURL(local.url)
  localObjects.clear()
  localIds = new WeakMap()
  cacheResets.forEach(reset => reset())
}

export function localObject(objId: string | undefined): LocalObject | undefined {
  return objId ? localObjects.get(objId) : undefined
}

/** `context_path` of the files the display of the hosted group Session at `session` shows (EXT-04). */
export function displayContext(session: string): string {
  return `${session}/_display`
}

export function isLocalObjectId(objId: string): boolean {
  return objId.startsWith('local:')
}

export function rememberLocalObject(objId: string, file: Blob, name: string): LocalObject {
  const previous = localObjects.get(objId)
  if (previous) return previous
  const sameFile = localIds.get(file)
  const url = (sameFile && localObjects.get(sameFile)?.url) || URL.createObjectURL(file)
  const entry: LocalObject = { info: { objId, name, size: file.size, mimeType: file.type || undefined, isFile: true }, url }
  localObjects.set(objId, entry)
  return entry
}

export function localObjectIdFor(file: Blob, name: string): string {
  let objId = localIds.get(file)
  if (!objId) {
    objId = `local:${crypto.randomUUID()}`
    localIds.set(file, objId)
  }
  rememberLocalObject(objId, file, name)
  return objId
}
