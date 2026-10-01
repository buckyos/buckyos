import { useI18n } from '../../../../i18n/provider'
import { isActionMessage } from '../../sessionModel'
import { groupErrorText, parseGroupNotice } from '../../groupModel'
import { memo, useContext, useState } from 'react'
import {
  AlertCircle,
  Check,
  CheckCheck,
  Clock,
  Users,
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
import { MessageMarkdown } from './MessageMarkdown'
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
  continued: boolean
  peerMarkdown: boolean
}

const bubbleWidthClass = 'max-w-[85%] md:max-w-[min(75%,660px)]'
const bodyTextClass = 'text-[16px] leading-[1.5] md:text-[15px]'

function bubbleStyle(isSelf: boolean, continued: boolean): React.CSSProperties {
  return {
    '--mh-link': isSelf ? 'var(--cp-message-self-link)' : 'var(--cp-accent)',
    background: isSelf ? 'var(--cp-message-self-bg)' : 'var(--cp-message-peer-bg)',
    color: isSelf ? 'var(--cp-message-self-text)' : 'var(--cp-text)',
    border: isSelf ? '1px solid transparent' : '1px solid var(--cp-message-peer-border)',
    borderRadius: isSelf
      ? `18px ${continued ? 6 : 18}px 6px 18px`
      : `${continued ? 6 : 18}px 18px 18px 6px`,
    padding: '8px 12px',
  } as React.CSSProperties
}

function rowClass(isSelf: boolean, continued: boolean): string {
  return `flex ${isSelf ? 'justify-end' : 'justify-start'} ${continued ? 'pt-0.5' : 'pt-3'}`
}

type MessageRenderer = (
  message: MessageObject,
  context: MessageRenderContext,
) => React.ReactNode | null

const messageRenderers: readonly MessageRenderer[] = [
  message => isActionMessage(message) ? <ActionMessage message={message} /> : null,
  message => message.ui_unavailable === true ? <UnavailableMessage message={message} /> : null,
  (message, context) => parseGroupNotice(message) ? <GroupNoticeMessage message={message} context={context} /> : null,
  (message, context) => hasAttachmentRefs(message) ? <AttachmentMessage message={message} context={context} /> : null,
  renderTextMessage,
  renderFallbackMessage,
]

export const ConversationListRow = memo(function ConversationListRow({
  item,
  isGroup,
  selfDid,
  continued = false,
  peerMarkdown = false,
}: {
  item: ConversationListItem
  isGroup: boolean
  selfDid: DID
  continued?: boolean
  peerMarkdown?: boolean
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
      {messageRenderers.map((renderer) => renderer(item.data, { isGroup, selfDid, messageIndex: item.messageIndex, continued, peerMarkdown })).find(Boolean)}
    </>
  )
})

ConversationListRow.displayName = 'ConversationListRow'

function renderTextMessage(
  message: MessageObject,
  { isGroup, selfDid, continued, peerMarkdown }: MessageRenderContext,
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
  const asMarkdown = format === 'text/markdown' || (peerMarkdown && !isSelf && format === 'text/plain')

  return (
    <div
      className={rowClass(isSelf, continued)}
      key={`${message.from}:${message.created_at_ms}`}
    >
      <div
        className={`${bubbleWidthClass} min-w-[80px]`}
        style={bubbleStyle(isSelf, continued)}
      >
        {!isSelf && isGroup && !continued ? (
          <p
            className="text-[13px] font-semibold mb-1"
            style={{ color: 'var(--cp-accent)' }}
          >
            {senderName}
          </p>
        ) : null}
        {asMarkdown ? (
          <div className={bodyTextClass}><MessageMarkdown text={message.content.content ?? ''} /></div>
        ) : (
          <p className={`${bodyTextClass} whitespace-pre-wrap break-words`}>
            {message.content.content}
          </p>
        )}
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
      <div className="mt-1 flex items-center justify-end gap-1">
        {record?.boxKind === 'REQUEST_BOX' ? <span className="mr-auto rounded-full px-1.5 text-[11px] leading-[18px]" data-testid="request-chip" style={{ background: 'color-mix(in srgb, var(--cp-warning) 16%, transparent)', color: 'color-mix(in srgb, var(--cp-warning) 70%, var(--cp-text))' }}>{t('messagehub.requestShort')}</span> : null}
        <span
          className="text-[11px] tabular-nums"
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
    <div className={rowClass(isSelf, context.continued)}>
      <div
        className={`${bubbleWidthClass} min-w-[160px]`}
        style={bubbleStyle(isSelf, context.continued)}
      >
        {!isSelf && context.isGroup && !context.continued ? <p className="text-[13px] font-semibold mb-1" style={{ color: 'var(--cp-accent)' }}>{senderName}</p> : null}
        <div className="flex flex-col gap-2">
          {refs.map((ref, index) => <MessageRef key={`${index}:${ref.target.type === 'data_obj' ? ref.target.obj_id : ref.target.did}`} item={ref} id={`${messageId}#${index}`} isSelf={isSelf} />)}
        </div>
        {caption.length > 0 ? <p className={`${bodyTextClass} mt-2 whitespace-pre-wrap break-words`}>{caption}</p> : null}
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
  { selfDid, continued }: MessageRenderContext,
) {
  const isSelf = message.from === selfDid

  return (
    <div
      className={rowClass(isSelf, continued)}
      key={`${message.from}:${message.created_at_ms}:fallback`}
    >
      <div
        className={`${bubbleWidthClass} min-w-[120px]`}
        style={{ ...bubbleStyle(false, continued), borderRadius: '18px' }}
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

const knownActions = new Set([
  'entity.member_joined', 'entity.member_left', 'entity.member_removed', 'session.title_changed', 'session.member_state_changed', 'session.shared_state_changed',
  'entity.group_created', 'entity.member_invited', 'entity.invite_revoked', 'entity.invite_expired', 'entity.member_requested', 'entity.member_rejected', 'entity.member_role_changed',
  'entity.owner_changed', 'entity.group_archived', 'entity.group_deleted', 'entity.config_changed', 'session.created', 'session.archived', 'session.deleted',
])

function ActionMessage({ message }: { message: MessageObject }) {
  const { t } = useI18n()
  const { displayName } = useContext(ConversationMessageActionsContext)
  const data = message.content.machine?.data
  let summary = message.content.content || t('messagehub.action.unknown')
  if (data?.schema_version === 1 && typeof data.action === 'string') {
    const actor = typeof data.actor_did === 'string' ? displayName?.(data.actor_did) ?? data.actor_did : t('messagehub.action.unknownActor')
    const subject = typeof data.subject_did === 'string' ? displayName?.(data.subject_did) ?? data.subject_did : t('messagehub.action.unknownMember')
    if (knownActions.has(data.action)) summary = t(`messagehub.action.${data.action}`, undefined, { actor, subject })
  } else if (data?.schema_version !== 1) summary = `${t('messagehub.action.unsupported')} · ${summary}`
  return <div data-testid="action-message" className="mx-auto my-2 max-w-xl rounded-lg bg-[color:color-mix(in_srgb,var(--cp-text)_5%,transparent)] px-3 py-2 text-center text-xs text-[color:var(--cp-muted)] break-words">{summary}</div>
}

function GroupNoticeMessage({ message, context }: { message: MessageObject; context: MessageRenderContext }) {
  const { t } = useI18n()
  const actions = useContext(ConversationMessageActionsContext)
  const [state, setState] = useState<'idle' | 'pending'>('idle')
  const [error, setError] = useState('')
  const notice = parseGroupNotice(message)
  const view = notice ? actions.groupNotice?.(message) : null
  if (!notice || !view) return renderFallbackMessage(message, context)
  const inviter = actions.displayName?.(message.from) ?? message.from
  const invitation = notice.invitation
  const join = () => {
    if (!actions.joinGroup || state === 'pending') return
    setState('pending'); setError('')
    void actions.joinGroup(message).then(() => setState('idle'), failure => { setState('idle'); setError(groupErrorText(t, failure)) })
  }
  const line = invitation ? t('messagehub.group.invitedBy', undefined, { name: inviter })
    : notice.action === 'pending_approval' ? t('messagehub.group.notice.pending_approval', undefined, { name: notice.memberDid ? actions.displayName?.(notice.memberDid) ?? notice.memberDid : inviter })
    : notice.action === 'rejected' || notice.action === 'removed' || notice.action === 'session_invite' ? t(`messagehub.group.notice.${notice.action}`, undefined, { name: inviter })
    : notice.action
  return (
    <div className={rowClass(false, context.continued)}>
      <div className={`${bubbleWidthClass} w-72 min-w-[220px]`} style={{ ...bubbleStyle(false, context.continued), padding: 12 }} data-testid="group-notice" data-action={notice.action} data-state={view.state}>
        <div className="flex items-center gap-2.5">
          <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full" style={{ background: 'color-mix(in srgb, var(--cp-warning) 18%, transparent)', color: 'var(--cp-warning)' }} aria-hidden><Users size={17} /></span>
          <div className="min-w-0 flex-1">
            <p className="truncate text-xs" style={{ color: 'var(--cp-muted)' }}>{line}</p>
            <p className="truncate text-[15px] font-semibold" title={notice.groupDid}>{view.groupName}</p>
          </div>
        </div>
        {invitation && invitation.role !== 'member' ? <p className="mt-2 text-xs" style={{ color: 'var(--cp-muted)' }}>{t('messagehub.group.invitedAs', undefined, { role: t(`messagehub.group.role.${invitation.role}`) })}</p> : null}
        {invitation ? (
          <div className="mt-3 flex flex-wrap items-center gap-2">
            {view.state === 'pending' && actions.joinGroup ? <button type="button" className="min-h-11 rounded-lg px-3 text-sm font-semibold text-white disabled:opacity-50 md:min-h-9" style={{ background: 'var(--cp-accent)' }} disabled={state === 'pending'} onClick={join}>{t(state === 'pending' ? 'messagehub.group.joining' : 'messagehub.group.join')}</button> : null}
            {view.state === 'joined' ? <>
              <span className="text-xs font-medium" style={{ color: 'var(--cp-success)' }}>{t('messagehub.group.joined')}</span>
              {actions.openEntity ? <button type="button" className="min-h-11 rounded-lg border px-3 text-sm md:min-h-9" style={{ borderColor: 'var(--cp-border)' }} onClick={() => actions.openEntity?.(notice.groupDid)}>{t('messagehub.group.open')}</button> : null}
            </> : null}
            {view.state === 'expired' ? <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('messagehub.group.invitationExpired')}</span> : null}
            {view.state === 'pending' && invitation.expiresAt ? <span className="text-[11px]" style={{ color: 'var(--cp-muted)' }}>{t('messagehub.group.expiresAt', undefined, { time: new Date(invitation.expiresAt).toLocaleDateString() })}</span> : null}
          </div>
        ) : null}
        {error ? <p role="alert" className="mt-2 text-xs" style={{ color: 'var(--cp-danger)' }}>{error}</p> : null}
        <MessageFooter message={message} isSelf={false} />
      </div>
    </div>
  )
}
