import { z } from 'zod'
import { notifyCategoryFromSessionId } from 'buckyos'
import { NOTIFY_CATEGORIES } from '../../components/conversation/model/msgobj'
import { defaultSessionPrefs, sessionTitle } from '../../shared/messagehub/session.ts'
import type { ConversationMemoryMode } from '../../components/conversation/backendTypes'
import type { Entity, MessageHubContext, Session, SessionAccess, SessionBinding, SessionPreferences } from './types'

export const MEMORY_MODES = ['normal', 'no_write', 'no_read_write'] as const satisfies readonly ConversationMemoryMode[]
/** `setDefault` and `memoryMode` are an Agent's alone. */
export const createSessionSchema = z.object({ entityId: z.string().min(1), title: z.string().trim().max(64), description: z.string().trim().max(500).optional(), connection: z.string().min(1), setDefault: z.boolean().optional(), memoryMode: z.enum(MEMORY_MODES).optional() })
/** The new-session dialog: the entity is the one whose session list it was opened from. */
export const newSessionFormSchema = z.object({ title: z.string().trim().min(1).max(64), description: z.string().trim().max(500), connection: z.string().min(1), setDefault: z.boolean(), memoryMode: z.enum(MEMORY_MODES) })
/** Display name of the native connection (MsgObject over the Named Message Protocol). */
export const NATIVE_CONNECTION_LABEL = 'Named Message Protocol'
export const sharedStateSchema = z.object({ title: z.string().trim().max(64), description: z.string().trim().max(500) })
/** Group Sessions also carry an announcement (`group.update_shared_state`, at most 1024 chars). */
export const groupSharedStateSchema = sharedStateSchema.extend({ announcement: z.string().trim().max(1024).optional() })
export const memberStateSchema = z.object({ nickname: z.string().trim().max(64) })
export const presentationSchema = z.object({ title: z.string().trim().max(64), description: z.string().trim().max(500), pinned: z.boolean(), muted: z.boolean() })
export const pinnedMessageSchema = z.object({ id: z.string().min(1), text: z.string().max(2000), senderDid: z.string(), createdAt: z.number() })
export const defaultPreferences: SessionPreferences = { ...defaultSessionPrefs, showActions: true }
export const sessionKey = (ownerDid: string, sessionId: string) => JSON.stringify([ownerDid, sessionId])
export const viewerSessionKey = (context: MessageHubContext, sessionId: string) => JSON.stringify([context.viewerDid, context.ownerDid, sessionId])
/** Rank of a notification session in the fixed category order (to-do, Agent, App); -1 for any other session. */
function notifyRank(session: Session) {
  const category = notifyCategoryFromSessionId(session.id)
  return category ? NOTIFY_CATEGORIES.indexOf(category) : -1
}
/** Pinned first, then by activity; the notification sessions among them keep the category order (to-do, Agent, App). */
export function sortSessions(sessions: Session[], preferences: (id: string) => SessionPreferences) {
  const sorted = [...sessions].sort((a, b) => Number(preferences(b.id).pinned) - Number(preferences(a.id).pinned) || b.lastActiveAt - a.lastActiveAt || a.id.localeCompare(b.id))
  const slots = sorted.flatMap((session, index) => notifyRank(session) >= 0 ? [index] : [])
  const notify = slots.map(index => sorted[index]).sort((a, b) => notifyRank(a) - notifyRank(b))
  slots.forEach((slot, index) => { sorted[slot] = notify[index] })
  return sorted
}
/** A conversation with an Agent on a channel of another platform, read from the Agent's mailbox (`MessageHubStore.channelSessions`). */
export const isChannelSession = (context: MessageHubContext, session: Session) => context.mode === 'self' && session.ownerDid !== context.ownerDid
/** The context a session listed in `context` is read in. */
export const readingContext = (context: MessageHubContext, session: Session): MessageHubContext => isChannelSession(context, session) ? { viewerDid: context.viewerDid, ownerDid: session.ownerDid, mode: 'observe' } : context
export const channelAccess: SessionAccess = { mode: 'read_only', canManage: false, canEnableWrite: false, canEditPresentation: false, canEditSharedState: false, canEditOwnMemberState: false, readOnlyReason: 'channel' }
/** Where the main entry of an entity leads; the owner's own Agents have a stored default (`MessageHubStore.defaultSession`). */
export function selectDefaultSession(entity: Entity, sessions: Session[]): Session | null {
  const active = sessions.filter(session => session.entityId === entity.id && session.lifecycle === 'active')
  // The system notifications open on their most important category.
  const notify = entity.systemNotify ? active.filter(session => notifyRank(session) >= 0).sort((a, b) => notifyRank(a) - notifyRank(b))[0] : undefined
  if (notify) return notify
  const candidates = entity.type === 'agent' ? active.filter(session => session.binding.kind === 'native') : active
  const main = entity.type === 'group' ? candidates.find(session => session.id === entity.id) : undefined
  if (main) return main
  return [...candidates].sort((a, b) => b.lastActiveAt - a.lastActiveAt || a.id.localeCompare(b.id))[0] ?? null
}
export { sessionTitle }
export function sessionDescription(session: Session, preferences: SessionPreferences) {
  return preferences.description || session.shared.description
}
/** The session of an entity that saw the latest activity, among its active ones. */
export function latestActiveSession(sessions: Session[]): Session | null {
  return sessions.filter(session => session.lifecycle === 'active').sort((a, b) => b.lastActiveAt - a.lastActiveAt || a.id.localeCompare(b.id))[0] ?? null
}
export function sessionAccess(context: MessageHubContext, session: Session, confirmed: boolean): SessionAccess {
  const own = context.mode === 'self' && context.viewerDid === context.ownerDid && session.ownerDid === context.ownerDid
  const native = session.binding.kind === 'native' && !notifyCategoryFromSessionId(session.id)
  const tunnel = session.binding.kind === 'tunnel' ? session.binding : null
  const reason = !own ? 'agent_observer' : notifyCategoryFromSessionId(session.id) ? 'system_notify' : session.binding.kind === 'unknown' ? 'binding_unknown' : tunnel && !tunnel.connected ? 'transport_unavailable' : tunnel && !tunnel.canSend ? 'platform_read_only' : tunnel && !confirmed ? 'tunnel_default' : undefined
  return { mode: reason ? 'read_only' : 'read_write', canManage: own, canEnableWrite: own && !!tunnel?.connected && tunnel.canSend && !confirmed, canEditPresentation: own, canEditSharedState: own && native, canEditOwnMemberState: own && native, readOnlyReason: reason }
}
export function creationReason(context: MessageHubContext, entity: Entity, policy: string, binding?: SessionBinding) {
  if (context.mode !== 'self' || context.viewerDid !== context.ownerDid) return 'agent_observer'
  if (entity.systemNotify) return 'system_notify'
  if (policy === 'deny' || (policy !== 'allow' && entity.type !== 'agent' && binding?.kind !== 'native')) return 'creation_disabled'
  if (!binding || binding.kind === 'unknown') return 'binding_unknown'
  if (binding.kind === 'tunnel') {
    if (!binding.connected) return 'transport_unavailable'
    if (!binding.supportsMultipleSessions || !binding.canCreateRemoteSession) return 'platform_read_only'
  }
  return undefined
}
