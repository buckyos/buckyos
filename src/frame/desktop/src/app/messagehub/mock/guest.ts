/**
 * Mock host group for the Session Only guest page (`guest/api.ts`): one
 * customer-service group with two ticket Sessions. The visitor is whoever
 * the mock control panel's session speaks for (`loadCurrentIdentity`);
 * reads follow the host rules the page depends on: only included guests
 * read a Session, a removed guest gets a structured denial, attachments are
 * served only with a matching `context_path`.
 *
 * The host state lives in localStorage under `MOCK_GUEST_HOST_STORAGE_KEY`
 * (a partial object is merged over the seed), so e2e tests can seed or
 * change it; messages live in memory for the page.
 */
import { RPCError, type CurrentIdentity } from 'buckyos'
import { loadCurrentIdentity } from '../../../sso/identity'
import { endSessionInPlace } from '../../../auth/signOut'
import { waitForMockLatency } from '../../../runtime'
import type { ObjectAccess } from '../conversation/history/objectAccess'
import { groupSessionKey } from '../groupModel'
import type { MsgObject } from '../protocol/msgobj'
import { ObjectRouteError } from '../api/objects'
import type { GroupSessionMemberRecord } from '../datamodel/sessionApi'
import type { GuestApi } from '../guest/api'
import { mockObjectAccess } from './objects'

export const MOCK_GUEST_HOST_STORAGE_KEY = 'buckyos.mock.messagehub.guest.v1'
export const MOCK_SUPPORT_GROUP = 'did:buckyos:group:acme-support'
const STAFF = 'did:bns:bob'

type Participation = 'invited' | 'included' | 'removed'

interface MockGuestHost {
  /** Session id → participant DID → state. */
  participants: Record<string, Record<string, Participation>>
  titles: Record<string, string>
  lifecycle: Record<string, 'active' | 'archived'>
  /** The group's public entry takes guest requests. */
  entry: boolean
}

interface StoredMessage { seq: number; objId: string; msg: MsgObject }

function seedHost(): MockGuestHost {
  return {
    participants: { 't-123': { 'did:bns:acme-c1': 'invited' }, 't-456': { 'did:bns:acme-c2': 'included' } },
    titles: { 't-123': 'Order #123', 't-456': 'Order #456' },
    lifecycle: { 't-123': 'active', 't-456': 'active' },
    entry: true,
  }
}

function loadHost(): MockGuestHost {
  const seed = seedHost()
  try {
    const raw = window.localStorage.getItem(MOCK_GUEST_HOST_STORAGE_KEY)
    if (raw) return { ...seed, ...(JSON.parse(raw) as Partial<MockGuestHost>) }
  } catch {
    // Storage unavailable: the seed applies.
  }
  return seed
}

function saveHost(host: MockGuestHost) {
  try { window.localStorage.setItem(MOCK_GUEST_HOST_STORAGE_KEY, JSON.stringify(host)) } catch { /* in memory only */ }
}

const now = Date.now()
const groupMessage = (sessionId: string, from: string, at: number, content: MsgObject['content']): MsgObject => ({ from, to: [MOCK_SUPPORT_GROUP], kind: 'group_msg', to_session: sessionId, created_at_ms: at, content })

const messages: Record<string, StoredMessage[]> = {
  't-123': [
    { seq: 1, objId: 'cymsg:mock-t123-1', msg: groupMessage('t-123', STAFF, now - 3_600_000, { format: 'text/plain', content: 'Hello, this is Acme support. We are looking into order #123.' }) },
    { seq: 2, objId: 'cymsg:mock-t123-2', msg: groupMessage('t-123', STAFF, now - 3_500_000, { format: 'text/plain', content: 'Here is the shipping note.', refs: [{ role: 'output', target: { type: 'data_obj', obj_id: 'cyfile:mock-doc-brief' }, label: 'shipping-note.pdf' }] }) },
  ],
  't-456': [
    { seq: 1, objId: 'cymsg:mock-t456-1', msg: groupMessage('t-456', STAFF, now - 7_200_000, { format: 'text/plain', content: 'Another customer’s ticket.' }) },
  ],
}
const accepted = new Map<string, string>()
const requests = new Map<string, string>()
const sends = new Map<string, string>()
let counter = 0

/** Objects the page asked for and the `context_path` it gave, for tests. */
export const mockGuestObjectRequests: Array<{ objId: string; context?: string }> = []

function denied(method: string) {
  return new RPCError('access denied: resource_grant_missing', 'access_denied', { reason: 'resource_grant_missing', adjust: { party: 'sharer', location: 'share_panel' }, service: 'msg-center', method })
}

async function caller(): Promise<string> {
  const identity: CurrentIdentity = await loadCurrentIdentity()
  if (identity.kind === 'anonymous') throw new RPCError('authentication-required')
  return identity.did
}

function requireGroup(groupDid: string) {
  if (groupDid !== MOCK_SUPPORT_GROUP) throw new RPCError('not-found')
}

/** Reads of a Session: an included guest only; a removed one is told so with a denial (D-4), anyone else finds nothing. */
function requireIncluded(host: MockGuestHost, sessionId: string, did: string, method: string) {
  const state = host.participants[sessionId]?.[did]
  if (state === 'included') return
  if (state === 'removed') throw denied(method)
  throw new RPCError('not-found')
}

const sessionOfAddress = (address: string) => Object.keys(loadHost().participants).find(id => groupSessionKey(MOCK_SUPPORT_GROUP, id) === address)

async function authorizeObject(objId: string, context: string | undefined) {
  mockGuestObjectRequests.push({ objId, ...(context ? { context } : {}) })
  const did = await caller()
  const slash = context?.lastIndexOf('/') ?? -1
  const sessionId = context && slash > 0 ? sessionOfAddress(context.slice(0, slash)) : undefined
  const msgId = context?.slice(slash + 1)
  const host = loadHost()
  const message = sessionId ? messages[sessionId]?.find(item => item.objId === msgId) : undefined
  const referenced = message?.msg.content.refs?.some(ref => ref.target.type === 'data_obj' && ref.target.obj_id === objId)
  if (!sessionId || host.participants[sessionId]?.[did] !== 'included' || !referenced) throw new Error('404 object unavailable')
}

const guestObjects: ObjectAccess = {
  async describe(objId, context) {
    await authorizeObject(objId, context)
    return mockObjectAccess.describe(objId)
  },
  async contentUrl(objId, context) {
    await authorizeObject(objId, context)
    return mockObjectAccess.contentUrl(objId)
  },
}

function members(sessionId: string, did: string): GroupSessionMemberRecord[] {
  const guests = Object.entries(loadHost().participants[sessionId] ?? {}).filter(([, state]) => state === 'included').map(([member]) => member)
  return [
    { member_did: STAFF, kind: 'group_member', role: 'admin', entity_kind: 'user', state: 'included' },
    ...guests.filter(member => member === did).map(member => ({ member_did: member, kind: 'guest' as const, role: null, entity_kind: null, state: 'included' as const })),
  ]
}

const api: GuestApi = {
  identity: loadCurrentIdentity,
  async groupName(groupDid) {
    await waitForMockLatency(40)
    requireGroup(groupDid)
    return 'Acme Support'
  },
  async entry(groupDid) {
    requireGroup(groupDid)
    return loadHost().entry ? { guest: false, password: true, self_register: true, did_login: true } : null
  },
  async sessions(groupDid) {
    await waitForMockLatency(40)
    requireGroup(groupDid)
    const did = await caller()
    const host = loadHost()
    const mine = Object.entries(host.participants).filter(([, people]) => people[did] === 'included')
    if (Object.values(host.participants).some(people => people[did] === 'removed') && mine.length === 0) throw denied('group.list_sessions')
    if (mine.length === 0) throw new RPCError('not-found')
    return mine.map(([sessionId]) => ({ sessionId, address: groupSessionKey(groupDid, sessionId), title: host.titles[sessionId] ?? '', lifecycle: host.lifecycle[sessionId] ?? 'active' }))
  },
  async invited(groupDid, sessionId) {
    requireGroup(groupDid)
    return loadHost().participants[sessionId]?.[await caller()] === 'invited'
  },
  async members(groupDid, sessionId) {
    requireGroup(groupDid)
    const did = await caller()
    requireIncluded(loadHost(), sessionId, did, 'group.list_session_members')
    return members(sessionId, did)
  },
  async messages(groupDid, sessionId, afterSeq, limit) {
    await waitForMockLatency(30)
    requireGroup(groupDid)
    requireIncluded(loadHost(), sessionId, await caller(), 'group.list_messages')
    const after = (messages[sessionId] ?? []).filter(item => item.seq > afterSeq)
    const page = after.slice(0, limit)
    return { items: page.map(item => ({ seq: item.seq, objId: item.objId })), nextAfterSeq: page.at(-1)?.seq ?? afterSeq, limited: after.length > limit }
  },
  async message(sessionAddress, objId) {
    const sessionId = sessionOfAddress(sessionAddress)
    const did = await caller()
    if (!sessionId || loadHost().participants[sessionId]?.[did] !== 'included') throw new ObjectRouteError(404, 'not-found')
    const stored = messages[sessionId]?.find(item => item.objId === objId)
    if (!stored) throw new ObjectRouteError(404, 'not-found')
    return structuredClone(stored.msg)
  },
  async accept(groupDid, sessionId, idempotencyKey) {
    await waitForMockLatency(60)
    requireGroup(groupDid)
    const did = await caller()
    if (accepted.get(idempotencyKey) === `${did}\n${sessionId}`) return
    const host = loadHost()
    if (host.participants[sessionId]?.[did] !== 'invited') throw new RPCError('not-found')
    host.participants[sessionId][did] = 'included'
    saveHost(host)
    accepted.set(idempotencyKey, `${did}\n${sessionId}`)
  },
  async request(groupDid, requestId) {
    await waitForMockLatency(60)
    requireGroup(groupDid)
    const did = await caller()
    const known = requests.get(`${did}\n${requestId}`)
    if (known) return known
    const host = loadHost()
    if (!host.entry) throw new RPCError('guest-entry-disabled')
    const sessionId = `g-${++counter}-${Math.random().toString(36).slice(2, 6)}`
    host.participants[sessionId] = { [did]: 'included' }
    host.titles[sessionId] = 'New conversation'
    host.lifecycle[sessionId] = 'active'
    saveHost(host)
    messages[sessionId] = []
    requests.set(`${did}\n${requestId}`, sessionId)
    return sessionId
  },
  async send(message, idempotencyKey) {
    await waitForMockLatency(60)
    const did = await caller()
    const sessionId = message.to_session ?? ''
    if (message.from !== did) return { ok: false, msgId: '', reason: 'sender-mismatch' }
    requireIncluded(loadHost(), sessionId, did, 'msg.post_send')
    const known = sends.get(idempotencyKey)
    if (known) return { ok: true, msgId: known }
    const list = messages[sessionId] ??= []
    const objId = `cymsg:mock-${++counter}`
    list.push({ seq: (list.at(-1)?.seq ?? 0) + 1, objId, msg: structuredClone(message) })
    sends.set(idempotencyKey, objId)
    return { ok: true, msgId: objId }
  },
  async markRead() {},
  objects: guestObjects,
  signOut: endSessionInPlace,
}

/** Test hooks: change the host while the page is open. */
const controls = {
  objectRequests: mockGuestObjectRequests,
  /** The host removes a guest from a Session (a revoked grant): their next read is refused. */
  remove(sessionId: string, did: string) {
    const host = loadHost()
    host.participants[sessionId] = { ...host.participants[sessionId], [did]: 'removed' }
    saveHost(host)
  },
  /** Someone of the group posts in a Session. */
  post(sessionId: string, content: string, from = STAFF) {
    const list = messages[sessionId] ??= []
    list.push({ seq: (list.at(-1)?.seq ?? 0) + 1, objId: `cymsg:mock-${++counter}`, msg: groupMessage(sessionId, from, Date.now(), { format: 'text/plain', content }) })
  },
  messages: (sessionId: string) => structuredClone(messages[sessionId] ?? []),
}

export function mockGuestApi(): GuestApi {
  if (import.meta.env.DEV && typeof window !== 'undefined') Object.assign(window, { __messageHubGuestMock: controls })
  return api
}
