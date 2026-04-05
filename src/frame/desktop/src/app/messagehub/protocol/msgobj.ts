/**
 * Thin TypeScript mirror of:
 * /Users/liuzhicong/project/cyfs-ndn/src/ndn-lib/src/msgobj.rs
 *
 * Keep the field names aligned with Rust serde output so the UI can consume
 * protocol objects directly instead of mapping them into a separate UI DTO.
 */

export type DID = string
export type ObjId = string
export type Uri = string

export type JsonValue =
  | null
  | boolean
  | number
  | string
  | JsonValue[]
  | { [key: string]: JsonValue }

export type MsgObjKind =
  | 'chat'
  | 'group_msg'
  | 'deliver'
  | 'notify'
  | 'event'
  | 'operation'

export type MsgContentFormat =
  | 'text/plain'
  | 'text/markdown'
  | 'text/html'
  | 'text/css'
  | 'text/xml'
  | 'image/png'
  | 'image/jpeg'
  | 'image/gif'
  | 'image/webp'
  | 'image/svg+xml'
  | 'image/bmp'
  | 'video/mp4'
  | 'video/webm'
  | 'video/ogg'
  | 'video/quicktime'
  | 'video/x-msvideo'
  | 'audio/mpeg'
  | 'audio/wav'
  | 'audio/ogg'
  | 'audio/webm'
  | 'audio/aac'
  | 'audio/flac'
  | 'application/json'
  | 'application/xml'
  | 'application/pdf'
  | 'application/zip'
  | 'application/octet-stream'
  | string

export interface TopicThread {
  topic?: string
  reply_to?: ObjId
  correlation_id?: string
  tunnel_id?: string
}

export type CanonValue =
  | null
  | boolean
  | number
  | string
  | number[]
  | CanonValue[]
  | { [key: string]: CanonValue }

export interface MachineContent {
  intent?: string
  data?: Record<string, CanonValue>
}

export type RefTarget =
  | {
      type: 'data_obj'
      obj_id: ObjId
      uri_hint?: Uri
    }
  | {
      type: 'service_did'
      did: DID
    }

export type RefRole =
  | 'context'
  | 'input'
  | 'output'
  | 'evidence'
  | 'control'

export interface RefItem {
  role: RefRole
  target: RefTarget
  label?: string
}

export interface MsgContent {
  title?: string
  format?: MsgContentFormat
  content: string
  machine?: MachineContent
  refs?: RefItem[]
}

/**
 * Rust flattens `meta` into the top-level object, so unknown keys are allowed
 * here on purpose. UI-specific hints should also live there.
 */
export interface MsgObject {
  from: DID
  to: DID[]
  kind: MsgObjKind
  thread?: TopicThread
  workspace?: DID
  created_at_ms: number
  expires_at_ms?: number
  nonce?: number
  content: MsgContent
  proof?: string
  [key: string]: unknown
}

export type MessageObject = MsgObject
export type MessageDeliveryStatus =
  | 'sending'
  | 'sent'
  | 'delivered'
  | 'read'
  | 'failed'

export type ConversationStatusType =
  | 'typing'
  | 'processing'
  | 'disconnected'
  | 'info'

export function getMessageMetaString(
  message: MessageObject,
  key: string,
): string | undefined {
  const value = message[key]
  return typeof value === 'string' ? value : undefined
}

export function getMessageSenderName(message: MessageObject): string {
  return getMessageMetaString(message, 'ui_sender_name') ?? message.from
}

export function getMessageDeliveryStatus(
  message: MessageObject,
): MessageDeliveryStatus | undefined {
  const status = getMessageMetaString(message, 'ui_delivery_status')
  if (
    status === 'sending'
    || status === 'sent'
    || status === 'delivered'
    || status === 'read'
    || status === 'failed'
  ) {
    return status
  }

  return undefined
}

export function getMessageStatusType(
  message: MessageObject,
): ConversationStatusType | undefined {
  const status = getMessageMetaString(message, 'ui_status_type')
  if (
    status === 'typing'
    || status === 'processing'
    || status === 'disconnected'
    || status === 'info'
  ) {
    return status
  }

  return undefined
}

export function getMessageStableId(
  message: MessageObject,
  indexHint: number,
): string {
  return (
    getMessageMetaString(message, 'ui_message_id')
    ?? `${message.from}:${message.created_at_ms}:${indexHint}`
  )
}

export function isStatusMessageObject(message: MessageObject): boolean {
  return (
    message.kind === 'notify'
    || getMessageMetaString(message, 'ui_item_kind') === 'status'
  )
}
