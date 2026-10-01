import { z } from 'zod'
import type { MessageObject } from './protocol/msgobj'
import type { MessageHubStore } from './store/types'
import type { Entity, GroupInvitation, GroupMember, GroupRole, MessageHubContext } from './types'

export const GROUP_INVITATION_INTENT = 'buckyos.group_invitation'
export const GROUP_MEMBER_LIMIT = 100

export const createGroupSchema = z.object({
  name: z.string().trim().max(64),
  members: z.array(z.string().min(1)).max(GROUP_MEMBER_LIMIT),
})

/** Members that still occupy a slot: invited, waiting for approval or active. */
export const participating = (member: GroupMember) => member.state === 'active' || member.state === 'invited' || member.state === 'pending_admin_approval'

const roleRank: Record<GroupRole, number> = { owner: 0, admin: 1, member: 2 }
const stateRank: Record<string, number> = { active: 0, pending_admin_approval: 1, invited: 2 }

export function sortGroupMembers(members: GroupMember[]): GroupMember[] {
  return [...members].sort((a, b) => (stateRank[a.state] ?? 3) - (stateRank[b.state] ?? 3) || roleRank[a.role] - roleRank[b.role] || a.did.localeCompare(b.did))
}

/** Name used when the creator leaves the group name empty. */
export function defaultGroupName(memberNames: string[]): string {
  const names = memberNames.filter(Boolean)
  if (names.length === 0) return ''
  const head = names.slice(0, 3).join(', ')
  return names.length > 3 ? `${head} +${names.length - 3}` : head
}

/**
 * People and agents reachable over a native BuckyOS connection: a group only
 * accepts single-entity DIDs that can sign their own membership proof.
 */
export function groupMemberCandidates(entities: Entity[], ownerDid: string, hasNative: (entity: Entity) => boolean, blocked: (entity: Entity) => boolean = () => false): Entity[] {
  const seen = new Set<string>()
  const result: Entity[] = []
  const visit = (entity: Entity) => {
    if (seen.has(entity.id)) return
    seen.add(entity.id)
    if ((entity.type === 'person' || entity.type === 'agent') && entity.id !== ownerDid && entity.id.startsWith('did:') && !entity.id.startsWith('did:msgtunnel:') && hasNative(entity) && !blocked(entity)) result.push(entity)
    entity.children?.forEach(visit)
  }
  entities.forEach(visit)
  return result.sort((a, b) => a.name.localeCompare(b.name))
}

/** Group member candidates of the owner's contact list, as offered by the member picker. */
export function memberCandidates(store: MessageHubStore, context: MessageHubContext): Entity[] {
  return groupMemberCandidates(
    store.entities(context),
    context.ownerDid,
    entity => store.connections(context, entity.id).some(choice => choice.binding.kind === 'native'),
    entity => store.admission(context, entity.id)?.accessLevel === 'block',
  )
}

/** A personal group notification (`invite`, `pending_approval`, `rejected`, `removed`, `session_invite`). */
export interface GroupNotice {
  groupDid: string
  action: string
  memberDid?: string
  invitation?: GroupInvitation
}

export function parseGroupNotice(message: MessageObject): GroupNotice | null {
  const machine = message.content.machine
  if (machine?.intent !== GROUP_INVITATION_INTENT) return null
  const data = machine.data ?? {}
  const detail = data.data && typeof data.data === 'object' && !Array.isArray(data.data) ? data.data as Record<string, unknown> : {}
  if (typeof data.group_did !== 'string' || typeof data.action !== 'string') return null
  const notice: GroupNotice = { groupDid: data.group_did, action: data.action, ...(typeof detail.member_did === 'string' ? { memberDid: detail.member_did } : {}) }
  if (data.action === 'invite' && typeof detail.invite_id === 'string') {
    const role = detail.role === 'admin' || detail.role === 'owner' ? detail.role : 'member'
    notice.invitation = { groupDid: data.group_did, inviteId: detail.invite_id, role, expiresAt: typeof detail.expires_at_ms === 'number' ? detail.expires_at_ms : undefined, inviterDid: message.from }
  }
  return notice
}

export function parseGroupInvitation(message: MessageObject): GroupInvitation | null {
  return parseGroupNotice(message)?.invitation ?? null
}

const knownGroupErrors = new Set(['owner-proof-required', 'member-proof-required', 'signed-member-proof-required', 'capability-denied', 'member-already-participating', 'member-must-be-single-entity', 'member-limit', 'owner-must-transfer-first', 'owner-required', 'role-not-allowed', 'invite-expired', 'invite-already-expired', 'proof-invitation-mismatch', 'not-found', 'blocked', 'rate-limited', 'invalid-group-name', 'session-limit'])

/** User-facing text for a group operation failure; unknown host reasons are shown verbatim. */
export function groupErrorText(t: (key: string, fallback?: string, variables?: Record<string, string | number>) => string, error: unknown): string {
  const message = error instanceof Error ? error.message : String(error)
  const reason = message.replace(/^rejected:\s*/, '').match(/[a-z0-9]+(?:-[a-z0-9]+)+/)?.[0] ?? (message === 'agent_observer' ? 'agent_observer' : 'unknown')
  if (reason === 'agent_observer') return t('messagehub.reason.agent_observer')
  return knownGroupErrors.has(reason) ? t(`messagehub.groupError.${reason}`) : t('messagehub.groupError.generic', undefined, { reason })
}

/** Canonical MailboxAddress of a group session (`<group_did>[/<encoded session id>]`). */
export function groupSessionKey(groupDid: string, sessionId?: string): string {
  if (sessionId === undefined) return groupDid
  const encoded = [...new TextEncoder().encode(sessionId)].map(byte => /[A-Za-z0-9\-_.~:@]/.test(String.fromCharCode(byte)) && byte < 0x80 ? String.fromCharCode(byte) : `%${byte.toString(16).toUpperCase().padStart(2, '0')}`).join('')
  return `${groupDid}/${encoded}`
}

/** Raw `to_session` of a local group session key; undefined for the default session. */
export function groupSessionId(groupDid: string, sessionKey: string): string | undefined {
  if (sessionKey === groupDid || !sessionKey.startsWith(`${groupDid}/`)) return undefined
  return decodeURIComponent(sessionKey.slice(groupDid.length + 1))
}
