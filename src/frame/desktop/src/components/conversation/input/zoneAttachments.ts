/* Zone content as draft attachments (剪贴板规范 §2.2, §7 case 3): a Zone file or folder dropped or pasted into the
 * composer, or handed over by the Shell, is read with the user's own session into the same `File`-based draft as a
 * file of this device. The reference grants nothing: a read the user may not do fails for that item alone. */
import { contentRefString, extensionOf, mediaTypeFromExtension, type ZoneContent } from 'buckyos/content'
import { filesFromInputList, MAX_ATTACHMENT_BYTES, type ComposerAttachmentInput } from './attachmentDraft'

export type ZoneAttachFailure = 'denied' | 'not_found' | 'too_large' | 'empty_folder' | 'folder_object' | 'cancelled' | 'other'

export class ZoneAttachError extends Error {
  readonly failure: ZoneAttachFailure
  constructor(failure: ZoneAttachFailure, message: string = failure) {
    super(message)
    this.name = 'ZoneAttachError'
    this.failure = failure
  }
}

/** One item's outcome, as the Shell's delivery receipt names it (`accepted`: in the draft, not sent). */
export type ZoneAttachResult = { status: 'accepted' | 'skipped' | 'cancelled' } | { status: 'failed'; failure: ZoneAttachFailure; error: string }

/** The draft key of a Zone item: its address (a path follows the file, an object id is fixed). */
export const zoneAttachmentKey = (item: ZoneContent): string => contentRefString(item.source)

export function zoneItemName(item: ZoneContent): string {
  if (item.hints.name) return item.hints.name
  const tail = item.source.kind === 'cyfs-path' ? item.source.path : item.source.innerPath ?? item.source.objectId
  return tail.split('/').filter(Boolean).pop() ?? contentRefString(item.source)
}

const fileOf = (blob: Blob, name: string, hint?: string): File =>
  new File([blob], name, { type: mediaTypeFromExtension(extensionOf(name)) ?? (blob.type || hint || '') })

/** What a failed read means for the user; the message is the service's own words for anything else. */
export function zoneAttachFailure(error: unknown): { failure: ZoneAttachFailure; message: string } {
  if (error instanceof ZoneAttachError) return { failure: error.failure, message: error.message }
  if (error instanceof DOMException && error.name === 'AbortError') return { failure: 'cancelled', message: error.message }
  const record = error && typeof error === 'object' ? error as { code?: unknown; fallback?: unknown; message?: unknown } : {}
  const code = typeof record.code === 'string' ? record.code : ''
  const message = typeof record.fallback === 'string' ? record.fallback : typeof record.message === 'string' ? record.message : String(error)
  if (code === 'CANCELLED') return { failure: 'cancelled', message }
  if (code === 'PERMISSION_DENIED' || code === 'UNAUTHENTICATED' || code === 'SUDO_REQUIRED') return { failure: 'denied', message }
  if (code === 'NOT_FOUND' || code === 'STALE') return { failure: 'not_found', message }
  if (code === 'TOO_LARGE') return { failure: 'too_large', message }
  return { failure: 'other', message }
}

/**
 * The bytes of one Zone item as a draft attachment: a file as itself, a folder as one directory attachment (its
 * empty subfolders are not listed by the Zone reader and stay out). Paths are read like any other read of the path
 * (`readZoneFiles` / `readZoneFolder`), object ids through the Preview provider — both with the user's own session.
 */
export async function readZoneAttachment(item: ZoneContent, signal?: AbortSignal): Promise<ComposerAttachmentInput> {
  const zoneKey = zoneAttachmentKey(item)
  const name = zoneItemName(item)
  if (item.kind === 'file' && item.hints.size !== undefined && item.hints.size > MAX_ATTACHMENT_BYTES) throw new ZoneAttachError('too_large')
  if (item.source.kind === 'cyfs-path') {
    const [{ installFileBrowserData }, { readZoneFiles, readZoneFolder }] = await Promise.all([
      import('../../../app/filebrowser/data/install'),
      import('../../../sysdlg/select/zoneFiles'),
    ])
    installFileBrowserData()
    if (item.kind === 'folder') {
      const files = await readZoneFolder(item, signal)
      const [input] = filesFromInputList(files)
      if (!input) throw new ZoneAttachError('empty_folder')
      return { ...input, zoneKey }
    }
    const [file] = await readZoneFiles([item], signal)
    return { file, zoneKey }
  }
  if (item.kind === 'folder') throw new ZoneAttachError('folder_object')
  const [{ ensurePreviewProvider }, { readAll }] = await Promise.all([import('../../preview/provider'), import('../../preview/io')])
  const provider = await ensurePreviewProvider()
  const source = { kind: 'object-id' as const, objectId: item.source.objectId, ...(item.source.innerPath ? { innerPath: item.source.innerPath } : {}) }
  const resolved = await provider.resolvePreviewSource(source, { signal })
  if (resolved.objectType === 'dir') throw new ZoneAttachError('folder_object')
  if (resolved.size !== undefined && resolved.size > MAX_ATTACHMENT_BYTES) throw new ZoneAttachError('too_large')
  const blob = await readAll(resolved.readRef, MAX_ATTACHMENT_BYTES, signal)
  return { file: fileOf(blob, item.hints.name ?? resolved.displayName ?? name, item.hints.mime), zoneKey }
}
