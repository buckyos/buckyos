import { isHiddenRelationMessage } from '../history/relations'
import type { MessageObject } from './msgobj'

export const isActionMessage = (message: MessageObject) => message.kind === 'event' && message.content.machine?.intent === 'buckyos.action_log'
export function isMessageActivity(message: MessageObject) {
  if (isActionMessage(message) || message.ui_item_kind === 'status' || isHiddenRelationMessage(message)) return false
  return message.kind === 'chat' || message.kind === 'group_msg' || (message.kind === 'notify' && message.ui_unavailable !== true) || (message.kind === 'deliver' && Boolean(message.content.content?.trim() || message.content.refs?.some(ref => ref.role === 'output')))
}
export function relativeActivity(time: number | undefined, now: number, justNow: string) {
  if (!time || !Number.isFinite(time)) return '—'
  const minutes = Math.floor(Math.max(0, now - time) / 60_000)
  return minutes < 1 ? justNow : minutes < 60 ? `${minutes}m` : minutes < 1440 ? `${Math.floor(minutes / 60)}h` : `${Math.floor(minutes / 1440)}d`
}
