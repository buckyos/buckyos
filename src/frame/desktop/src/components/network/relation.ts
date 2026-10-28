/* ── Display rules of the network components (UI_DATAMODEL.md §2–§5); msg-center owns every decision ── */

import {
  CONTACT_TAG_HOMESTATION_FOLLOWING,
  FRIEND_INTENT,
  WELL_KNOWN_CONTACT_TAGS,
  contactTagApp,
  contactTagKind,
  type ContactTagKind,
} from 'buckyos'
import { parseUserGroupRef } from '../../api/user_type'
import { z } from 'zod'
import type {
  AccessGroupLevel,
  Contact,
  FriendRequest,
  FriendState,
  GroupInvitationItem,
  GroupRef,
  JoinRequestItem,
  LookupCandidate,
  LookupKind,
  MessageRequest,
  RequestSession,
  RequestItem,
} from './types'

export const GROUP_INVITATION_INTENT = 'buckyos.group_invitation'
export const FOLLOWING_TAG = CONTACT_TAG_HOMESTATION_FOLLOWING
export const FRIEND_MESSAGE_MAX = 500
export const USER_TAG_MAX = 64

type Translate = (key: string, fallback?: string, variables?: Record<string, string | number>) => string

/** msg-center timestamps: seconds in older records, milliseconds in newer ones. */
export function timeMs(value: number | undefined): number | undefined {
  if (value === undefined || !Number.isFinite(value) || value <= 0) return undefined
  return value > 1e12 ? value : value * 1000
}

// ── Relation ──

export interface RelationFacts {
  isContact: boolean
  friendState: FriendState
  accessLevel: AccessGroupLevel
  temporaryExpiresAtMs?: number
  tags: string[]
  pendingRequestId?: string
  /** Ordinary messages of this sender wait in REQUEST_BOX. */
  messageRequest?: boolean
  /** A system projection entry: it cannot be deleted (tags, a note and blocking still go to the owner's own record). */
  projected?: boolean
}

export function relationOfCandidate(candidate: LookupCandidate): RelationFacts {
  const relation = candidate.relation
  return {
    isContact: relation.is_contact,
    friendState: relation.friend_state ?? 'none',
    accessLevel: relation.access_level ?? 'stranger',
    temporaryExpiresAtMs: relation.temporary_expires_at_ms,
    tags: relation.tags ?? [],
    pendingRequestId: relation.pending_request_id,
  }
}

export function relationOfContact(contact: Contact, now = Date.now()): RelationFacts {
  const grant = contact.access_level === 'temporary'
    ? Math.max(0, ...(contact.temp_grants ?? []).map(item => timeMs(item.expires_at) ?? 0))
    : 0
  return {
    isContact: true,
    friendState: contact.friend_state ?? 'none',
    accessLevel: contact.access_level,
    temporaryExpiresAtMs: grant > now ? grant : undefined,
    tags: contact.tags ?? [],
    projected: (contact.projected_from?.length ?? 0) > 0,
  }
}

export const NOT_RELATED: RelationFacts = { isContact: false, friendState: 'none', accessLevel: 'stranger', tags: [] }

export type RelationBadgeKind =
  | 'blocked'
  | 'friend'
  | 'outgoing_pending'
  | 'incoming_pending'
  | 'following'
  | 'contact'
  | 'message_request'
  | 'stranger'
  | 'temporary'

/** §3.3: Block overrides everything; Friend is the only two-way state; Following is one-way and may sit next to Friend. */
export function relationBadges(facts: RelationFacts, now = Date.now()): RelationBadgeKind[] {
  if (facts.accessLevel === 'block') return ['blocked']
  const badges: RelationBadgeKind[] = []
  if (facts.friendState === 'friend') badges.push('friend')
  else if (facts.friendState === 'outgoing_pending') badges.push('outgoing_pending')
  else if (facts.friendState === 'incoming_pending') badges.push('incoming_pending')
  if (facts.tags.includes(FOLLOWING_TAG)) badges.push('following')
  if (badges.length === 0) badges.push(facts.isContact ? 'contact' : 'stranger')
  if (facts.messageRequest && facts.friendState === 'none') badges.push('message_request')
  if (facts.accessLevel === 'temporary' && (facts.temporaryExpiresAtMs ?? 0) > now) badges.push('temporary')
  return badges
}

/** Whether new messages of this person reach the inbox without a request. */
export function messagesAllowed(facts: RelationFacts, now = Date.now()) {
  return facts.accessLevel === 'allow' || (facts.accessLevel === 'temporary' && (facts.temporaryExpiresAtMs ?? 0) > now)
}

// ── Tags ──

export function tagKind(tag: string): ContactTagKind {
  return contactTagKind(tag)
}

export function tagApp(tag: string): string | null {
  return contactTagApp(tag)
}

export function isWellKnownTag(tag: string) {
  return WELL_KNOWN_CONTACT_TAGS.some(item => item.key === tag)
}

/**
 * Well-known tags have a Desktop display name; a projection source (`type:<type>`, `group:<id>`) shows
 * the account type or the group's name (its id when the name is not known); a user tag's name is its text.
 */
export function tagLabel(t: Translate, tag: string, groupNames?: ReadonlyMap<string, string>): string {
  if (isWellKnownTag(tag)) return t(`network.tag.${tag}`, tag)
  const ref = parseUserGroupRef(tag)
  if (ref?.kind === 'type') return t(`network.tag.type.${ref.userType}`)
  if (ref?.kind === 'group') return groupNames?.get(ref.groupId) ?? ref.groupId
  return tag
}

export const userTagSchema = z
  .string()
  .trim()
  .min(1, 'network.tag.error.required')
  .max(USER_TAG_MAX, 'network.tag.error.tooLong')
  .refine(value => contactTagKind(value) === 'user', 'network.tag.error.reserved')

export function userTags(tags: readonly string[]) {
  return tags.filter(tag => contactTagKind(tag) === 'user')
}

// ── Errors ──

const KNOWN_ERRORS = new Set([
  'invalid-input',
  'not-found',
  'unreachable',
  'forbidden',
  'system-relation',
  'projected-contact-read-only',
  'contact-edit-not-allowed',
  'recipient-has-no-inbox',
  'tag-not-writable',
  'invalid-tag',
  'peer-blocked',
  'friend-request-cooldown',
  'cannot-befriend-self',
  'peer-not-befriendable',
  'message-too-long',
  'request-not-pending',
  'request-expired',
  'owner-not-hosted',
  'contact_mgr_owner-required',
  'invite-required',
  'join-not-allowed',
  'invalid-invite-link',
  'invite-expired',
  'invitation-expired',
  'invitation-revoked',
  'member-already-participating',
  'remote-group-unsupported',
  'remote-group-unavailable',
  'remote-group-auth-required',
  'remote-group-auth-failed',
  'remote-group-not-found',
  'group-sync-pending',
  'group-refresh-pending',
  'invitation-mismatch',
  'member-list-hidden',
  'capability-denied',
  'owner-must-transfer-first',
  'blocked',
  'rate-limited',
  'unavailable',
])

/** The reason code msg-center put in an RPC error (`system-relation`, `invite-required`, …). */
export function networkErrorCode(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error ?? '')
  const lower = message.toLowerCase()
  const codes = lower.match(/[a-z0-9_]+(?:-[a-z0-9_]+)+/g) ?? []
  const known = codes.find(code => KNOWN_ERRORS.has(code)) ?? (/\bblocked\b/.test(lower) ? 'blocked' : undefined)
  if (known) return known
  if (lower.includes('failed to fetch') || lower.includes('network') || lower.includes('timeout')) return 'unavailable'
  if (lower.includes('permission')) return 'forbidden'
  return codes[0] ?? 'unknown'
}

export function networkErrorText(t: Translate, error: unknown): string {
  const code = networkErrorCode(error)
  return KNOWN_ERRORS.has(code)
    ? t(`network.error.${code}`)
    : t('network.error.generic', undefined, { reason: code })
}

// ── Requests ──

function pendingPeers(requests: readonly FriendRequest[]) {
  return new Set(requests.filter(request => request.state === 'pending').map(request => request.peer_did))
}

/** §3.3 “message request”: a conversation with REQUEST_BOX messages from a sender with no pending friend request who is neither a friend, allowed nor blocked. */
export function messageRequests(sessions: readonly RequestSession[], contacts: readonly Contact[], friendRequests: readonly FriendRequest[], now = Date.now()): MessageRequest[] {
  const pending = pendingPeers(friendRequests)
  const byDid = new Map(contacts.map(contact => [contact.did, contact]))
  return sessions
    .filter((session) => {
      if (session.count <= 0 || pending.has(session.peerDid)) return false
      if ((session.intent === FRIEND_INTENT || session.intent === GROUP_INVITATION_INTENT) && session.count <= 1) return false
      const contact = byDid.get(session.peerDid)
      if (!contact) return true
      const facts = relationOfContact(contact, now)
      return facts.accessLevel !== 'block' && facts.friendState !== 'friend' && !messagesAllowed(facts, now)
    })
    .map(session => ({
      senderDid: session.peerDid,
      senderName: byDid.get(session.peerDid)?.name || session.peerName,
      count: session.count,
      lastAtMs: session.lastAtMs,
      preview: session.intent === FRIEND_INTENT || session.intent === GROUP_INVITATION_INTENT ? undefined : session.preview,
      sessionId: session.sessionId,
    }))
    .sort((a, b) => b.lastAtMs - a.lastAtMs)
}

/** Invitations still worth answering: not expired, and not for a group the owner already belongs to. */
export function pendingInvitations(invitations: readonly GroupInvitationItem[], groups: readonly GroupRef[], now = Date.now()) {
  const active = new Set(groups.filter(group => group.state === 'active').map(group => group.did))
  return invitations
    .filter(item => (item.expiresAtMs ?? Infinity) > now && !active.has(item.groupDid))
    .sort((a, b) => (b.receivedAtMs ?? 0) - (a.receivedAtMs ?? 0))
}

export interface RequestSources {
  friendRequests: readonly FriendRequest[]
  messageRequests: readonly MessageRequest[]
  invitations: readonly GroupInvitationItem[]
  joinRequests: readonly JoinRequestItem[]
}

/** The four Requests item types in display order; incoming friend requests first. */
export function requestItems(sources: RequestSources): RequestItem[] {
  const friend = [...sources.friendRequests]
    .filter(request => request.state === 'pending')
    .sort((a, b) => Number(a.direction === 'outgoing') - Number(b.direction === 'outgoing') || b.created_at_ms - a.created_at_ms)
  return [
    ...friend.map(request => ({ type: 'friend_request' as const, key: `friend:${request.request_id}`, request })),
    ...sources.messageRequests.map(request => ({ type: 'message_request' as const, key: `message:${request.senderDid}`, request })),
    ...sources.invitations.map(invitation => ({ type: 'group_invitation' as const, key: `invite:${invitation.inviteId}`, invitation })),
    ...sources.joinRequests.map(request => ({ type: 'join_request' as const, key: `join:${request.groupDid}:${request.memberDid}`, request })),
  ]
}

/** Items that wait for the owner's decision (outgoing friend requests only wait for the peer). */
export function actionableCount(items: readonly RequestItem[]) {
  return items.filter(item => item.type !== 'friend_request' || item.request.direction === 'incoming').length
}

// ── Actions ──

export type RelationActionId =
  | 'add_friend'
  | 'accept_request'
  | 'cancel_request'
  | 'allow_messages'
  | 'block'
  | 'unblock'
  | 'unfriend'
  | 'remove_contact'
  | 'remove_from_collection'

/** The relationship actions that make sense for the person's current relation (order = display order). */
export function availableActions(facts: RelationFacts, options: { collectionTag?: string; kind?: LookupKind } = {}, now = Date.now()): RelationActionId[] {
  const actions: RelationActionId[] = []
  if (options.collectionTag && contactTagKind(options.collectionTag) === 'user' && facts.tags.includes(options.collectionTag)) actions.push('remove_from_collection')
  const removable = facts.isContact && !facts.projected
  if (facts.accessLevel === 'block') {
    actions.push('unblock')
    if (removable) actions.push('remove_contact')
    return actions
  }
  if (facts.friendState === 'none' && options.kind !== 'msgtunnel' && options.kind !== 'group') actions.push('add_friend')
  if (facts.friendState === 'incoming_pending' && facts.pendingRequestId) actions.push('accept_request')
  if (facts.friendState === 'outgoing_pending' && facts.pendingRequestId) actions.push('cancel_request')
  if (facts.friendState !== 'friend' && !messagesAllowed(facts, now)) actions.push('allow_messages')
  if (facts.friendState === 'friend') actions.push('unfriend')
  actions.push('block')
  if (removable) actions.push('remove_contact')
  return actions
}

/** A contact record does not carry its pending request id; the friend request list does. */
export function withPendingRequest(facts: RelationFacts, did: string, friendRequests: readonly FriendRequest[] | undefined): RelationFacts {
  const request = friendRequests?.find(item => item.peer_did === did && item.state === 'pending')
  if (!request) return facts
  return {
    ...facts,
    pendingRequestId: request.request_id,
    friendState: facts.friendState === 'none' ? (request.direction === 'incoming' ? 'incoming_pending' : 'outgoing_pending') : facts.friendState,
  }
}
