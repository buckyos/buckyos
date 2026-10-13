import { localObjectIdFor } from '../conversation/history/objectAccess'
import type { MessageObject, MsgObject, RefItem } from '../protocol/msgobj'
import type { OutgoingPayload, OutgoingUpload } from './types'

export const pendingBubbleId = (localId: string) => `pending:${localId}`

const STAGED_RELATIONS = new Set(['edit', 'redact', 'reaction'])

export function canStage(payload: OutgoingPayload): payload is OutgoingPayload & { localId: string } {
  return !!payload.localId && !(payload.relatesTo && STAGED_RELATIONS.has(payload.relatesTo.rel))
}

export function localAttachmentRefs(payload: OutgoingPayload): RefItem[] {
  return payload.attachments.map(item => {
    const label = item.relativePath || item.file.name
    return { role: 'input', target: { type: 'data_obj', obj_id: localObjectIdFor(item.file, label) }, label }
  })
}

export function pendingBubble(
  payload: OutgoingPayload & { localId: string },
  base: Pick<MsgObject, 'from' | 'to' | 'kind' | 'to_session'> & { sessionId: string; senderName: string; now: number },
  upload: OutgoingUpload | undefined,
  staged: boolean,
): MessageObject {
  const refs = localAttachmentRefs(payload)
  return {
    from: base.from,
    to: base.to,
    kind: base.kind,
    ...(base.to_session !== undefined ? { to_session: base.to_session } : {}),
    ...(payload.relatesTo ? { relates_to: payload.relatesTo } : {}),
    ...(payload.mentions && (payload.mentions.all || payload.mentions.dids?.length) ? { mentions: payload.mentions } : {}),
    created_at_ms: payload.submittedAt ?? base.now,
    content: { format: 'text/plain', content: payload.content.trim(), ...(payload.machine ? { machine: payload.machine } : {}), ...(refs.length ? { refs } : {}) },
    ui_message_id: pendingBubbleId(payload.localId),
    ui_local_id: payload.localId,
    ui_session_id: base.sessionId,
    ui_sender_name: base.senderName,
    ui_delivery_status: 'sending',
    ...(staged ? { ui_staged: true } : {}),
    ...(upload && refs.length ? { ui_upload: upload } : {}),
  }
}

export function outgoingUpload(message: MessageObject): OutgoingUpload | undefined {
  const upload = message.ui_upload as OutgoingUpload | undefined
  return upload && typeof upload === 'object' && typeof upload.phase === 'string' ? upload : undefined
}
