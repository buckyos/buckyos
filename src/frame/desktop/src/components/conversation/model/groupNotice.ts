import type { MessageObject } from './msgobj'
import type { GroupInvitation } from './types'

export const GROUP_INVITATION_INTENT = 'buckyos.group_invitation'

/** A group notification: from the inviter for `invite`, `session_invite`; from the group for `pending_approval`, `rejected`, `removed`, `owner_transfer`, `session_removed`. */
export interface GroupNotice {
  groupDid: string
  action: string
  /** Who acted, when the group itself is the sender. An invitation comes from the inviter instead. */
  by?: string
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
  const by = text(data.by)
  if (by) notice.by = by
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

const knownGroupErrors = new Set(['agent-group-disabled', 'capability-denied', 'member-already-participating', 'member-must-be-single-entity', 'member-limit', 'owner-must-transfer-first', 'owner-required', 'controller-required', 'role-not-allowed', 'invite-expired', 'invitation-expired', 'invitation-revoked', 'invite-already-expired', 'invitation-mismatch', 'transfer-mismatch', 'owner-transfer-pending', 'agent-owner-required', 'invite-required', 'join-not-allowed', 'member-not-pending', 'use-transfer-owner', 'cannot-moderate-owner', 'revision-conflict', 'invalid-invite-link', 'guests-not-allowed', 'guest-limit', 'not-a-session-guest', 'session-archived', 'post-not-allowed', 'not-found', 'blocked', 'rate-limited', 'invalid-group-name', 'session-limit', 'hook-denied', 'hook-unavailable', 'remote-group-unavailable', 'remote-group-auth-required', 'remote-group-auth-failed', 'remote-group-not-found', 'group-sync-pending', 'group-refresh-pending', 'unavailable'])

/** The Agent has not been allowed to join group chats (its Users and Agents setting). */
export function isAgentGroupDisabled(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error)
  return message.includes('agent-group-disabled') || message.includes('agent_group_disabled')
}

export function groupErrorCode(error: unknown): string {
  if (isAgentGroupDisabled(error)) return 'agent-group-disabled'
  const message = error instanceof Error ? error.message : String(error)
  const codes = message.replace(/^rejected:\s*/, '').match(/[a-z0-9]+(?:-[a-z0-9]+)+/g) ?? []
  return codes.find(code => knownGroupErrors.has(code)) ?? (knownGroupErrors.has(message) ? message : codes[0]) ?? (message === 'agent_observer' ? 'agent_observer' : 'unknown')
}

/** User-facing text for a group operation failure; unknown host reasons are shown verbatim. */
export function groupErrorText(t: (key: string, fallback?: string, variables?: Record<string, string | number>) => string, error: unknown): string {
  const reason = groupErrorCode(error)
  if (reason === 'agent_observer') return t('messagehub.reason.agent_observer')
  return knownGroupErrors.has(reason) ? t(`messagehub.groupError.${reason}`) : t('messagehub.groupError.generic', undefined, { reason })
}

/** Whether an edit / recall window (`EditRule`) still allows acting on a message sent at `start`. */
export function withinWindow(windowMs: number | undefined, start: number, now: number): boolean {
  return windowMs === undefined || (windowMs > 0 && now - start <= windowMs)
}
