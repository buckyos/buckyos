import { z } from 'zod'
import type { GroupMember, GroupRole, GroupSessionParticipant } from './types'

export const GROUP_MEMBER_LIMIT = 100
/** A group holds single native identities: people and Agents, never another group or an external account. */
export const GROUP_MEMBER_KINDS: Array<'user' | 'agent'> = ['user', 'agent']

export const createGroupSchema = z.object({
  name: z.string().trim().max(64),
  members: z.array(z.string().min(1)).max(GROUP_MEMBER_LIMIT),
})

/** Members that still occupy a slot: invited, waiting for approval or active. */
export const participating = (member: GroupMember) => member.state === 'active' || member.state === 'invited' || member.state === 'pending_admin_approval'

export const inviteBlocked = (member: GroupMember) => member.state === 'active' || member.state === 'pending_admin_approval'

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

/** Text of an invite link: the group DID plus the token issued by `group.create_invite_link`. */
export function formatInviteLink(groupDid: string, token: string): string {
  return `${groupDid}?invite=${encodeURIComponent(token)}`
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
