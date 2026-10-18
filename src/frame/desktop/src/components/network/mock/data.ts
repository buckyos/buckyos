/* ── Mock relationship world: the owner's contacts, this Zone, other Zones, groups and requests ── */

import type { Contact, FriendRequest, LookupKind, ReachabilityState } from 'buckyos'
import type { GroupInvitationItem, GroupRef, JoinRequestItem, RequestSession } from '../types'

export const MOCK_OWNER_DID = 'did:bns:alice'
export const MOCK_ZONE = 'alice.buckyos.io'

const HOUR = 3600_000
const DAY = 24 * HOUR

export interface MockPerson {
  did: string
  kind: LookupKind
  name: string
  zone?: string
  reachability: ReachabilityState
  reason?: string
  /** A user or Agent of this Zone (msg-center makes them friends on its own). */
  zoneMember?: boolean
}

export interface MockGroup {
  did: string
  name: string
  description: string
  owner: string
  host: string
  joinPolicy: 'open' | 'request_and_approve' | 'invite_only'
  memberCount: number
  hostedHere: boolean
  invite?: string
}

export interface MockWorld {
  people: Map<string, MockPerson>
  contacts: Map<string, Contact>
  groups: Map<string, MockGroup>
  joined: Map<string, GroupRef>
  pendingJoins: Set<string>
  friendRequests: FriendRequest[]
  requestSessions: RequestSession[]
  invitations: GroupInvitationItem[]
  /** State of the invitation notices (`dm:<inviter>` records) by record id. */
  notices: Map<string, 'UNREAD' | 'ARCHIVED'>
  joinRequests: JoinRequestItem[]
}

const people: MockPerson[] = [
  { did: MOCK_OWNER_DID, kind: 'user', name: 'Alice', zone: MOCK_ZONE, reachability: 'local', zoneMember: true },
  { did: 'did:bns:bob', kind: 'user', name: 'Bob', zone: MOCK_ZONE, reachability: 'local', zoneMember: true },
  { did: 'did:bns:carol', kind: 'user', name: 'Carol', zone: MOCK_ZONE, reachability: 'local', zoneMember: true },
  { did: 'did:bns:dave', kind: 'user', name: 'Dave', zone: MOCK_ZONE, reachability: 'local', zoneMember: true },
  { did: 'did:bns:assistant.alice', kind: 'agent', name: 'BuckyOS Assistant', zone: MOCK_ZONE, reachability: 'local', zoneMember: true },
  { did: 'did:bns:eve', kind: 'user', name: 'Eve', zone: 'eve.buckyos.io', reachability: 'routable' },
  { did: 'did:bns:frank', kind: 'user', name: 'Frank', zone: 'frank.example.com', reachability: 'routable' },
  { did: 'did:msgtunnel:telegram:grace_g', kind: 'msgtunnel', name: 'Grace.telegram', reachability: 'tunnel' },
  { did: 'did:bns:heidi', kind: 'user', name: 'Heidi', zone: 'heidi.example.org', reachability: 'routable' },
  { did: 'did:bns:ivan', kind: 'user', name: 'Ivan', zone: 'ivan.buckyos.io', reachability: 'routable' },
  { did: 'did:msgtunnel:telegram:judy_j', kind: 'msgtunnel', name: 'Judy.telegram', reachability: 'tunnel' },
  { did: 'did:bns:kevin', kind: 'user', name: 'Kevin', zone: 'kevin.example.org', reachability: 'routable' },
  { did: 'did:bns:lily', kind: 'user', name: 'Lily', zone: 'lily.buckyos.io', reachability: 'routable' },
  { did: 'did:bns:mallory', kind: 'user', name: 'Mallory', zone: 'mallory.example.net', reachability: 'routable' },
  { did: 'did:bns:nina', kind: 'user', name: 'Nina', zone: 'nina.example.net', reachability: 'routable' },
  { did: 'did:bns:nora', kind: 'user', name: 'Nora', zone: 'nora.example.com', reachability: 'routable' },
  { did: 'did:bns:oscar', kind: 'user', name: 'Oscar', zone: 'oscar.example.net', reachability: 'routable' },
  { did: 'did:bns:peggy', kind: 'user', name: 'Peggy', zone: 'peggy.buckyos.io', reachability: 'routable' },
  { did: 'did:bns:quinn', kind: 'user', name: 'Quinn', zone: 'quinn.example.org', reachability: 'unreachable', reason: 'zone-gateway-unreachable' },
  { did: 'did:bns:ruth', kind: 'user', name: 'Ruth', zone: 'ruth.buckyos.io', reachability: 'routable' },
  { did: 'did:bns:victor', kind: 'user', name: 'Victor', zone: 'victor.example.com', reachability: 'routable' },
  { did: 'did:bns:wendy', kind: 'user', name: 'Wendy', zone: 'wendy.example.com', reachability: 'routable' },
]

const groups: MockGroup[] = [
  { did: 'did:bns:proj-alpha', name: 'Project Alpha', description: 'Cross-team project collaboration group.', owner: MOCK_OWNER_DID, host: MOCK_ZONE, joinPolicy: 'request_and_approve', memberCount: 5, hostedHere: true },
  { did: 'did:bns:family.alice', name: 'Family Space', description: 'Family members sharing the space.', owner: 'did:bns:carol', host: MOCK_ZONE, joinPolicy: 'invite_only', memberCount: 3, hostedHere: true },
  { did: 'did:bns:buckyos-community', name: 'BuckyOS Community', description: 'Open community for BuckyOS users.', owner: 'did:bns:frank', host: 'community.buckyos.org', joinPolicy: 'open', memberCount: 128, hostedHere: false },
  { did: 'did:bns:book-club', name: 'Book Club', description: 'One book a month, talked over on Sundays.', owner: 'did:bns:carol', host: MOCK_ZONE, joinPolicy: 'open', memberCount: 6, hostedHere: true },
  { did: 'did:bns:design-guild', name: 'Design Guild', description: 'Interface and product designers of this Zone.', owner: 'did:bns:bob', host: MOCK_ZONE, joinPolicy: 'request_and_approve', memberCount: 12, hostedHere: true },
  { did: 'did:bns:core-team', name: 'Core Team', description: 'Maintainers only.', owner: 'did:bns:bob', host: MOCK_ZONE, joinPolicy: 'invite_only', memberCount: 4, hostedHere: true, invite: 'core-2026' },
  { did: 'did:bns:garden.eve', name: "Eve's Garden", description: 'Plants, seeds and weekend swaps.', owner: 'did:bns:eve', host: 'eve.buckyos.io', joinPolicy: 'open', memberCount: 23, hostedHere: false },
  { did: 'did:bns:hiking-club', name: 'Hiking Club', description: 'Weekend trails around the bay.', owner: 'did:bns:frank', host: MOCK_ZONE, joinPolicy: 'invite_only', memberCount: 9, hostedHere: true },
]

function contact(did: string, now: number, patch: Partial<Contact> & { ageDays: number }): Contact {
  const person = people.find(item => item.did === did)
  const { ageDays, ...rest } = patch
  return {
    did,
    name: person?.name ?? did,
    source: 'manual_create',
    is_verified: false,
    access_level: 'stranger',
    friend_state: 'none',
    tags: [],
    created_at: now - ageDays * DAY,
    updated_at: now - Math.min(ageDays, 2) * DAY,
    ...rest,
  }
}

function contacts(now: number): Contact[] {
  return [
    contact('did:bns:bob', now, { ageDays: 300, source: 'shared', is_verified: true, access_level: 'allow', friend_state: 'friend', tags: ['zone_user'] }),
    contact('did:bns:carol', now, { ageDays: 280, source: 'shared', is_verified: true, access_level: 'allow', friend_state: 'friend', tags: ['zone_user'] }),
    contact('did:bns:assistant.alice', now, { ageDays: 90, source: 'shared', is_verified: true, access_level: 'allow', friend_state: 'friend', tags: ['zone_agent'] }),
    contact('did:bns:eve', now, {
      ageDays: 200,
      access_level: 'allow',
      friend_state: 'friend',
      note: 'Met at BuckyOS Meetup 2025',
      tags: ['Project Alpha Team', 'homestation.following'],
      bindings: [{ platform: 'telegram', account_id: '@eve_t', display_id: '@eve_t', tunnel_instance_id: 'tg-main', account_type: 'user', last_active_at: now - 3 * DAY }],
    }),
    contact('did:bns:frank', now, { ageDays: 180, access_level: 'allow', friend_state: 'friend', tags: ['Project Alpha Team'] }),
    contact('did:msgtunnel:telegram:grace_g', now, {
      ageDays: 40,
      source: 'auto_inferred',
      tags: [],
      bindings: [{ platform: 'telegram', account_id: '@grace_g', display_id: '@grace_g', tunnel_instance_id: 'tg-main', account_type: 'user', last_active_at: now - DAY }],
    }),
    contact('did:bns:heidi', now, {
      ageDays: 30,
      source: 'manual_import',
      note: 'Imported from CSV on 2026-01-15',
      bindings: [{ platform: 'email', account_id: 'heidi@mail.com', display_id: 'heidi@mail.com', tunnel_instance_id: 'email-main', account_type: 'user', last_active_at: now - 20 * DAY }],
    }),
    contact('did:bns:ivan', now, { ageDays: 3, friend_state: 'outgoing_pending', tags: ['Spring Trip'] }),
    contact('did:msgtunnel:telegram:judy_j', now, { ageDays: 12, source: 'auto_inferred' }),
    contact('did:bns:kevin', now, { ageDays: 60, access_level: 'block', note: 'Sent repeated spam invitations.' }),
    contact('did:bns:lily', now, { ageDays: 1, tags: ['homestation.following', 'Project Alpha Team'] }),
    contact('did:bns:mallory', now, {
      ageDays: 2,
      access_level: 'temporary',
      temp_grants: [{ context_id: 'design-review', granted_at: now - DAY, expires_at: now + 2 * DAY }],
      tags: ['Spring Trip'],
    }),
  ]
}

function joined(): GroupRef[] {
  return [
    { did: 'did:bns:proj-alpha', name: 'Project Alpha', description: 'Cross-team project collaboration group.', owner: MOCK_OWNER_DID, host: MOCK_ZONE, joinPolicy: 'request_and_approve', memberCount: 5, role: 'owner', state: 'active', hostedHere: true, canPost: true },
    { did: 'did:bns:family.alice', name: 'Family Space', description: 'Family members sharing the space.', owner: 'did:bns:carol', host: MOCK_ZONE, joinPolicy: 'invite_only', memberCount: 3, role: 'admin', state: 'active', hostedHere: true, canPost: true },
    { did: 'did:bns:buckyos-community', name: 'BuckyOS Community', description: 'Open community for BuckyOS users.', owner: 'did:bns:frank', host: 'community.buckyos.org', joinPolicy: 'open', memberCount: 128, role: 'member', state: 'active', hostedHere: false, canPost: false, postBlockedReason: 'post-not-allowed' },
  ]
}

export function createMockWorld(options: { empty: boolean }, now = Date.now()): MockWorld {
  const world: MockWorld = {
    people: new Map(people.map(person => [person.did, person])),
    contacts: new Map(),
    groups: new Map(groups.map(group => [group.did, group])),
    joined: new Map(),
    pendingJoins: new Set(),
    friendRequests: [],
    requestSessions: [],
    invitations: [],
    notices: new Map(),
    joinRequests: [],
  }
  if (options.empty) return world
  for (const item of contacts(now)) world.contacts.set(item.did, item)
  for (const group of joined()) world.joined.set(group.did, group)
  world.friendRequests = [
    { request_id: 'fr-nina', peer_did: 'did:bns:nina', direction: 'incoming', state: 'pending', message: 'Hi Alice, we met at the BuckyOS meetup last week.', peer_name: 'Nina', msg_id: 'msg-fr-nina', created_at_ms: now - 5 * HOUR, expires_at_ms: now + 6 * DAY, updated_at_ms: now - 5 * HOUR },
    { request_id: 'fr-peggy', peer_did: 'did:bns:peggy', direction: 'incoming', state: 'pending', peer_name: 'Peggy', msg_id: 'msg-fr-peggy', created_at_ms: now - 2 * DAY, expires_at_ms: now + 5 * DAY, updated_at_ms: now - 2 * DAY },
    { request_id: 'fr-ivan', peer_did: 'did:bns:ivan', direction: 'outgoing', state: 'pending', message: 'Let us keep in touch about the spring trip.', delivery: { state: 'delivered' }, created_at_ms: now - 3 * DAY, expires_at_ms: now + 4 * DAY, updated_at_ms: now - 3 * DAY },
  ]
  world.requestSessions = [
    { sessionId: 'dm:did:bns:victor', peerDid: 'did:bns:victor', peerName: 'Victor', count: 2, lastAtMs: now - HOUR, preview: 'I also have a question about your HomeStation post.' },
    { sessionId: 'dm:did:bns:wendy', peerDid: 'did:bns:wendy', peerName: 'Wendy', count: 1, lastAtMs: now - 30 * 60_000, preview: 'Hi! Can we talk about the Design Guild?' },
    { sessionId: 'dm:did:bns:nina', peerDid: 'did:bns:nina', peerName: 'Nina', count: 1, lastAtMs: now - 5 * HOUR, intent: 'buckyos.friend', preview: 'Hi Alice, we met at the BuckyOS meetup last week.' },
    { sessionId: 'dm:did:bns:peggy', peerDid: 'did:bns:peggy', peerName: 'Peggy', count: 1, lastAtMs: now - 2 * DAY, intent: 'buckyos.friend' },
  ]
  world.invitations = [
    { groupDid: 'did:bns:hiking-club', groupName: 'Hiking Club', recordId: 'notice-hiking', inviteId: 'inv-hiking-1', inviterDid: 'did:bns:frank', role: 'member', receivedAtMs: now - 6 * HOUR, expiresAtMs: now + 5 * DAY },
  ]
  world.notices.set('notice-hiking', 'UNREAD')
  world.joinRequests = [
    { groupDid: 'did:bns:proj-alpha', groupName: 'Project Alpha', memberDid: 'did:bns:ruth', entityKind: 'user' },
  ]
  return world
}
