import { z } from 'zod'
import type { MessageObject } from './protocol/msgobj'
import type { Entity, MessageHubContext, Session, SessionAccess, SessionBinding, SessionPreferences } from './types'

export const createSessionSchema = z.object({ entityId: z.string().min(1), title: z.string().trim().max(64), connection: z.string().min(1) })
export const sharedStateSchema = z.object({ title: z.string().trim().max(64), description: z.string().trim().max(500) })
export const memberStateSchema = z.object({ nickname: z.string().trim().max(64) })
export const presentationSchema = z.object({ title: z.string().trim().max(64), pinned: z.boolean(), muted: z.boolean() })
export const defaultPreferences: SessionPreferences = { title: '', pinned: false, muted: false, showActions: true }
export const sessionKey = (ownerDid: string, sessionId: string) => JSON.stringify([ownerDid, sessionId])
export const viewerSessionKey = (context: MessageHubContext, sessionId: string) => JSON.stringify([context.viewerDid, context.ownerDid, sessionId])
export const isActionMessage = (message: MessageObject) => message.kind === 'event' && message.content.machine?.intent === 'buckyos.action_log'
export function isMessageActivity(message: MessageObject) {
  if (isActionMessage(message) || message.ui_item_kind === 'status') return false
  return message.kind === 'chat' || message.kind === 'group_msg' || (message.kind === 'deliver' && Boolean(message.content.content.trim() || message.content.refs?.some(ref => ref.role === 'output')))
}
export function relativeActivity(time: number | undefined, now: number, justNow: string) {
  if (!time || !Number.isFinite(time)) return '—'
  const minutes = Math.floor(Math.max(0, now - time) / 60_000)
  return minutes < 1 ? justNow : minutes < 60 ? `${minutes}m` : minutes < 1440 ? `${Math.floor(minutes / 60)}h` : `${Math.floor(minutes / 1440)}d`
}
export function sortSessions(sessions: Session[], preferences: (id: string) => SessionPreferences) {
  return [...sessions].sort((a, b) => Number(preferences(b.id).pinned) - Number(preferences(a.id).pinned) || b.lastActiveAt - a.lastActiveAt || a.id.localeCompare(b.id))
}
export function sessionTitle(session: Session, preferences: SessionPreferences) {
  return preferences.title || session.shared.title || session.title
}
export function sessionAccess(context: MessageHubContext, session: Session, confirmed: boolean): SessionAccess {
  const own = context.mode === 'self' && context.viewerDid === context.ownerDid && session.ownerDid === context.ownerDid
  const native = session.binding.kind === 'native'
  const tunnel = session.binding.kind === 'tunnel' ? session.binding : null
  const reason = !own ? 'agent_observer' : session.binding.kind === 'unknown' ? 'binding_unknown' : tunnel && !tunnel.connected ? 'transport_unavailable' : tunnel && !tunnel.canSend ? 'platform_read_only' : tunnel && !confirmed ? 'tunnel_default' : undefined
  return { mode: reason ? 'read_only' : 'read_write', canManage: own, canEnableWrite: own && !!tunnel?.connected && tunnel.canSend && !confirmed, canEditPresentation: own, canEditSharedState: own && native, canEditOwnMemberState: own && native, readOnlyReason: reason }
}
export function creationReason(context: MessageHubContext, entity: Entity, policy: string, binding?: SessionBinding) {
  if (context.mode !== 'self' || context.viewerDid !== context.ownerDid) return 'agent_observer'
  if (policy === 'deny' || (policy !== 'allow' && entity.type !== 'agent')) return 'creation_disabled'
  if (!binding || binding.kind === 'unknown') return 'binding_unknown'
  if (binding.kind === 'tunnel') {
    if (!binding.connected) return 'transport_unavailable'
    if (!binding.supportsMultipleSessions || !binding.canCreateRemoteSession) return 'platform_read_only'
  }
  return undefined
}
