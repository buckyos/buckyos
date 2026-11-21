import { isHiddenRelationMessage } from '../history/relations'
import type { MessageObject } from './msgobj'

export const isActionMessage = (message: MessageObject) => message.kind === 'event' && message.content.machine?.intent === 'buckyos.action_log'
export function isMessageActivity(message: MessageObject) {
  if (isActionMessage(message) || message.ui_item_kind === 'status' || isHiddenRelationMessage(message)) return false
  return message.kind === 'chat' || message.kind === 'group_msg' || (message.kind === 'notify' && message.ui_unavailable !== true) || (message.kind === 'deliver' && Boolean(message.content.content?.trim() || message.content.refs?.some(ref => ref.role === 'output')))
}
export function relativeActivity(time: number | undefined, now: number, justNow: string, locale = 'en') {
  if (!time || !Number.isFinite(time)) return '—'
  const minutes = Math.floor(Math.max(0, now - time) / 60_000)
  if (minutes < 1) return justNow
  const unit = minutes < 60 ? 'minute' : minutes < 1440 ? 'hour' : 'day'
  const value = unit === 'minute' ? minutes : Math.floor(minutes / (unit === 'hour' ? 60 : 1440))
  return new Intl.NumberFormat(locale, { style: 'unit', unit, unitDisplay: 'narrow' }).format(value)
}
