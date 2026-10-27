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
