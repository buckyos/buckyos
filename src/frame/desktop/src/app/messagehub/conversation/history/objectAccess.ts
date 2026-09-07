/**
 * Registry for attachment object access. The api store registers a handler
 * that resolves `refs[].target.obj_id` through the msg-center object route;
 * mock mode leaves it unregistered so renderers fall back to `uri_hint`.
 */
export interface ObjectInfo {
  objId: string
  name?: string
  size?: number
  mimeType?: string
  isFile: boolean
}

export interface ObjectAccess {
  describe(objId: string): Promise<ObjectInfo>
  /** Blob URL of the file content (cached per object id). */
  contentUrl(objId: string): Promise<string>
}

let handler: ObjectAccess | null = null

export function registerObjectAccess(access: ObjectAccess | null) {
  handler = access
}

export function getObjectAccess(): ObjectAccess | null {
  return handler
}
