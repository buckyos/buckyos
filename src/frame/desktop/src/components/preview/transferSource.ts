/* Preview sources as BuckyOS standard content (剪贴板规范 §2.2): what dragging or copying the item being previewed,
 * or an archive member, hands to another App. */
import { contentDescriptor, readZoneContent, type ContentHints, type TransferableContentRef, type ZoneContent } from 'buckyos/content'
import { normalizeCyfsPath } from './session'
import { isCyfsPathRef, isObjectIdRef, type ContentRef } from './types'

/** The Zone address of a preview source: a path, an object (an archive member included), or an attachment a host
 * resolves that names its object id. Null for content outside the Zone (a file of this device, a bucket binding, a web
 * picture). */
function transferableRefOf(ref: ContentRef): TransferableContentRef | null {
  if (isCyfsPathRef(ref)) return { kind: 'cyfs-path', path: normalizeCyfsPath(ref.path) }
  if (isObjectIdRef(ref)) return { kind: 'object-id', objectId: ref.objectId, ...(ref.innerPath ? { innerPath: ref.innerPath } : {}) }
  const value = (ref as { value?: unknown }).value
  const objId = value && typeof value === 'object' ? (value as { objId?: unknown }).objId : undefined
  return typeof objId === 'string' && objId ? { kind: 'object-id', objectId: objId } : null
}

/** A preview source as one Zone content reference with its name, MIME type and size as hints; null when it is not
 * Zone content or not a valid reference. */
export function zoneContentOfPreviewSource(ref: ContentRef, hints: ContentHints = {}, kind: 'file' | 'folder' = 'file'): ZoneContent | null {
  const source = transferableRefOf(ref)
  if (!source) return null
  const mime = kind === 'file' ? hints.mime ?? contentDescriptor(source, { name: hints.name }).mime : undefined
  return readZoneContent({ origin: 'zone', kind, source, hints: { ...hints, mime } }) ?? null
}
