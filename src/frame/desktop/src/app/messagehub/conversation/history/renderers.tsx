import { useI18n } from '../../../../i18n/provider'
import { isActionMessage } from '../../sessionModel'
import { memo, useContext, useState } from 'react'
import {
  AlertCircle,
  Check,
  CheckCheck,
  Clock,
} from 'lucide-react'
import {
  getMessageDeliveryStatus,
  getMessageSenderName,
  getMessageStableId,
  type RefItem,
  type DID,
  type MessageDeliveryStatus,
  type MessageObject,
} from '../../protocol/msgobj'
import { MessageAttachmentView } from '../media/MediaAttachment'
import { attachmentOfRef, isHttpUri } from '../media/source'
import { ConversationMessageActionsContext } from './actions'
import { getObjectAccess } from './objectAccess'
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
  messageIndex: number
}

type MessageRenderer = (
  message: MessageObject,
  context: MessageRenderContext,
) => React.ReactNode | null

const messageRenderers: readonly MessageRenderer[] = [
  message => isActionMessage(message) ? <ActionMessage message={message} /> : null,
  message => message.ui_unavailable === true ? <UnavailableMessage message={message} /> : null,
  (message, context) => hasAttachmentRefs(message) ? <AttachmentMessage message={message} context={context} /> : null,
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
      {messageRenderers.map((renderer) => renderer(item.data, { isGroup, selfDid, messageIndex: item.messageIndex })).find(Boolean)}
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
  const metaColor = isSelf ? 'var(--cp-message-self-meta)' : 'var(--cp-muted)'
  return (
    <>
      <div className="flex items-center justify-end gap-1 mt-1">
        {record?.boxKind === 'REQUEST_BOX' ? <span className="mr-auto rounded-full px-1.5 text-[10px]" data-testid="request-chip" style={{ background: 'color-mix(in srgb, var(--cp-warning) 18%, transparent)', color: 'var(--cp-warning)' }}>{t('messagehub.requestShort')}</span> : null}
        <span
          className="text-[10px]"
          style={{
            color: metaColor,
          }}
        >
          {formatMessageTime(message.created_at_ms)}
        </span>
        {isSelf ? <MessageStatusIcon status={deliveryStatus} /> : null}
      </div>
      {isSelf && record?.delivery && (failedTargets.length > 0 || record.delivery.overall === 'partial_failed') ? (
        <div className="mt-1 text-[11px]" data-testid="delivery-failure" style={{ color: metaColor }}>
          <p role="note">{t(record.delivery.overall === 'partial_failed' ? 'messagehub.delivery.partialFailed' : 'messagehub.delivery.failed')} · {t('messagehub.delivery.failedHint', undefined, { count: Math.max(1, failedTargets.length) })}</p>
          {record.delivery.overall === 'failed' && failedTargets.length === (record.delivery.per_target?.length ?? 0) ? <ResendButton message={message} /> : null}
          <details className="mt-1 text-[10px]" data-testid="delivery-details">
            <summary className="cursor-pointer">{t('messagehub.delivery.technical')}</summary>
            <ul className="mt-1 space-y-0.5 break-all">
              {(record.delivery.per_target ?? []).map(target => (
                <li key={target.target_did}>{target.target_did} · {target.state}{target.attempts ? ` · ${t('messagehub.delivery.attempts', undefined, { count: target.attempts })}` : ''}{target.last_error ? ` · ${target.last_error.message}${target.last_error.retryable ? ` (${t('messagehub.delivery.retryable')})` : ''}${target.last_error.duplicate_risk ? ` (${t('messagehub.delivery.duplicateRisk')})` : ''}` : ''}</li>
              ))}
            </ul>
            {pendingTargets.length > 0 ? <p className="mt-1">{t('messagehub.delivery.pending', undefined, { count: pendingTargets.length })}</p> : null}
          </details>
        </div>
      ) : null}
    </>
  )
}

function ResendButton({ message }: { message: MessageObject }) {
  const { t } = useI18n()
  const { resend } = useContext(ConversationMessageActionsContext)
  const [state, setState] = useState<'idle' | 'pending' | 'failed'>('idle')
  if (!resend) return null
  const run = () => {
    setState('pending')
    void resend(message).then(() => setState('idle'), () => setState('failed'))
  }
  return (
    <div className="mt-1 flex flex-wrap items-center gap-2">
      <button type="button" className="min-h-8 rounded-lg border border-current px-2 text-[11px] font-medium disabled:opacity-50" disabled={state === 'pending'} onClick={run} data-testid="message-resend">{t('messagehub.resend')}</button>
      {state === 'failed' ? <span role="alert">{t('messagehub.resendFailed')}</span> : null}
    </div>
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

function hasAttachmentRefs(message: MessageObject): boolean {
  const hasAccess = getObjectAccess() !== null
  return (message.content.refs ?? []).some(ref => {
    if (ref.target.type !== 'data_obj') return false
    const uri = ref.target.uri_hint?.trim()
    return uri && isHttpUri(uri) ? isLikelyImageUri(uri) : hasAccess
  })
}

function isLikelyImageUri(uri: string): boolean {
  try {
    return /\.(avif|bmp|gif|jpe?g|png|svg|webp)$/i.test(new URL(uri).pathname)
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
  const messageId = getMessageStableId(message, context.messageIndex)
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
          {refs.map((ref, index) => <MessageRef key={`${index}:${ref.target.type === 'data_obj' ? ref.target.obj_id : ref.target.did}`} item={ref} id={`${messageId}#${index}`} isSelf={isSelf} />)}
        </div>
        {caption.length > 0 ? <p className="text-sm whitespace-pre-wrap break-words leading-relaxed mt-2">{caption}</p> : null}
        <MessageFooter message={message} isSelf={isSelf} deliveryStatus={deliveryStatus} />
      </div>
    </div>
  )
}

function MessageRef({ item, id, isSelf }: { item: RefItem; id: string; isSelf: boolean }) {
  const target = item.target
  if (target.type === 'service_did') {
    return <span className="text-xs break-all" data-testid="attachment-service">{item.label ? `${item.label} · ` : ''}{target.did} · {item.role}</span>
  }
  const attachment = attachmentOfRef(item, id)
  if (attachment) return <MessageAttachmentView attachment={attachment} isSelf={isSelf} />
  const uri = target.uri_hint?.trim()
  if (uri && isHttpUri(uri)) {
    return <a href={uri} target="_blank" rel="noreferrer noopener" className="text-sm break-all underline underline-offset-2" style={{ color: isSelf ? 'var(--cp-message-self-link)' : 'var(--cp-accent)' }}>{item.label ?? uri}</a>
  }
  return <span className="text-xs break-all">{item.label ?? target.obj_id}</span>
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
  const { t } = useI18n()
  let icon: React.ReactNode
  switch (status) {
    case 'sending':
      icon = <Clock size={14} style={{ color: 'var(--cp-muted)' }} aria-hidden />
      break
    case 'sent':
      icon = <Check size={14} style={{ color: 'var(--cp-muted)' }} aria-hidden />
      break
    case 'delivered':
      icon = <CheckCheck size={14} style={{ color: 'var(--cp-muted)' }} aria-hidden />
      break
    case 'read':
      icon = <CheckCheck size={14} style={{ color: 'var(--cp-accent)' }} aria-hidden />
      break
    case 'failed':
      icon = <AlertCircle size={14} style={{ color: 'var(--cp-danger)' }} aria-hidden />
      break
    default:
      return null
  }
  // A labelled image rather than a live region: the state is read with the
  // bubble, not announced again every time it changes.
  const label = t(`messagehub.deliveryStatus.${status}`)
  return <span role="img" aria-label={label} title={label} className="inline-flex" data-testid="delivery-status" data-status={status}>{icon}</span>
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
