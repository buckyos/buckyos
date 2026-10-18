/* ── Mock msg-center relationship surface: simulated latency, one-shot failures, change events ──
 *
 * `?networkScenario=empty|error|slow` (comma separated) selects the empty world, a first failure of
 * every read (retry succeeds) and four times the latency.
 */

import type { Contact, FriendRequest, FriendRequestResult, LookupCandidate, LookupKind, LookupResult } from 'buckyos'
import type { GroupInvitationItem, GroupRef, JoinOutcome, JoinRequestItem, NetworkBackend, NetworkChange } from '../types'
import { createMockWorld, MOCK_OWNER_DID, MOCK_ZONE, type MockGroup, type MockWorld } from './data'
import { networkMockHosts, onNetworkMockHosts, type NetworkMockGroup } from './hosts'

const DAY = 24 * 3600_000

function scenario() {
  const values = new URLSearchParams(window.location.search).getAll('networkScenario').flatMap(value => value.split(','))
  return { empty: values.includes('empty'), error: values.includes('error'), slow: values.includes('slow') }
}

class MockError extends Error {
  constructor(code: string) {
    super(`rejected: ${code}`)
    this.name = 'MockNetworkError'
  }
}

const isSystemRelation = (contact: Contact | undefined) => Boolean(contact?.tags?.some(tag => tag === 'zone_user' || tag === 'zone_agent'))
const isUserTag = (tag: string) => tag !== 'zone_user' && tag !== 'zone_agent' && !/^[a-z][a-z0-9_-]*\.[a-z0-9_.-]+$/.test(tag)

export function createMockNetworkBackend(): NetworkBackend {
  const options = scenario()
  const world: MockWorld = createMockWorld({ empty: options.empty })
  const listeners = new Set<(change: NetworkChange) => void>()
  const failed = new Set<string>()
  let sequence = 0

  const delay = (scale = 1) => new Promise<void>(resolve => window.setTimeout(resolve, (240 + Math.random() * 260) * scale * (options.slow ? 4 : 1)))
  const emit = (...changes: NetworkChange[]) => {
    window.setTimeout(() => { for (const change of changes) listeners.forEach(listener => listener(change)) }, 40)
  }
  const syncHosts = () => {
    let added = false
    for (const host of networkMockHosts()) {
      for (const person of host.people()) {
        if (person.did === MOCK_OWNER_DID || world.people.has(person.did)) continue
        world.people.set(person.did, { did: person.did, kind: person.kind, name: person.name, zone: person.zone, reachability: person.reachability })
        if (!world.contacts.has(person.did)) {
          world.contacts.set(person.did, {
            did: person.did,
            name: person.name,
            ...(person.avatar ? { avatar: person.avatar } : {}),
            source: 'manual_create',
            is_verified: false,
            access_level: person.friend ? 'allow' : 'stranger',
            friend_state: person.friend ? 'friend' : 'none',
            tags: [],
            created_at: Date.now(),
            updated_at: Date.now(),
          })
        }
        added = true
      }
    }
    if (added) emit('contacts', 'groups')
  }
  syncHosts()
  onNetworkMockHosts(syncHosts)
  const hostGroup = (did: string): { group: NetworkMockGroup; join: (invite?: string) => Promise<JoinOutcome> } | null => {
    for (const host of networkMockHosts()) {
      const group = host.group(did)
      if (group) return { group, join: invite => host.requestJoin(did, invite) }
    }
    return null
  }
  const hostGroupCandidate = (group: NetworkMockGroup, invite?: string): LookupCandidate => ({
    did: group.did,
    kind: 'group',
    section: group.myState === 'active' ? 'relation' : 'resolved',
    name: group.name,
    ...(group.description ? { note: group.description } : {}),
    reachability: { state: 'local', zone: MOCK_ZONE },
    relation: { is_contact: false, ...(group.myState ? { group_state: group.myState } : {}), ...(group.myRole ? { group_role: group.myRole } : {}) },
    group: { join_policy: group.joinPolicy, hosted_here: true, host: MOCK_ZONE, owner: group.owner, member_count: group.memberCount, ...(invite ? { invite } : {}) },
  })
  const read = async <T>(name: string, run: () => T): Promise<T> => {
    await delay()
    if (options.error && !failed.has(name)) {
      failed.add(name)
      throw new Error('Failed to fetch: mock network unavailable')
    }
    return structuredClone(run())
  }
  const write = async <T>(run: () => T, ...changes: NetworkChange[]): Promise<T> => {
    await delay(0.8)
    const result = run()
    emit(...changes)
    return structuredClone(result)
  }
  const now = () => Date.now()

  const ensureContact = (did: string, name?: string): Contact => {
    const existing = world.contacts.get(did)
    if (existing) return existing
    const person = world.people.get(did)
    const created: Contact = {
      did,
      name: name || person?.name || did,
      source: 'manual_create',
      is_verified: false,
      access_level: 'stranger',
      friend_state: world.friendRequests.some(request => request.peer_did === did && request.state === 'pending' && request.direction === 'incoming') ? 'incoming_pending' : 'none',
      tags: [],
      created_at: now(),
      updated_at: now(),
    }
    world.contacts.set(did, created)
    return created
  }
  const touch = (contact: Contact) => { contact.updated_at = now() }
  const pendingRequest = (did: string) => world.friendRequests.find(request => request.peer_did === did && request.state === 'pending')
  const settle = (request: FriendRequest, state: FriendRequest['state']) => {
    request.state = state
    request.updated_at_ms = now()
  }
  const becomeFriends = (did: string, name?: string) => {
    const contact = ensureContact(did, name)
    contact.friend_state = 'friend'
    if (contact.access_level !== 'block') contact.access_level = 'allow'
    contact.temp_grants = []
    touch(contact)
    return contact
  }

  const groupCandidate = (group: MockGroup, invite?: string): LookupCandidate => {
    const ref = world.joined.get(group.did)
    const pending = world.pendingJoins.has(group.did)
    return {
      did: group.did,
      kind: 'group',
      section: ref ? 'relation' : 'resolved',
      name: group.name,
      note: group.description,
      reachability: group.hostedHere ? { state: 'local', zone: MOCK_ZONE } : { state: 'routable', zone: group.host },
      relation: {
        is_contact: false,
        ...(ref ? { group_state: ref.state, group_role: ref.role } : pending ? { group_state: 'pending_admin_approval' } : {}),
      },
      group: {
        join_policy: group.joinPolicy,
        hosted_here: group.hostedHere,
        host: group.host,
        owner: group.owner,
        member_count: group.memberCount,
        ...(invite ? { invite } : {}),
      },
    }
  }

  const personCandidate = (did: string, section: LookupCandidate['section']): LookupCandidate | null => {
    const person = world.people.get(did)
    const contact = world.contacts.get(did)
    if (!person && !contact) return null
    const request = pendingRequest(did)
    const grant = contact?.access_level === 'temporary' ? Math.max(0, ...(contact.temp_grants ?? []).map(item => item.expires_at)) : 0
    return {
      did,
      kind: person?.kind ?? 'unknown',
      section,
      name: contact?.name || person?.name,
      ...(contact?.avatar ? { avatar: contact.avatar } : {}),
      ...(contact?.note ? { note: contact.note } : {}),
      reachability: { state: person?.reachability ?? 'unknown', ...(person?.zone ? { zone: person.zone } : {}), ...(person?.reason ? { reason: person.reason } : {}) },
      relation: {
        is_contact: Boolean(contact),
        friend_state: contact?.friend_state ?? (request ? (request.direction === 'incoming' ? 'incoming_pending' : 'outgoing_pending') : 'none'),
        access_level: contact?.access_level ?? 'stranger',
        ...(grant > now() ? { temporary_expires_at_ms: grant } : {}),
        tags: contact?.tags ?? [],
        ...(request ? { pending_request_id: request.request_id } : {}),
      },
    }
  }

  const sectionOf = (did: string): LookupCandidate['section'] => {
    if (world.contacts.has(did)) return 'relation'
    return world.people.get(did)?.zoneMember ? 'zone' : 'resolved'
  }

  const lookup = (input: string, kinds?: LookupKind[], limit = 30): LookupResult => {
    const text = input.trim()
    const keep = (items: LookupCandidate[]) => items.filter(item => !kinds?.length || kinds.includes(item.kind)).slice(0, limit)
    if (!text) {
      const relation = [...world.contacts.keys()].map(did => personCandidate(did, 'relation')).filter(Boolean) as LookupCandidate[]
      const zone = [...world.people.values()].filter(person => person.zoneMember && person.did !== MOCK_OWNER_DID && !world.contacts.has(person.did)).map(person => personCandidate(person.did, 'zone')!)
      return { input, input_kind: 'empty', candidates: keep([...relation, ...zone]) }
    }
    const link = text.match(/^(did:[^?\s]+)\?invite=([^&\s]+)$/)
    if (link) {
      const group = world.groups.get(link[1])
      const hosted = group ? null : hostGroup(link[1])
      if (!group && !hosted) return { input, input_kind: 'invite_link', candidates: [], error: { code: 'not-found' } }
      const invite = decodeURIComponent(link[2])
      return { input, input_kind: 'invite_link', candidates: keep([group ? groupCandidate(group, invite) : hostGroupCandidate(hosted!.group, invite)]) }
    }
    if (text.startsWith('did:')) {
      if (!/^did:[a-z0-9]+:[^\s?#]+$/i.test(text)) return { input, input_kind: 'did', candidates: [], error: { code: 'invalid-input' } }
      if (text === 'did:bns:secret') return { input, input_kind: 'did', candidates: [], error: { code: 'forbidden' } }
      const group = world.groups.get(text)
      const hosted = group ? null : hostGroup(text)
      const candidate = group ? groupCandidate(group) : hosted ? hostGroupCandidate(hosted.group) : text === MOCK_OWNER_DID ? null : personCandidate(text, sectionOf(text))
      return candidate
        ? { input, input_kind: 'did', candidates: keep([candidate]) }
        : { input, input_kind: 'did', candidates: [], error: { code: 'not-found' } }
    }
    if (!/[\p{L}\p{N}]/u.test(text)) return { input, input_kind: 'text', candidates: [], error: { code: 'invalid-input' } }
    const needle = text.toLowerCase()
    const matches = (...values: (string | undefined)[]) => values.some(value => value?.toLowerCase().includes(needle))
    const relation = [...world.contacts.values()]
      .filter(item => matches(item.name, item.did, item.note, ...(item.tags ?? [])))
      .map(item => personCandidate(item.did, 'relation')!)
    const groups = [
      ...[...world.joined.values()].filter(group => matches(group.name, group.did)).map(group => groupCandidate(world.groups.get(group.did)!)),
      ...networkMockHosts().flatMap(host => host.groups()).filter(group => matches(group.name, group.did)).map(group => hostGroupCandidate(group)),
    ]
    const zone = [...world.people.values()]
      .filter(person => person.zoneMember && person.did !== MOCK_OWNER_DID && !world.contacts.has(person.did) && matches(person.name, person.did))
      .map(person => personCandidate(person.did, 'zone')!)
    const found = [...relation, ...groups, ...zone]
    const bns = /^[a-z0-9-]+(\.[a-z0-9-]+)*$/.test(needle)
    const resolvedDid = `did:bns:${needle}`
    if (bns && !found.some(item => item.did === resolvedDid)) {
      const group = world.groups.get(resolvedDid)
      const resolved = group ? groupCandidate(group) : resolvedDid === MOCK_OWNER_DID ? null : personCandidate(resolvedDid, 'resolved')
      if (resolved) found.push(resolved)
    }
    if (found.length === 0 && bns) return { input, input_kind: 'name', candidates: [], error: { code: 'not-found' } }
    return { input, input_kind: bns ? 'name' : 'text', candidates: keep(found) }
  }

  const join = (group: MockGroup, role: GroupRef['role'] = 'member'): JoinOutcome => {
    world.joined.set(group.did, {
      did: group.did,
      name: group.name,
      description: group.description,
      owner: group.owner,
      host: group.host,
      joinPolicy: group.joinPolicy,
      memberCount: group.memberCount + 1,
      role,
      state: 'active',
      hostedHere: group.hostedHere,
      canPost: true,
    })
    world.pendingJoins.delete(group.did)
    return 'active'
  }

  const joinOwn = (groupDid: string, invite?: string) => write((): JoinOutcome => {
      const group = world.groups.get(groupDid)
      if (!group) throw new MockError('not-found')
      if (!group.hostedHere) throw new MockError('remote-group-unsupported')
      if (world.joined.has(groupDid)) throw new MockError('member-already-participating')
      if (invite) {
        if (invite !== group.invite) throw new MockError('invalid-invite-link')
        return join(group)
      }
      if (group.joinPolicy === 'invite_only') throw new MockError('invite-required')
      if (group.joinPolicy === 'request_and_approve') {
        world.pendingJoins.add(groupDid)
        return 'pending_admin_approval'
      }
      return join(group)
  }, 'groups')

  const backend: NetworkBackend = {
    isMock: true,
    ownerDid: async () => MOCK_OWNER_DID,
    shareText: async () => MOCK_OWNER_DID,
    lookup: async (input, opts) => {
      await delay(0.7)
      return structuredClone(lookup(input, opts?.kinds, opts?.limit))
    },
    listContacts: () => read('contacts', () => { syncHosts(); return [...world.contacts.values()] }),
    saveContact: (did, name) => write(() => ensureContact(did, name), 'contacts'),
    updateTags: (did, change) => write(() => {
      const bad = [...(change.add ?? []), ...(change.remove ?? [])].find(tag => !isUserTag(tag))
      if (bad) throw new MockError('tag-not-allowed')
      const contact = ensureContact(did)
      const tags = new Set(contact.tags ?? [])
      for (const tag of change.remove ?? []) tags.delete(tag)
      for (const tag of change.add ?? []) tags.add(tag.trim())
      contact.tags = [...tags]
      touch(contact)
      return contact
    }, 'contacts'),
    removeContact: (did) => write(() => {
      const contact = world.contacts.get(did)
      if (!contact) throw new MockError('not-found')
      if (isSystemRelation(contact)) throw new MockError('system-relation')
      world.contacts.delete(did)
    }, 'contacts'),
    blockContact: (did) => write(() => {
      const contact = ensureContact(did)
      contact.access_level = 'block'
      contact.temp_grants = []
      touch(contact)
      const request = pendingRequest(did)
      if (request?.direction === 'incoming') settle(request, 'ignored')
    }, 'contacts', 'requests'),
    unblockContact: (did) => write(() => {
      const contact = world.contacts.get(did)
      if (!contact) throw new MockError('not-found')
      if (contact.access_level === 'block') contact.access_level = contact.friend_state === 'friend' ? 'allow' : 'stranger'
      touch(contact)
      return contact
    }, 'contacts'),
    allowMessages: (did, name) => write(() => {
      const contact = ensureContact(did, name)
      if (contact.access_level !== 'block') contact.access_level = 'allow'
      touch(contact)
      return contact
    }, 'contacts'),
    sendFriendRequest: (did, message) => write((): FriendRequestResult => {
      const person = world.people.get(did)
      const contact = world.contacts.get(did)
      if (did === MOCK_OWNER_DID) throw new MockError('self')
      if (contact?.access_level === 'block') throw new MockError('blocked')
      if (contact?.friend_state === 'friend') return { friend_state: 'friend' }
      if (person?.reachability === 'unreachable') throw new MockError('unreachable')
      const existing = pendingRequest(did)
      if (existing?.direction === 'incoming') {
        settle(existing, 'accepted')
        becomeFriends(did)
        return { friend_state: 'friend', request: existing }
      }
      if (person?.zoneMember) {
        const created = becomeFriends(did)
        created.source = 'shared'
        created.is_verified = true
        created.tags = [...new Set([...(created.tags ?? []), person.kind === 'agent' ? 'zone_agent' : 'zone_user'])]
        return { friend_state: 'friend' }
      }
      if (existing) return { friend_state: 'outgoing_pending', request: existing }
      const target = ensureContact(did)
      target.friend_state = 'outgoing_pending'
      touch(target)
      const request: FriendRequest = {
        request_id: `fr-${++sequence}-${Date.now()}`,
        peer_did: did,
        direction: 'outgoing',
        state: 'pending',
        ...(message ? { message } : {}),
        delivery: did === 'did:bns:oscar' ? { state: 'pending', reason: 'zone-offline' } : { state: 'delivered' },
        created_at_ms: now(),
        expires_at_ms: now() + 7 * DAY,
        updated_at_ms: now(),
      }
      world.friendRequests.push(request)
      return { friend_state: 'outgoing_pending', request }
    }, 'contacts', 'requests'),
    respondFriendRequest: (requestId, decision) => write((): FriendRequestResult => {
      const request = world.friendRequests.find(item => item.request_id === requestId && item.state === 'pending' && item.direction === 'incoming')
      if (!request) throw new MockError('request-not-pending')
      if (decision === 'accept') {
        settle(request, 'accepted')
        becomeFriends(request.peer_did, request.peer_name)
        return { friend_state: 'friend', request }
      }
      settle(request, decision === 'reject' ? 'rejected' : 'ignored')
      const contact = world.contacts.get(request.peer_did)
      if (contact && contact.friend_state === 'incoming_pending') contact.friend_state = 'none'
      return { friend_state: 'none', request }
    }, 'contacts', 'requests'),
    cancelFriendRequest: (requestId) => write((): FriendRequestResult => {
      const request = world.friendRequests.find(item => item.request_id === requestId && item.state === 'pending' && item.direction === 'outgoing')
      if (!request) throw new MockError('request-not-pending')
      settle(request, 'cancelled')
      const contact = world.contacts.get(request.peer_did)
      if (contact) contact.friend_state = 'none'
      return { friend_state: 'none', request }
    }, 'contacts', 'requests'),
    removeFriend: (did) => write((): FriendRequestResult => {
      const contact = world.contacts.get(did)
      if (!contact || contact.friend_state !== 'friend') throw new MockError('not-found')
      if (isSystemRelation(contact)) throw new MockError('system-relation')
      contact.friend_state = 'none'
      if (contact.access_level !== 'block') contact.access_level = 'stranger'
      touch(contact)
      return { friend_state: 'none' }
    }, 'contacts'),
    listFriendRequests: () => read('friendRequests', () => world.friendRequests),
    listRequestSessions: () => read('requestSessions', () => world.requestSessions),
    listGroups: () => read('groups', () => [
      ...world.joined.values(),
      ...[...world.pendingJoins].flatMap((did): GroupRef[] => {
        const group = world.groups.get(did)
        return group ? [{ did, name: group.name, description: group.description, owner: group.owner, host: group.host, joinPolicy: group.joinPolicy, state: 'pending_admin_approval', hostedHere: true, canPost: false }] : []
      }),
      ...networkMockHosts().flatMap(host => host.groups()).map((group): GroupRef => ({
        did: group.did, name: group.name, description: group.description, owner: group.owner, host: MOCK_ZONE, joinPolicy: group.joinPolicy, memberCount: group.memberCount,
        role: group.myRole ?? 'member', state: 'active', hostedHere: true, canPost: true,
      })),
    ]),
    getGroupDoc: async (groupDid) => {
      await delay(0.6)
      const hosted = hostGroup(groupDid)?.group
      if (hosted) return { did: hosted.did, name: hosted.name, description: hosted.description, owner: hosted.owner, host: MOCK_ZONE, joinPolicy: hosted.joinPolicy }
      const group = world.groups.get(groupDid)
      if (!group || !group.hostedHere) throw new MockError('not-found')
      return { did: group.did, name: group.name, description: group.description, owner: group.owner, host: group.host, joinPolicy: group.joinPolicy }
    },
    requestJoin: async (groupDid, invite) => {
      const hosted = hostGroup(groupDid)
      if (!hosted) return joinOwn(groupDid, invite)
      await delay(0.8)
      const outcome = await hosted.join(invite)
      emit('groups')
      return outcome
    },
    leaveGroup: (groupDid) => write(() => {
      const ref = world.joined.get(groupDid)
      if (!ref) throw new MockError('not-found')
      if (ref.role === 'owner') throw new MockError('owner-must-transfer-first')
      world.joined.delete(groupDid)
    }, 'groups'),
    listGroupInvitations: () => read('invitations', () => world.invitations.filter(item => !item.recordId || world.notices.get(item.recordId) !== 'ARCHIVED')),
    acceptGroupInvitation: (invitation: GroupInvitationItem) => write((): JoinOutcome => {
      const group = world.groups.get(invitation.groupDid)
      if (!group) throw new MockError('not-found')
      world.invitations = world.invitations.filter(item => item.inviteId !== invitation.inviteId)
      return join(group, invitation.role)
    }, 'groups', 'mailbox'),
    dismissGroupInvitation: (invitation) => write(() => {
      if (invitation.recordId) world.notices.set(invitation.recordId, 'ARCHIVED')
    }, 'mailbox'),

    listJoinRequests: () => read('joinRequests', () => world.joinRequests),
    approveJoinRequest: (request: JoinRequestItem) => write(() => {
      world.joinRequests = world.joinRequests.filter(item => !(item.groupDid === request.groupDid && item.memberDid === request.memberDid))
      const ref = world.joined.get(request.groupDid)
      if (ref) ref.memberCount = (ref.memberCount ?? 0) + 1
    }, 'groups'),
    rejectJoinRequest: (request) => write(() => {
      world.joinRequests = world.joinRequests.filter(item => !(item.groupDid === request.groupDid && item.memberDid === request.memberDid))
    }, 'groups'),
    subscribe: (listener) => {
      listeners.add(listener)
      return () => { listeners.delete(listener) }
    },
  }
  return backend
}
