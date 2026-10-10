import { z } from 'zod'
import type { MessageObject } from './protocol/msgobj'
import type { MessageHubStore } from './store/types'
import type { Entity, GroupInvitation, GroupMember, GroupRole, GroupSessionParticipant, MessageHubContext } from './types'

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

/** Session participants in display order: owner, admins, members, guests, then pending guest invitations. */
export function sortSessionParticipants(items: GroupSessionParticipant[]): GroupSessionParticipant[] {
  const rank = (item: GroupSessionParticipant) => item.state === 'invited' ? 4 : item.role ? roleRank[item.role] : 3
  return [...items].sort((a, b) => rank(a) - rank(b) || a.did.localeCompare(b.did))
}

/** Name used when the creator leaves the group name empty. */
export function defaultGroupName(memberNames: string[]): string {
  const names = memberNames.filter(Boolean)
  if (names.length === 0) return ''
  const head = names.slice(0, 3).join(', ')
  return names.length > 3 ? `${head} +${names.length - 3}` : head
}

/**
 * People and agents reachable over a native BuckyOS connection. A group only
 * accepts single-entity DIDs (user, agent, device): nested groups and DID
 * Collections are deferred (v2 §13), so another group is never a candidate.
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

/** A personal group notification (`invite`, `pending_approval`, `rejected`, `removed`, `owner_transfer`, `session_invite`, `session_removed`). */
export interface GroupNotice {
  groupDid: string
  action: string
  memberDid?: string
  invitedBy?: string
  invitation?: GroupInvitation
  /** `owner_transfer` only. */
  transferId?: string
  expiresAt?: number
  /** `session_invite` / `session_removed` only. */
  sessionId?: string
  sessionTitle?: string
}

const text = (value: unknown) => typeof value === 'string' ? value : undefined

export function parseGroupNotice(message: MessageObject): GroupNotice | null {
  const machine = message.content.machine
  if (machine?.intent !== GROUP_INVITATION_INTENT) return null
  const data = machine.data ?? {}
  const detail = data.data && typeof data.data === 'object' && !Array.isArray(data.data) ? data.data as Record<string, unknown> : {}
  if (typeof data.group_did !== 'string' || typeof data.action !== 'string') return null
  const notice: GroupNotice = { groupDid: data.group_did, action: data.action }
  const memberDid = text(detail.member_did), invitedBy = text(detail.invited_by), transferId = text(detail.transfer_id), sessionId = text(detail.session_id), sessionTitle = text(detail.title)
  if (memberDid) notice.memberDid = memberDid
  if (invitedBy) notice.invitedBy = invitedBy
  if (transferId) notice.transferId = transferId
  if (sessionId) notice.sessionId = sessionId
  if (sessionTitle) notice.sessionTitle = sessionTitle
  if (typeof detail.expires_at_ms === 'number') notice.expiresAt = detail.expires_at_ms
  if (data.action === 'invite' && typeof detail.invite_id === 'string') {
    const role = detail.role === 'admin' || detail.role === 'owner' ? detail.role : 'member'
    const state = detail.state === 'active' || detail.state === 'pending_admin_approval' || detail.state === 'invited' ? detail.state : undefined
    notice.invitation = { groupDid: data.group_did, inviteId: detail.invite_id, role, expiresAt: notice.expiresAt, inviterDid: message.from, ...(state ? { state } : {}), ...(memberDid ? { memberDid } : {}) }
  }
  return notice
}

export function parseGroupInvitation(message: MessageObject): GroupInvitation | null {
  return parseGroupNotice(message)?.invitation ?? null
}

const knownGroupErrors = new Set(['agent-group-disabled', 'capability-denied', 'member-already-participating', 'member-must-be-single-entity', 'member-limit', 'owner-must-transfer-first', 'owner-required', 'controller-required', 'role-not-allowed', 'invite-expired', 'invite-already-expired', 'invitation-mismatch', 'transfer-mismatch', 'owner-transfer-pending', 'agent-owner-required', 'invite-required', 'join-not-allowed', 'member-not-pending', 'use-transfer-owner', 'cannot-moderate-owner', 'revision-conflict', 'invalid-invite-link', 'guests-not-allowed', 'guest-limit', 'not-a-session-guest', 'session-archived', 'post-not-allowed', 'not-found', 'blocked', 'rate-limited', 'invalid-group-name', 'session-limit'])

/** The Agent has not been allowed to join group chats (its Users and Agents setting). */
export function isAgentGroupDisabled(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error)
  return message.includes('agent-group-disabled') || message.includes('agent_group_disabled')
}

/** User-facing text for a group operation failure; unknown host reasons are shown verbatim. */
export function groupErrorText(t: (key: string, fallback?: string, variables?: Record<string, string | number>) => string, error: unknown): string {
  if (isAgentGroupDisabled(error)) return t('messagehub.groupError.agent-group-disabled')
  const message = error instanceof Error ? error.message : String(error)
  const reason = message.replace(/^rejected:\s*/, '').match(/[a-z0-9]+(?:-[a-z0-9]+)+/)?.[0] ?? (message === 'agent_observer' ? 'agent_observer' : 'unknown')
  if (reason === 'agent_observer') return t('messagehub.reason.agent_observer')
  return knownGroupErrors.has(reason) ? t(`messagehub.groupError.${reason}`) : t('messagehub.groupError.generic', undefined, { reason })
}

/** Text of an invite link: the group DID plus the token issued by `group.create_invite_link`. */
export function formatInviteLink(groupDid: string, token: string): string {
  return `${groupDid}?invite=${encodeURIComponent(token)}`
}

/** A pasted invite link (or a bare group DID, which asks to join without a token). */
export function parseInviteLink(input: string): { groupDid: string; invite?: string } | null {
  const trimmed = input.trim()
  const match = trimmed.match(/^(did:[^?\s]+)(?:\?invite=([^&\s]+))?$/)
  if (!match) return null
  return match[2] ? { groupDid: match[1], invite: decodeURIComponent(match[2]) } : { groupDid: match[1] }
}

/** Whether an edit / recall window (`EditRule`) still allows acting on a message sent at `start`. */
export function withinWindow(windowMs: number | undefined, start: number, now: number): boolean {
  return windowMs === undefined || (windowMs > 0 && now - start <= windowMs)
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
