import { useI18n } from '../../../../i18n/provider'
import { isActionMessage } from '../../sessionModel'
import { memo, useEffect, useState } from 'react'
import {
  AlertCircle,
  Check,
  CheckCheck,
  Clock,
  FileText,
} from 'lucide-react'
import {
  getMessageDeliveryStatus,
  getMessageSenderName,
  type RefItem,
  type DID,
  type MessageDeliveryStatus,
  type MessageObject,
} from '../../protocol/msgobj'
import { getObjectAccess, type ObjectInfo } from './objectAccess'
import type { ConversationListItem } from './types'

interface RecordContext {
  boxKind?: string
  direction?: string
  recipientState?: string
  delivery?: { overall?: string; per_target?: Array<{ target_did: string; state: string; attempts?: number; external_msg_id?: string; last_error?: { message: string; retryable?: boolean; duplicate_risk?: boolean } }> }
}

function getRecordContext(message: MessageObject): RecordContext | undefined {
  const value = message.ui_record
  return value && typeof value === 'object' ? value as RecordContext : undefined
}

interface MessageRenderContext {
  isGroup: boolean
  selfDid: DID
}

type MessageRenderer = (
  message: MessageObject,
  context: MessageRenderContext,
) => React.ReactNode | null

const messageRenderers: readonly MessageRenderer[] = [
  message => isActionMessage(message) ? <ActionMessage message={message} /> : null,
  message => message.ui_unavailable === true ? <UnavailableMessage message={message} /> : null,
  (message, context) => hasObjectRefs(message) ? <AttachmentMessage message={message} context={context} /> : null,
  renderImageMessage,
  renderTextMessage,
  renderFallbackMessage,
]

export const ConversationListRow = memo(function ConversationListRow({
  item,
  isGroup,
  selfDid,
}: {
  item: ConversationListItem
  isGroup: boolean
  selfDid: DID
}) {
  if (item.kind === 'timestamp') {
    return (
      <div className="flex justify-center py-3">
        <span
          className="px-3 py-1 rounded-full text-xs font-medium"
          style={{
            background: 'color-mix(in srgb, var(--cp-text) 8%, transparent)',
            color: 'var(--cp-muted)',
          }}
        >
          {formatDateSeparator(item.date)}
        </span>
      </div>
    )
  }

  if (item.kind === 'status') {
    return (
      <div className="flex justify-center py-2">
        <span
          className="px-3 py-1 rounded-full text-xs"
          style={{
            background: 'color-mix(in srgb, var(--cp-text) 6%, transparent)',
            color: 'var(--cp-muted)',
          }}
        >
          {item.label}
        </span>
      </div>
    )
  }

  return (
    <>
      {messageRenderers.map((renderer) => renderer(item.data, { isGroup, selfDid })).find(Boolean)}
    </>
  )
})

ConversationListRow.displayName = 'ConversationListRow'

function renderTextMessage(
  message: MessageObject,
  { isGroup, selfDid }: MessageRenderContext,
) {
  const format = message.content.format ?? 'text/plain'

  if (
    format !== 'text/plain'
    && format !== 'text/markdown'
    && format !== 'text/html'
  ) {
    return null
  }

  const isSelf = message.from === selfDid
  const senderName = getMessageSenderName(message)
  const deliveryStatus = getMessageDeliveryStatus(message)

  return (
    <div
      className={`flex ${isSelf ? 'justify-end' : 'justify-start'} mb-1`}
      key={`${message.from}:${message.created_at_ms}`}
    >
      <div
        className="max-w-[75%] min-w-[80px]"
        style={{
          background: isSelf
            ? 'var(--cp-message-self-bg)'
            : 'color-mix(in srgb, var(--cp-text) 8%, transparent)',
          color: isSelf ? 'var(--cp-message-self-text)' : 'var(--cp-text)',
          borderRadius: isSelf
            ? '18px 18px 4px 18px'
            : '18px 18px 18px 4px',
          padding: '8px 12px',
        }}
      >
        {!isSelf && isGroup ? (
          <p
            className="text-xs font-semibold mb-1"
            style={{ color: 'var(--cp-accent)' }}
          >
            {senderName}
          </p>
        ) : null}
        <p className="text-sm whitespace-pre-wrap break-words leading-relaxed">
          {message.content.content}
        </p>
        <MessageFooter message={message} isSelf={isSelf} deliveryStatus={deliveryStatus} />
      </div>
    </div>
  )
}

function MessageFooter({ message, isSelf, deliveryStatus }: { message: MessageObject; isSelf: boolean; deliveryStatus?: MessageDeliveryStatus }) {
  const { t } = useI18n()
  const record = getRecordContext(message)
  const failedTargets = record?.delivery?.per_target?.filter(target => target.state === 'FAILED' || target.state === 'DEAD') ?? []
  const pendingTargets = record?.delivery?.per_target?.filter(target => target.state === 'WAIT' || target.state === 'SENDING') ?? []
  return (
    <>
      <div className="flex items-center justify-end gap-1 mt-1">
        {record?.boxKind === 'REQUEST_BOX' ? <span className="mr-auto rounded-full px-1.5 text-[10px]" data-testid="request-chip" style={{ background: 'color-mix(in srgb, var(--cp-warning) 18%, transparent)', color: 'var(--cp-warning)' }}>{t('messagehub.requestShort')}</span> : null}
        <span
          className="text-[10px]"
          style={{
            color: isSelf
              ? 'var(--cp-message-self-meta)'
              : 'var(--cp-muted)',
          }}
        >
          {formatMessageTime(message.created_at_ms)}
        </span>
        {isSelf ? <MessageStatusIcon status={deliveryStatus} /> : null}
      </div>
      {isSelf && record?.delivery && (failedTargets.length > 0 || record.delivery.overall === 'partial_failed') ? (
        <details className="mt-1 text-[10px]" data-testid="delivery-details" style={{ color: isSelf ? 'var(--cp-message-self-meta)' : 'var(--cp-muted)' }}>
          <summary className="cursor-pointer">{t(record.delivery.overall === 'partial_failed' ? 'messagehub.delivery.partialFailed' : 'messagehub.delivery.failed')}</summary>
          <ul className="mt-1 space-y-0.5 break-all">
            {(record.delivery.per_target ?? []).map(target => (
              <li key={target.target_did}>{target.target_did} · {target.state}{target.attempts ? ` · ${t('messagehub.delivery.attempts', undefined, { count: target.attempts })}` : ''}{target.last_error ? ` · ${target.last_error.message}${target.last_error.retryable ? ` (${t('messagehub.delivery.retryable')})` : ''}${target.last_error.duplicate_risk ? ` (${t('messagehub.delivery.duplicateRisk')})` : ''}` : ''}</li>
            ))}
          </ul>
          {pendingTargets.length > 0 ? <p className="mt-1">{t('messagehub.delivery.pending', undefined, { count: pendingTargets.length })}</p> : null}
        </details>
      ) : null}
    </>
  )
}

function UnavailableMessage({ message }: { message: MessageObject }) {
  const { t } = useI18n()
  const record = getRecordContext(message)
  return (
    <div className="mx-auto my-2 max-w-xl rounded-lg px-3 py-2 text-center text-xs" data-testid="message-unavailable" style={{ background: 'color-mix(in srgb, var(--cp-danger) 8%, transparent)', color: 'var(--cp-muted)' }}>
      {t('messagehub.messageUnavailable')} · {formatMessageTime(message.created_at_ms)}{record?.boxKind ? ` · ${record.boxKind}` : ''}
    </div>
  )
}

function hasObjectRefs(message: MessageObject): boolean {
  return (message.content.refs ?? []).some(ref => ref.target.type === 'data_obj' && !isHttpUri(ref.target.uri_hint)) && getObjectAccess() !== null
}

function isHttpUri(uri: string | undefined): boolean {
  if (!uri) return false
  try {
    const parsed = new URL(uri)
    return parsed.protocol === 'https:' || parsed.protocol === 'http:'
  } catch {
    return false
  }
}

function AttachmentMessage({ message, context }: { message: MessageObject; context: MessageRenderContext }) {
  const isSelf = message.from === context.selfDid
  const senderName = getMessageSenderName(message)
  const deliveryStatus = getMessageDeliveryStatus(message)
  const caption = message.content.content?.trim() ?? ''
  const refs = message.content.refs ?? []
  return (
    <div className={`flex ${isSelf ? 'justify-end' : 'justify-start'} mb-1`}>
      <div
        className="max-w-[75%] min-w-[160px]"
        style={{
          background: isSelf ? 'var(--cp-message-self-bg)' : 'color-mix(in srgb, var(--cp-text) 8%, transparent)',
          color: isSelf ? 'var(--cp-message-self-text)' : 'var(--cp-text)',
          borderRadius: isSelf ? '18px 18px 4px 18px' : '18px 18px 18px 4px',
          padding: '8px 12px',
        }}
      >
        {!isSelf && context.isGroup ? <p className="text-xs font-semibold mb-1" style={{ color: 'var(--cp-accent)' }}>{senderName}</p> : null}
        <div className="flex flex-col gap-2">
          {refs.map((ref, index) => <AttachmentRef key={`${index}:${ref.target.type === 'data_obj' ? ref.target.obj_id : ref.target.did}`} item={ref} isSelf={isSelf} />)}
        </div>
        {caption.length > 0 ? <p className="text-sm whitespace-pre-wrap break-words leading-relaxed mt-2">{caption}</p> : null}
        <MessageFooter message={message} isSelf={isSelf} deliveryStatus={deliveryStatus} />
      </div>
    </div>
  )
}

type AttachmentState =
  | { phase: 'loading' }
  | { phase: 'ready'; info: ObjectInfo; contentUrl?: string }
  | { phase: 'error'; message: string }

function AttachmentRef({ item, isSelf }: { item: RefItem; isSelf: boolean }) {
  const { t } = useI18n()
  const linkColor = isSelf ? 'var(--cp-message-self-link)' : 'var(--cp-accent)'
  const target = item.target
  const objId = target.type === 'data_obj' ? target.obj_id : null
  const httpUri = target.type === 'data_obj' && isHttpUri(target.uri_hint) ? target.uri_hint : null
  const [state, setState] = useState<AttachmentState>({ phase: 'loading' })
  const [retry, setRetry] = useState(0)
  useEffect(() => {
    if (!objId || httpUri) return
    const access = getObjectAccess()
    let cancelled = false
    // Resolution is asynchronous by design; state changes only land in the
    // promise callbacks so the effect body never sets state synchronously.
    const resolve = access
      ? access.describe(objId)
      : Promise.reject(new Error('unavailable'))
    void resolve.then(async (info) => {
      if (cancelled) return
      const previewable = info.isFile && (info.mimeType?.startsWith('image/') ?? false)
      const contentUrl = previewable && access ? await access.contentUrl(objId).catch(() => undefined) : undefined
      if (!cancelled) setState({ phase: 'ready', info, contentUrl })
    }).catch((error: unknown) => {
      if (!cancelled) setState({ phase: 'error', message: error instanceof Error ? error.message : String(error) })
    })
    return () => { cancelled = true }
  }, [objId, httpUri, retry, t])
  if (target.type === 'service_did') {
    return <span className="text-xs break-all" data-testid="attachment-service">{item.label ? `${item.label} · ` : ''}{target.did} · {item.role}</span>
  }
  if (httpUri) {
    return <a href={httpUri} target="_blank" rel="noreferrer noopener" className="text-sm break-all underline underline-offset-2" style={{ color: linkColor }}>{item.label ?? httpUri}</a>
  }
  const label = item.label ?? (state.phase === 'ready' ? state.info.name : undefined) ?? objId ?? ''
  if (state.phase === 'loading') return <span className="text-xs" data-testid="attachment-loading">{t('messagehub.attachmentLoading')} · {label}</span>
  if (state.phase === 'error') {
    return <span className="text-xs break-all" role="alert" data-testid="attachment-error">{label} · {t('messagehub.attachmentUnavailable')} <button type="button" className="underline" onClick={() => { setState({ phase: 'loading' }); setRetry(value => value + 1) }}>{t('messagehub.retry')}</button></span>
  }
  const openContent = async () => {
    const access = getObjectAccess()
    if (!access || !objId) return
    try {
      const url = await access.contentUrl(objId)
      window.open(url, '_blank', 'noopener')
    } catch {
      setState({ phase: 'error', message: 'download' })
    }
  }
  return (
    <div className="flex flex-col gap-1" data-testid="attachment-ready">
      {state.contentUrl ? <img src={state.contentUrl} alt={label} className="block w-full h-auto max-h-[360px] object-cover" style={{ borderRadius: 12 }} /> : null}
      <button type="button" onClick={() => void openContent()} disabled={!state.info.isFile} className="flex items-center gap-2 text-left text-sm underline-offset-2 disabled:no-underline disabled:opacity-70" style={{ color: linkColor }}>
        <FileText size={14} />
        <span className="break-all">{label}{state.info.size !== undefined ? ` · ${formatBytes(state.info.size)}` : ''}{state.info.mimeType ? ` · ${state.info.mimeType}` : ''}{!state.info.isFile ? ` · ${t('messagehub.attachmentNotFile')}` : ''}</span>
      </button>
    </div>
  )
}

function formatBytes(size: number): string {
  if (size < 1024) return `${size} B`
  if (size < 1024 * 1024) return `${Math.round(size / 102.4) / 10} KB`
  return `${Math.round(size / (1024 * 102.4)) / 10} MB`
}

function renderImageMessage(
  message: MessageObject,
  { isGroup, selfDid }: MessageRenderContext,
) {
  const imageRefs = getImageRefs(message)
  if (imageRefs.length === 0) {
    return null
  }

  const isSelf = message.from === selfDid
  const senderName = getMessageSenderName(message)
  const deliveryStatus = getMessageDeliveryStatus(message)
  const caption = message.content.content?.trim() ?? ''

  return (
    <div
      className={`flex ${isSelf ? 'justify-end' : 'justify-start'} mb-1`}
      key={`${message.from}:${message.created_at_ms}:image`}
    >
      <div
        className="max-w-[75%] min-w-[120px]"
        style={{
          background: isSelf
            ? 'var(--cp-message-self-bg)'
            : 'color-mix(in srgb, var(--cp-text) 8%, transparent)',
          color: isSelf ? 'var(--cp-message-self-text)' : 'var(--cp-text)',
          borderRadius: isSelf
            ? '18px 18px 4px 18px'
            : '18px 18px 18px 4px',
          padding: '8px 12px',
        }}
      >
        {!isSelf && isGroup ? (
          <p
            className="text-xs font-semibold mb-1"
            style={{ color: 'var(--cp-accent)' }}
          >
            {senderName}
          </p>
        ) : null}
        <div className="flex flex-col gap-2">
          {imageRefs.map((imageRef, index) => (
            <ImageRefPreview
              key={`${imageRef.uri}:${index}`}
              imageRef={imageRef}
              isSelf={isSelf}
            />
          ))}
        </div>
        {caption.length > 0 ? (
          <p className="text-sm whitespace-pre-wrap break-words leading-relaxed mt-2">
            {caption}
          </p>
        ) : null}
        <div className="flex items-center justify-end gap-1 mt-1">
          <span
            className="text-[10px]"
            style={{
              color: isSelf
                ? 'var(--cp-message-self-meta)'
                : 'var(--cp-muted)',
            }}
          >
            {formatMessageTime(message.created_at_ms)}
          </span>
          {isSelf ? <MessageStatusIcon status={deliveryStatus} /> : null}
        </div>
      </div>
    </div>
  )
}

function renderFallbackMessage(
  message: MessageObject,
  { selfDid }: MessageRenderContext,
) {
  const isSelf = message.from === selfDid

  return (
    <div
      className={`flex ${isSelf ? 'justify-end' : 'justify-start'} mb-1`}
      key={`${message.from}:${message.created_at_ms}:fallback`}
    >
      <div
        className="max-w-[75%] min-w-[120px]"
        style={{
          background: 'color-mix(in srgb, var(--cp-text) 8%, transparent)',
          color: 'var(--cp-text)',
          borderRadius: '18px',
          padding: '8px 12px',
        }}
      >
        <p className="text-xs font-semibold mb-1" style={{ color: 'var(--cp-muted)' }}>
          {message.content.format ?? 'unknown content'}
        </p>
        <pre className="text-xs whitespace-pre-wrap break-words leading-relaxed">
          {message.content.content}
        </pre>
      </div>
    </div>
  )
}

function MessageStatusIcon({
  status,
}: {
  status?: MessageDeliveryStatus
}) {
  switch (status) {
    case 'sending':
      return <Clock size={14} style={{ color: 'var(--cp-muted)' }} />
    case 'sent':
      return <Check size={14} style={{ color: 'var(--cp-muted)' }} />
    case 'delivered':
      return <CheckCheck size={14} style={{ color: 'var(--cp-muted)' }} />
    case 'read':
      return <CheckCheck size={14} style={{ color: 'var(--cp-accent)' }} />
    case 'failed':
      return <AlertCircle size={14} style={{ color: 'var(--cp-danger)' }} />
    default:
      return null
  }
}

function ImageRefPreview({
  imageRef,
  isSelf,
}: {
  imageRef: ImageRefDescriptor
  isSelf: boolean
}) {
  const linkColor = isSelf ? 'var(--cp-message-self-link)' : 'var(--cp-accent)'

  if (!imageRef.isTrusted) {
    return (
      <a
        href={imageRef.uri}
        target="_blank"
        rel="noreferrer noopener"
        className="text-sm break-all underline underline-offset-2"
        style={{ color: linkColor }}
      >
        {imageRef.label ?? imageRef.uri}
      </a>
    )
  }

  return (
    <a
      href={imageRef.uri}
      target="_blank"
      rel="noreferrer noopener"
      className="block"
    >
      <img
        src={imageRef.uri}
        alt={imageRef.label ?? 'Image preview'}
        className="block w-full h-auto max-h-[360px] object-cover"
        style={{
          borderRadius: 12,
          background: 'color-mix(in srgb, var(--cp-text) 6%, transparent)',
        }}
      />
    </a>
  )
}

interface ImageRefDescriptor {
  uri: string
  label?: string
  isTrusted: boolean
}

function getImageRefs(message: MessageObject): ImageRefDescriptor[] {
  return (message.content.refs ?? [])
    .map(resolveImageRef)
    .filter((value): value is ImageRefDescriptor => value !== null)
}

function resolveImageRef(ref: RefItem): ImageRefDescriptor | null {
  if (ref.target.type !== 'data_obj' || typeof ref.target.uri_hint !== 'string') {
    return null
  }

  const uri = ref.target.uri_hint.trim()
  if (!isLikelyImageUri(uri)) {
    return null
  }

  return {
    uri,
    label: ref.label,
    isTrusted: isTrustedImageHost(uri),
  }
}

function isLikelyImageUri(uri: string): boolean {
  try {
    const url = new URL(uri)
    if (url.protocol !== 'https:' && url.protocol !== 'http:') {
      return false
    }

    return /\.(avif|bmp|gif|jpe?g|png|svg|webp)$/i.test(url.pathname)
  }
  catch {
    return false
  }
}

function isTrustedImageHost(uri: string): boolean {
  try {
    const host = new URL(uri).hostname.toLowerCase()
    return host === 'wikimedia.org' || host.endsWith('.wikimedia.org')
  }
  catch {
    return false
  }
}

function formatMessageTime(ts: number): string {
  return new Date(ts).toLocaleTimeString([], {
    hour: '2-digit',
    minute: '2-digit',
  })
}

function formatDateSeparator(date: Date): string {
  const today = new Date()
  const yesterday = new Date()
  yesterday.setDate(yesterday.getDate() - 1)

  if (date.toDateString() === today.toDateString()) {
    return 'Today'
  }

  if (date.toDateString() === yesterday.toDateString()) {
    return 'Yesterday'
  }

  return date.toLocaleDateString(undefined, {
    month: 'short',
    day: 'numeric',
    year: date.getFullYear() !== today.getFullYear() ? 'numeric' : undefined,
  })
}

function ActionMessage({ message }: { message: MessageObject }) {
  const { t } = useI18n()
  const data = message.content.machine?.data
  let summary = message.content.content || t('messagehub.action.unknown')
  if (data?.schema_version === 1 && typeof data.action === 'string') {
    const actor = typeof data.actor_did === 'string' ? data.actor_did : t('messagehub.action.unknownActor')
    const subject = typeof data.subject_did === 'string' ? data.subject_did : t('messagehub.action.unknownMember')
    const actions = ['entity.member_joined', 'entity.member_left', 'entity.member_removed', 'session.title_changed', 'session.member_state_changed', 'session.shared_state_changed']
    if (actions.includes(data.action)) summary = t(`messagehub.action.${data.action}`, undefined, { actor, subject })
  } else if (data?.schema_version !== 1) summary = `${t('messagehub.action.unsupported')} · ${summary}`
  return <div data-testid="action-message" className="mx-auto my-2 max-w-xl rounded-lg bg-[color:color-mix(in_srgb,var(--cp-text)_5%,transparent)] px-3 py-2 text-center text-xs text-[color:var(--cp-muted)] break-words">{summary}</div>
}
