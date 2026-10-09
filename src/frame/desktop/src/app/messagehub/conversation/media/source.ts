import { registerPreviewSourceResolver } from '../../../../components/preview/hostSources'
import { detectRuntimeProfile, extensionOf, mediaTypeFromExtension, stripMediaTypeParams } from '../../../../components/preview/mediaTypes'
import {
  PreviewError,
  type ContentRef,
  type PreviewSessionContext,
  type PreviewSessionItemInput,
  type ResolvedPreviewSource,
} from '../../../../components/preview/types'
import { getMessageStableId, type MessageObject, type RefItem } from '../../protocol/msgobj'
import { effectiveContent } from '../history/relations'
import type { ConversationMessageReader } from '../history/types'
import { getObjectAccess, type ObjectInfo } from '../history/objectAccess'

export const MESSAGEHUB_OBJECT_SOURCE = 'messagehub-object'

export interface MessageObjectSourceValue {
  objId?: string
  uri?: string
  name?: string
  reference: string
}

export type AttachmentKind = 'image' | 'video' | 'audio' | 'file'

export interface MessageAttachment {
  id: string
  source: ContentRef
  label: string
  objId?: string
  uri?: string
}

export function isHttpUri(uri: string | undefined): boolean {
  if (!uri) return false
  try {
    const parsed = new URL(uri)
    return parsed.protocol === 'https:' || parsed.protocol === 'http:'
  } catch {
    return false
  }
}

export function isTrustedImageUri(uri: string): boolean {
  try {
    const url = new URL(uri)
    const host = url.hostname.toLowerCase()
    const trusted = host === 'wikimedia.org' || host.endsWith('.wikimedia.org')
    return trusted && /\.(avif|bmp|gif|jpe?g|png|svg|webp)$/i.test(url.pathname)
  } catch {
    return false
  }
}

function uriName(uri: string): string | undefined {
  try {
    return decodeURIComponent(new URL(uri).pathname.split('/').filter(Boolean).pop() ?? '') || undefined
  } catch {
    return undefined
  }
}

export function attachmentKindOf(mimeType: string | undefined, name: string | undefined): AttachmentKind | null {
  const type = stripMediaTypeParams(mimeType) || mediaTypeFromExtension(extensionOf(name)) || ''
  if (!type || type === 'application/octet-stream') return null
  if (type.startsWith('image/')) return 'image'
  if (type.startsWith('video/')) return 'video'
  if (type.startsWith('audio/')) return 'audio'
  return 'file'
}

export function attachmentMediaType(info: Pick<ObjectInfo, 'mimeType' | 'name'> | undefined, label: string): string | undefined {
  return stripMediaTypeParams(info?.mimeType) || mediaTypeFromExtension(extensionOf(info?.name ?? label))
}

export function isInlineImageType(mediaType: string | undefined): boolean {
  return !!mediaType && detectRuntimeProfile().imageMediaTypes.includes(stripMediaTypeParams(mediaType))
}

export function messageAttachments(message: MessageObject, indexHint: number): MessageAttachment[] {
  if (message.ui_unavailable === true) return []
  const messageId = getMessageStableId(message, indexHint)
  const hasAccess = getObjectAccess() !== null
  const out: MessageAttachment[] = []
  ;(effectiveContent(message).refs ?? []).forEach((ref, refIndex) => {
    const attachment = attachmentOfRef(ref, `${messageId}#${refIndex}`, hasAccess)
    if (attachment) out.push(attachment)
  })
  return out
}

export function attachmentOfRef(ref: RefItem, id: string, hasAccess = getObjectAccess() !== null): MessageAttachment | null {
  const target = ref.target
  if (target.type !== 'data_obj') return null
  const hint = target.uri_hint?.trim()
  if (hint && isHttpUri(hint)) {
    if (!isTrustedImageUri(hint)) return null
    const label = ref.label ?? uriName(hint) ?? hint
    const value: MessageObjectSourceValue = { uri: hint, name: label, reference: hint }
    return { id, label, uri: hint, source: { kind: MESSAGEHUB_OBJECT_SOURCE, value } }
  }
  if (!hasAccess) return null
  const label = ref.label ?? target.obj_id
  const value: MessageObjectSourceValue = { objId: target.obj_id, name: label, reference: target.obj_id }
  return { id, label, objId: target.obj_id, source: { kind: MESSAGEHUB_OBJECT_SOURCE, value } }
}

function valueOf(source: ContentRef): MessageObjectSourceValue {
  const value = (source as { value?: unknown }).value
  if (!value || typeof value !== 'object') throw new PreviewError('INVALID_SOURCE', 'Invalid attachment reference')
  return value as MessageObjectSourceValue
}

function accessError(error: unknown): PreviewError {
  const text = error instanceof Error ? error.message : String(error)
  const status = Number(text.split(' ')[0])
  if (status === 401 || status === 403) return new PreviewError('PERMISSION_DENIED', 'You do not have access to this attachment', { detail: text })
  if (status === 404) return new PreviewError('NOT_FOUND', 'This attachment is no longer available', { detail: text })
  if (status === 413) return new PreviewError('TOO_LARGE', 'This attachment is too large to open here', { detail: text })
  if (status === 415) return new PreviewError('UNSUPPORTED', 'This attachment has no downloadable content', { detail: text })
  return new PreviewError('NETWORK', 'The attachment could not be loaded', { detail: text, retryable: true })
}

async function resolveMessageObject(source: ContentRef): Promise<ResolvedPreviewSource> {
  const value = valueOf(source)
  if (value.uri) {
    const name = value.name ?? uriName(value.uri)
    const hint = mediaTypeFromExtension(extensionOf(uriName(value.uri) ?? name))
    return {
      originalSource: source,
      displayName: name,
      objectType: 'file',
      mediaTypeHints: hint ? [hint] : [],
      readRef: { kind: 'url', url: value.uri },
    }
  }
  if (!value.objId) throw new PreviewError('INVALID_SOURCE', 'Invalid attachment reference')
  const access = getObjectAccess()
  if (!access) throw new PreviewError('NOT_FOUND', 'Attachments are not available here')
  const info = await access.describe(value.objId).catch((error: unknown) => {
    throw accessError(error)
  })
  if (!info.isFile) throw new PreviewError('UNSUPPORTED', 'This attachment is not a file')
  const url = await access.contentUrl(value.objId).catch((error: unknown) => {
    throw accessError(error)
  })
  const name = info.name ?? value.name
  const hint = attachmentMediaType(info, name ?? '')
  return {
    originalSource: source,
    sourceObjectId: value.objId,
    inputObjectId: value.objId,
    displayName: name,
    size: info.size,
    objectType: 'file',
    mediaTypeHints: hint ? [hint] : [],
    readRef: { kind: 'url', url },
  }
}

let installed = false

export function installMessageHubPreviewSources() {
  if (installed) return
  installed = true
  registerPreviewSourceResolver(MESSAGEHUB_OBJECT_SOURCE, { resolvePreviewSource: resolveMessageObject })
}

const LIST_WINDOW = 100
const DESCRIBE_TIMEOUT_MS = 2500
const READ_PAGE = 200

async function kindOf(attachment: MessageAttachment): Promise<AttachmentKind | null> {
  const known = attachmentKindOf(undefined, attachment.uri ? uriName(attachment.uri) : attachment.label)
  if (known || !attachment.objId) return known
  const access = getObjectAccess()
  if (!access) return null
  let timer = 0
  const timeout = new Promise<null>(resolve => { timer = window.setTimeout(() => resolve(null), DESCRIBE_TIMEOUT_MS) })
  const described = access.describe(attachment.objId).then(info => (info.isFile ? attachmentKindOf(info.mimeType, info.name ?? attachment.label) : null), () => null)
  const kind = await Promise.race([described, timeout])
  window.clearTimeout(timer)
  return kind
}

export async function listConversationMedia(reader: ConversationMessageReader, current: MessageAttachment): Promise<PreviewSessionItemInput[]> {
  const single = [{ id: current.id, source: current.source, title: current.label }]
  try {
    const all: MessageAttachment[] = []
    for (let start = 0; start < reader.totalCount; start += READ_PAGE) {
      const messages = await reader.readRange(start, READ_PAGE)
      messages.forEach((message, offset) => all.push(...messageAttachments(message, start + offset)))
    }
    const index = all.findIndex(item => item.id === current.id)
    if (index < 0) return single
    const nearby = all.slice(Math.max(0, index - LIST_WINDOW), index + LIST_WINDOW + 1)
    const kinds = await Promise.all(nearby.map(item => (item.id === current.id ? Promise.resolve('image' as const) : kindOf(item))))
    return nearby
      .filter((_, i) => kinds[i] === 'image' || kinds[i] === 'video')
      .map(item => ({ id: item.id, source: item.source, title: item.label }))
  } catch {
    return single
  }
}

export function conversationMediaSession(reader: ConversationMessageReader, current: MessageAttachment): PreviewSessionContext {
  return {
    kind: 'provider',
    sessionId: `messagehub:${reader.readerKey}:${reader.totalCount}:${current.id}`,
    currentItemId: current.id,
    provider: { listItems: () => listConversationMedia(reader, current) },
    navigation: 'bounded',
  }
}
