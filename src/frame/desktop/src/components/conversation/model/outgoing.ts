import type { ndn } from 'buckyos'
import type { ComposerAttachmentInput } from '../input/attachmentDraft'
import type { ConversationComposerSubmitPayload } from '../input/ConversationComposer'
import type { MessageObject, VoiceNote } from './msgobj'

/**
 * What the composer hands to the store: text plus the raw browser files, and
 * for a relation message (edit / redact / reaction / reply) the MsgObject v2
 * `relates_to` and structured `mentions`, which travel in the message itself.
 * `machine` becomes `content.machine` (an answer that picked an option).
 * `voice` marks the only attachment as a voice note (`MsgObject.voice`).
 */
export interface OutgoingPayload {
  signal?: AbortSignal
  content: string
  attachments: ComposerAttachmentInput[]
  relatesTo?: ndn.MsgRelation
  mentions?: ndn.MsgMentions
  machine?: ndn.MachineContent
  voice?: VoiceNote
  localId?: string
  submittedAt?: number
}

/** `ui_upload` of a pending outgoing bubble: waiting behind an earlier send, preparing the files, or uploading them. */
export type OutgoingUpload =
  | { phase: 'queued' }
  | { phase: 'preparing' }
  | { phase: 'uploading'; loaded: number; total: number }

export function outgoingUpload(message: MessageObject): OutgoingUpload | undefined {
  const upload = message.ui_upload as OutgoingUpload | undefined
  return upload && typeof upload === 'object' && typeof upload.phase === 'string' ? upload : undefined
}

/** What the composer submitted, as a conversation source sends it. */
export const toOutgoing = (payload: ConversationComposerSubmitPayload): OutgoingPayload => ({
  content: payload.content,
  signal: payload.signal,
  attachments: payload.attachments.map(({ file, relativePath, directory }) => ({ file, relativePath, directory })),
  ...(payload.relatesTo ? { relatesTo: payload.relatesTo } : {}),
  ...(payload.mentions ? { mentions: payload.mentions } : {}),
  ...(payload.machine ? { machine: payload.machine } : {}),
  ...(payload.voice ? { voice: payload.voice } : {}),
  ...(payload.localId ? { localId: payload.localId, submittedAt: payload.submittedAt } : {}),
})
