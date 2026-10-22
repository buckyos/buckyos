import { localObject, onObjectCachesReset, type ObjectInfo } from '../history/objectAccess'
import { attachmentKindOf, attachmentMediaType, isInlineImageType, type AttachmentKind, type MessageAttachment } from './source'

export interface ReadyObjectState { phase: 'ready'; info: ObjectInfo; contentUrl?: string }

// Resolved attachments by object id and label: a bubble that is re-mounted
// (virtualized history) renders at its final size at once instead of going
// through the loading state and resizing the rows around it.
export const readyObjects = new Map<string, ReadyObjectState>()
const READY_OBJECTS_LIMIT = 256
onObjectCachesReset(() => readyObjects.clear())

export function rememberReadyObject(key: string, state: ReadyObjectState) {
  readyObjects.delete(key)
  readyObjects.set(key, state)
  if (readyObjects.size > READY_OBJECTS_LIMIT) readyObjects.delete(readyObjects.keys().next().value!)
}

export const objectCacheKey = (attachment: MessageAttachment) => `${attachment.context ?? ''}\n${attachment.objId ?? ''}\n${attachment.label}`

export function knownReadyObject(attachment: MessageAttachment): ReadyObjectState | undefined {
  const key = objectCacheKey(attachment)
  const ready = readyObjects.get(key)
  if (ready) return ready
  const local = localObject(attachment.objId)
  if (!local) return undefined
  const state: ReadyObjectState = { phase: 'ready', info: local.info, contentUrl: isInlineImageType(attachmentMediaType(local.info, attachment.label)) ? local.url : undefined }
  rememberReadyObject(key, state)
  return state
}

export function readyObjectKind(state: ReadyObjectState, label: string): AttachmentKind {
  if (!state.info.isFile) return 'file'
  return attachmentKindOf(attachmentMediaType(state.info, label), state.info.name ?? label) ?? 'file'
}

/** The kind of an attachment whose object was already resolved in this page; undefined before that. */
export function knownAttachmentKind(attachment: MessageAttachment): AttachmentKind | undefined {
  const ready = knownReadyObject(attachment)
  return ready ? readyObjectKind(ready, attachment.label) : undefined
}
