import { FRIEND_INTENT } from 'buckyos'
import type { MessageObject } from './protocol/msgobj'

/** A `buckyos.friend` request message (`machine.data`); accept / reject / cancel / remove leave no mailbox record. */
export interface FriendNotice {
  action: string
  requestId: string
  message?: string
  profileHint?: { name?: string; avatar?: string }
}

const text = (value: unknown) => (typeof value === 'string' && value.trim() ? value.trim() : undefined)

export function parseFriendNotice(message: MessageObject): FriendNotice | null {
  const machine = message.content.machine
  if (machine?.intent !== FRIEND_INTENT) return null
  const data = machine.data ?? {}
  const requestId = text(data.request_id)
  if (typeof data.action !== 'string' || !requestId) return null
  const hint = data.profile_hint && typeof data.profile_hint === 'object' ? data.profile_hint as Record<string, unknown> : null
  return {
    action: data.action,
    requestId,
    ...(text(data.message) ? { message: text(data.message) } : {}),
    ...(hint ? { profileHint: { name: text(hint.name), avatar: text(hint.avatar) } } : {}),
  }
}
