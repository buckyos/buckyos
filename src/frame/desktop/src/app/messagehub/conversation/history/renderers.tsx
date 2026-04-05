import { memo } from 'react'
import {
  AlertCircle,
  Check,
  CheckCheck,
  Clock,
} from 'lucide-react'
import {
  getMessageDeliveryStatus,
  getMessageSenderName,
  type RefItem,
  type DID,
  type MessageDeliveryStatus,
  type MessageObject,
} from '../../protocol/msgobj'
import type { ConversationListItem } from './types'

interface MessageRenderContext {
  isGroup: boolean
  selfDid: DID
}

type MessageRenderer = (
  message: MessageObject,
  context: MessageRenderContext,
) => React.ReactNode | null

const messageRenderers: readonly MessageRenderer[] = [
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
  const caption = message.content.content.trim()

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
