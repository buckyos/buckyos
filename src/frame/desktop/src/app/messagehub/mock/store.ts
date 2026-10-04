import type { ComposerAttachmentInput } from '../conversation/input/attachmentDraft'
import type { z } from 'zod'
import { createCodeAssistantMockReaders } from '../../codeassistant/mockHistory'
import { InMemoryConversationMessageReader } from '../conversation/history/data-source'
import { registerObjectAccess } from '../conversation/history/objectAccess'
import type { ConversationMessageReader } from '../conversation/history/types'
import { getMessageStableId, type MessageObject, type MessageDeliveryStatus } from '../protocol/msgobj'
import { displayedContent, foldMessageRelations, messageObjId } from '../conversation/history/relations'
import { createSessionSchema, creationReason, defaultPreferences, groupSharedStateSchema, isMessageActivity, sharedStateSchema, memberStateSchema, presentationSchema, selectDefaultSession, sessionAccess, sessionKey, sessionTitle, sortSessions, viewerSessionKey } from '../sessionModel'
import { createGroupSchema, formatInviteLink, GROUP_INVITATION_INTENT, groupSessionId, groupSessionKey, participating, withinWindow } from '../groupModel'
import { ensureDefaultSession } from '../store/defaultSession'
import { noGroupCapabilities, type CreationPolicy, type Entity, type EntityDetail, type GroupInfo, type GroupInvitation, type GroupMemberState, type GroupRole, type GroupSessionInfo, type GroupSessionParticipant, type GroupSessionParticipants, type MessageHubContext, type ReadReceipt, type RuntimeState, type Session, type SessionAccess, type SessionBinding, type SessionPreferences } from '../types'
import { createOutgoingMockMessage, getMockEntityDid, MOCK_SELF_DID, mockEntities, mockEntityDetails, mockMessageReaders, mockMessageSeeds, mockSessions } from './data'
import { isHiddenAccount } from '../api/projection'
import { mockObjectAccess } from './objects'
import type { ConnectionChoice, EntityAdmission, MessageHubStore, OutgoingPayload, OwnerStatus } from '../store/types'

interface MockGroupSession {
  sessionId: string
  title: string
  description: string
  announcement: string
  lifecycle: 'active' | 'archived' | 'deleted'
  revision: string
  receipts: 'hidden' | 'count' | 'readers'
  /** Explicit participants beyond inherited members: group members added by hand and guests. */
  participants: Record<string, 'member' | 'guest' | 'invited_guest' | 'removed'>
}

interface MockGroup {
  did: string
  name: string
  description: string
  ownerDid: string
  lifecycle: GroupInfo['lifecycle']
  createdAt: number
  revision: string
  members: Record<string, { role: GroupRole; state: GroupMemberState; inviteId?: string; expiresAt?: number; invitedBy?: string; mutedUntil?: number; blocked?: boolean }>
  sessions: Record<string, MockGroupSession>
  inviteLinks: Record<string, { requireApproval: boolean; revoked: boolean }>
  pendingTransfer?: { memberDid: string; transferId: string; expiresAt: number }
}

type Snapshot = {
  groups: Record<string, MockGroup>
  sessions: Record<string, Session>
  deleted: Record<string, { at: number; session: Session }>
  withoutSeed: Record<string, boolean>
  /** Stable ids of the messages an owner deleted from their own view, by session key. */
  hidden: Record<string, string[]>
  messages: Record<string, MessageObject[]>
  delivery: Record<string, Record<string, MessageDeliveryStatus>>
  preferences: Record<string, SessionPreferences>
  policies: Record<string, CreationPolicy>
  drafts: Record<string, string>
  draftAttachments: Record<string, ComposerAttachmentInput[]>
}
const allEntities = (entities: Entity[]): Entity[] => entities.flatMap(entity => [entity, ...allEntities(entity.children ?? [])])
export const findEntity = (id: string) => allEntities(mockEntities).find(entity => entity.id === id)
export const MOCK_AGENT_OWNER = getMockEntityDid('agent-coder')
export const MOCK_PRODUCT_TEAM = getMockEntityDid('group-team')
export const MOCK_HIKING_GROUP = 'did:buckyos:group:weekend-hiking'
export const MOCK_BOOK_CLUB = 'did:buckyos:group:book-club'
const INVITE_TTL_MS = 7 * 86400_000
/** Mock edit / recall windows (the host's `EditRule`). */
const EDIT_WINDOW_MS = 24 * 3600_000
const RECALL_WINDOW_MS = 2 * 3600_000
export const defaultContext: MessageHubContext = { viewerDid: MOCK_SELF_DID, ownerDid: MOCK_SELF_DID, mode: 'self' }

function seedSnapshot(now: number): Snapshot {
  const sessions = Object.fromEntries(Object.values(mockSessions).flat().map(session => [sessionKey(session.ownerDid, session.id), structuredClone(session)]))
  const alice = getMockEntityDid('person-alice')
  for (const [id, name, remote] of [['telegram-personal', 'Telegram · Personal', 'general'], ['telegram-work', 'Telegram · Work', 'general'], ['telegram-work', 'Telegram · Work', 'design']]) {
    const session: Session = {
      id: `${id}-${remote}`, ownerDid: MOCK_SELF_DID, entityId: alice, title: remote === 'general' ? 'General' : 'Design', type: 'chat', source: 'telegram',
      binding: { kind: 'tunnel', tunnelInstanceId: id, endpointDid: `did:telegram:${id}:alice`, remoteContextId: remote, connectionName: name, supportsMultipleSessions: id === 'telegram-work', canCreateRemoteSession: id === 'telegram-work', canSend: true, connected: true },
      origin: 'connection', lifecycle: 'active', createdAt: now - 86400000, lastActiveAt: now - 4 * 3600000, unreadCount: 0,
      shared: { title: '', description: '', updatedAt: now - 86400000 }, members: { [MOCK_SELF_DID]: { nickname: 'Me', updatedAt: now }, [alice]: { nickname: 'Alice', updatedAt: now } },
    }
    sessions[sessionKey(session.ownerDid, session.id)] = session
  }
  for (const ownerDid of [MOCK_AGENT_OWNER, 'did:bns:assistant.alice']) {
    const original = sessions[sessionKey(MOCK_SELF_DID, 'session-coder-1')]
    const observed = { ...structuredClone(original), ownerDid, entityId: alice, title: 'Agent inbox', shared: { title: 'Agent inbox', description: '', updatedAt: now }, members: { [ownerDid]: { nickname: 'Agent', updatedAt: now }, [alice]: { nickname: 'Alice', updatedAt: now } }, binding: { kind: 'native' as const, targetDid: alice }, unreadCount: 7 }
    sessions[sessionKey(ownerDid, observed.id)] = observed
  }
  const bob = getMockEntityDid('person-bob'), carol = getMockEntityDid('person-carol'), dave = getMockEntityDid('person-dave')
  const erin = 'did:buckyos:person:erin'
  const groups: Record<string, MockGroup> = {
    [MOCK_PRODUCT_TEAM]: { did: MOCK_PRODUCT_TEAM, name: 'Product Team', description: 'Everything about the product.', ownerDid: alice, lifecycle: 'active', createdAt: now - 90 * 86400000, revision: 'rev-team-1', members: {
      [alice]: { role: 'owner', state: 'active' }, [MOCK_SELF_DID]: { role: 'admin', state: 'active' }, [bob]: { role: 'member', state: 'active' },
      [dave]: { role: 'member', state: 'active' }, [MOCK_AGENT_OWNER]: { role: 'member', state: 'active' },
      [getMockEntityDid('agent-writer')]: { role: 'member', state: 'invited', inviteId: 'invite-writer', expiresAt: now + INVITE_TTL_MS },
      [erin]: { role: 'member', state: 'pending_admin_approval', invitedBy: bob },
    }, sessions: {
      'design-review': { sessionId: 'design-review', title: 'Design review', description: '', announcement: 'Review every Thursday.', lifecycle: 'active', revision: 'rev-design-1', receipts: 'readers', participants: { 'did:buckyos:person:frank': 'guest' } },
    }, inviteLinks: {}, pendingTransfer: { memberDid: MOCK_SELF_DID, transferId: 'transfer-team', expiresAt: now + INVITE_TTL_MS } },
    [MOCK_HIKING_GROUP]: { did: MOCK_HIKING_GROUP, name: 'Weekend Hiking', description: '', ownerDid: bob, lifecycle: 'active', createdAt: now - 86400000, revision: 'rev-hiking-1', members: {
      [bob]: { role: 'owner', state: 'active' }, [alice]: { role: 'member', state: 'active' },
      [MOCK_SELF_DID]: { role: 'member', state: 'invited', inviteId: 'invite-hiking', expiresAt: now + INVITE_TTL_MS },
    }, sessions: {}, inviteLinks: {} },
    [MOCK_BOOK_CLUB]: { did: MOCK_BOOK_CLUB, name: 'Book Club', description: 'One book a month.', ownerDid: carol, lifecycle: 'active', createdAt: now - 10 * 86400000, revision: 'rev-book-1', members: {
      [carol]: { role: 'owner', state: 'active' }, [dave]: { role: 'member', state: 'active' },
      [MOCK_AGENT_OWNER]: { role: 'member', state: 'invited', inviteId: 'invite-book-agent', expiresAt: now + INVITE_TTL_MS, invitedBy: carol },
    }, sessions: {
      reading: { sessionId: 'reading', title: 'Reading list', description: '', announcement: '', lifecycle: 'active', revision: 'rev-reading-1', receipts: 'count', participants: { [MOCK_SELF_DID]: 'invited_guest' } },
    }, inviteLinks: { 'book-club-token': { requireApproval: false, revoked: false } } },
  }
  const notice = (id: string, from: string, sessionId: string, at: number, groupDid: string, action: string, data: Record<string, unknown>): MessageObject => ({
    from, to: [MOCK_SELF_DID], kind: 'operation', created_at_ms: at, ui_message_id: id, ui_session_id: sessionId,
    content: { format: 'text/plain', content: action, machine: { intent: GROUP_INVITATION_INTENT, data: { group_did: groupDid, action, data: data as Record<string, never> } } },
  })
  sessions[sessionKey(MOCK_SELF_DID, 'session-carol-1')] = {
    id: 'session-carol-1', ownerDid: MOCK_SELF_DID, entityId: carol, title: 'Telegram', type: 'chat', source: 'telegram', binding: { kind: 'tunnel', tunnelInstanceId: 'telegram-personal', endpointDid: 'did:telegram:telegram-personal:carol', connectionName: 'Telegram · Personal', supportsMultipleSessions: false, canCreateRemoteSession: false, canSend: true, connected: true }, origin: 'connection', lifecycle: 'active',
    createdAt: now - 2 * 86400000, lastActiveAt: now - 40 * 60000, unreadCount: 0, shared: { title: '', description: '', updatedAt: now - 2 * 86400000 }, members: {},
  }
  sessions[sessionKey(MOCK_SELF_DID, groupSessionKey(MOCK_PRODUCT_TEAM, 'design-review'))] = {
    id: groupSessionKey(MOCK_PRODUCT_TEAM, 'design-review'), ownerDid: MOCK_SELF_DID, entityId: MOCK_PRODUCT_TEAM, title: 'Design review', type: 'chat', source: 'buckyos', binding: { kind: 'native', targetDid: MOCK_PRODUCT_TEAM }, origin: 'remote_context', lifecycle: 'active',
    createdAt: now - 5 * 86400000, lastActiveAt: now - 3 * 3600000, unreadCount: 0, shared: { title: 'Design review', description: '', updatedAt: now - 5 * 86400000 }, members: {},
  }
  const messages: Record<string, MessageObject[]> = {
    [sessionKey(MOCK_SELF_DID, 'session-bob-1')]: [notice('msg-invite-hiking', bob, 'session-bob-1', now - 50 * 60000, MOCK_HIKING_GROUP, 'invite', { invite_id: 'invite-hiking', role: 'member', expires_at_ms: now + INVITE_TTL_MS, state: 'invited' })],
    [sessionKey(MOCK_SELF_DID, 'session-alice-1')]: [
      notice('msg-invite-team-active', alice, 'session-alice-1', now - 12 * 60000, MOCK_PRODUCT_TEAM, 'invite', { invite_id: 'invite-team-self', role: 'admin', expires_at_ms: now + INVITE_TTL_MS, state: 'active' }),
      notice('msg-pending-erin', alice, 'session-alice-1', now - 11 * 60000, MOCK_PRODUCT_TEAM, 'pending_approval', { member_did: erin, invited_by: bob }),
      notice('msg-transfer-team', alice, 'session-alice-1', now - 10 * 60000, MOCK_PRODUCT_TEAM, 'owner_transfer', { transfer_id: 'transfer-team', expires_at_ms: now + INVITE_TTL_MS }),
    ],
    [sessionKey(MOCK_SELF_DID, 'session-carol-1')]: [
      notice('msg-invite-book-agent', carol, 'session-carol-1', now - 42 * 60000, MOCK_BOOK_CLUB, 'invite', { invite_id: 'invite-book-agent', role: 'member', expires_at_ms: now + INVITE_TTL_MS, state: 'invited', member_did: MOCK_AGENT_OWNER }),
      notice('msg-session-invite-reading', carol, 'session-carol-1', now - 40 * 60000, MOCK_BOOK_CLUB, 'session_invite', { session_id: 'reading', title: 'Reading list' }),
    ],
  }
  return { groups, sessions, deleted: {}, withoutSeed: {}, hidden: {}, messages, delivery: {}, preferences: {}, policies: {}, drafts: {}, draftAttachments: {} }
}

function groupEvent(group: MockGroup, action: string, actorDid: string, at: number, subjectDid?: string): MessageObject {
  const eventId = crypto.randomUUID()
  return { kind: 'event', from: group.did, to: [group.did], created_at_ms: at, ui_message_id: eventId, ui_session_id: group.did, content: { format: 'text/plain', content: action, machine: { intent: 'buckyos.action_log', data: { schema_version: 1, event_id: eventId, action, actor_did: actorDid, ...(subjectDid ? { subject_did: subjectDid } : {}), target: { kind: 'entity', entity_did: group.did }, occurred_at_ms: at, changes: [], source: { kind: 'native', producer_did: group.did } } } } }
}

function openDatabase(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open('messagehub-prototype-v1', 1)
    request.onupgradeneeded = () => request.result.createObjectStore('state')
    request.onsuccess = () => resolve(request.result)
    request.onerror = () => reject(request.error)
  })
}

export class MessageHubMockStore implements MessageHubStore {
  readonly isMock = true
  private snapshot = seedSnapshot(Date.now())
  private listeners = new Set<() => void>()
  private readers = new Map<string, ConversationMessageReader>()
  private seedIds = new Map<string, Set<string>>()
  private seeds: Record<string, ConversationMessageReader> = mockMessageReaders
  private db?: IDBDatabase
  private queue = Promise.resolve()
  private initializePromise?: Promise<void>
  private runtime = new Map<string, RuntimeState[]>()
  private runtimeListeners = new Set<() => void>()
  private timeListeners = new Set<() => void>()
  private clockOverride?: number
  private clockValue = Date.now()
  private runtimeVersion = 0
  private delayMs = 0
  private failure?: string
  private revokedOwners = new Set<string>()
  /** `groupSessionMembers` results per snapshot, so components see stable values between changes. */
  private sessionMemberLists = new WeakMap<Snapshot, Map<string, GroupSessionParticipants | null>>()
  constructor() {
    if (import.meta.env.DEV && typeof window !== 'undefined') Object.assign(window, { __messageHubMock: this })
  }
  defaultContext() { return defaultContext }
  ownerStatus(context: MessageHubContext): OwnerStatus { return this.canView(context) ? { phase: 'ready' } : { phase: 'denied' } }
  ensureOwner() { return this.initialize() }
  startSync() { return () => {} }
  findEntity(_context: MessageHubContext, id: string) { return this.lookup(id) }
  /** Seeded entities, plus self-host groups created or joined in this mock. */
  private lookup(id: string): Entity | undefined {
    const seeded = findEntity(id)
    const group = this.snapshot.groups[id]
    if (seeded) return group ? { ...seeded, name: group.name } : seeded
    if (!group || (group.members[MOCK_SELF_DID]?.state !== 'active' && !Object.values(this.snapshot.sessions).some(session => session.entityId === id))) return undefined
    return { id, type: 'group', name: group.name, tags: ['group'], unreadCount: 0, lastActiveAt: group.createdAt, source: 'buckyos', sources: ['buckyos'], domain: 'managed' }
  }
  hasMoreEntities() { return false }
  async loadMoreEntities() {}
  entityDetail(context: MessageHubContext, id: string): EntityDetail | null {
    const entity = this.entities(context).flatMap(item => [item, ...(item.children ?? [])]).find(item => item.id === id)
    return entity ? { ...mockEntityDetails[id], ...entity } : null
  }
  admission(): EntityAdmission | null { return null }
  async setAdmission(): Promise<void> { throw Error('backend_unavailable') }
  historyStatus(): 'ready' { return 'ready' }
  hasOlder() { return false }
  async loadOlder() { return false }
  async markRead() {}
  access(context: MessageHubContext, session: Session, confirmed: boolean): SessionAccess {
    const access = sessionAccess(context, session, confirmed)
    const group = this.snapshot.groups[session.entityId]
    if (!group || access.mode !== 'read_write') return access
    const sid = groupSessionId(group.did, session.id)
    const member = group.members[context.ownerDid]?.state === 'active'
    const guest = sid !== undefined && group.sessions[sid]?.participants[context.ownerDid] === 'guest'
    const info = this.groupInfo(group, context.ownerDid)
    const reason = group.lifecycle === 'archived' ? 'group_archived' : group.lifecycle !== 'active' || !(member || guest) ? 'group_not_member'
      : sid !== undefined && group.sessions[sid]?.lifecycle !== 'active' ? 'group_archived'
      : (group.members[context.ownerDid]?.mutedUntil ?? 0) > this.now() ? 'group_post_denied' : undefined
    return reason ? { ...access, mode: 'read_only', canEditSharedState: false, canEditOwnMemberState: false, readOnlyReason: reason } : { ...access, canEditSharedState: info.can.updateSharedState, canEditOwnMemberState: member }
  }
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener) } }
  getSnapshot = () => this.snapshot
  subscribeTime = (listener: () => void) => { this.timeListeners.add(listener); return () => { this.timeListeners.delete(listener) } }
  getTime = () => this.clockValue
  subscribeRuntime = (listener: () => void) => { this.runtimeListeners.add(listener); return () => { this.runtimeListeners.delete(listener) } }
  getRuntimeVersion = () => this.runtimeVersion
  now = () => this.clockOverride ?? Date.now()
  tick = () => {
    const now = this.now()
    if (Math.floor(now / 60_000) !== Math.floor(this.clockValue / 60_000)) { this.clockValue = now; this.timeListeners.forEach(listener => listener()) }
    if ([...this.runtime.values()].some(states => states.some(state => state.expiresAt <= this.now()))) {
      for (const [key, states] of this.runtime) this.runtime.set(key, states.filter(state => state.expiresAt > this.now()))
      this.emitRuntime()
    }
  }
  private emitRuntime() { this.runtimeVersion++; this.runtimeListeners.forEach(listener => listener()) }
  initialize = () => {
    this.initializePromise ??= this.load().catch(error => { this.initializePromise = undefined; throw error })
    return this.initializePromise
  }
  private async load() {
    registerObjectAccess(mockObjectAccess)
    this.db = await openDatabase()
    const stored = await new Promise<Snapshot | undefined>((resolve, reject) => {
      const request = this.db!.transaction('state').objectStore('state').get('snapshot')
      request.onsuccess = () => resolve(request.result)
      request.onerror = () => reject(request.error)
    })
    this.seeds = { ...mockMessageReaders, ...await createCodeAssistantMockReaders() }
    for (const [id, reader] of Object.entries(this.seeds)) {
      const ids = new Set<string>()
      const session = this.snapshot.sessions[sessionKey(MOCK_SELF_DID, id)]
      for (let start = 0; start < reader.totalCount; start += 128) {
        const messages = await reader.readRange(start, 128)
        messages.forEach((message, offset) => {
          ids.add(getMessageStableId(message, start + offset))
          if (!stored && session && isMessageActivity(message) && (!session.lastMessage || message.created_at_ms >= session.lastMessage.timestamp)) session.lastMessage = { text: message.content.content ?? '', timestamp: message.created_at_ms }
        })
      }
      this.seedIds.set(id, ids)
    }
    if (stored && (!stored.groups || Object.values(stored.groups).some(group => !group.sessions))) {
      const seeded = seedSnapshot(this.now())
      stored.groups = seeded.groups
      Object.assign(stored.messages, seeded.messages, stored.messages)
      Object.assign(stored.sessions, seeded.sessions, stored.sessions)
    }
    if (stored) this.snapshot = { ...stored, hidden: stored.hidden ?? {} }
    else await this.persist(this.snapshot)
    this.listeners.forEach(listener => listener())
  }
  private persist(snapshot: Snapshot) {
    if (!this.db) return Promise.resolve()
    return new Promise<void>((resolve, reject) => {
      const transaction = this.db!.transaction('state', 'readwrite')
      transaction.objectStore('state').put(snapshot, 'snapshot')
      transaction.oncomplete = () => resolve()
      transaction.onabort = () => reject(transaction.error ?? Error('storage_failed'))
      transaction.onerror = () => reject(transaction.error ?? Error('storage_failed'))
    })
  }
  private mutate<T>(operation: (next: Snapshot) => T, controlled = true): Promise<T> {
    const result = this.queue.then(async () => {
      if (controlled && this.delayMs) await new Promise(resolve => setTimeout(resolve, this.delayMs))
      if (controlled && this.failure) { const error = this.failure; this.failure = undefined; throw Error(error) }
      const next = structuredClone(this.snapshot)
      const value = operation(next)
      await this.persist(next)
      const old = this.snapshot
      this.snapshot = next
      for (const key of this.readers.keys()) {
        const [viewer, owner, sessionId] = JSON.parse(key) as string[]
        void viewer
        const ref = sessionKey(owner, sessionId)
        if (!next.sessions[ref] || JSON.stringify(next.messages[ref]) !== JSON.stringify(old.messages[ref]) || next.withoutSeed[ref] !== old.withoutSeed[ref] || (next.hidden[ref]?.length ?? 0) !== (old.hidden[ref]?.length ?? 0) || next.deleted[ref]?.at !== old.deleted[ref]?.at || JSON.stringify(next.delivery[ref]) !== JSON.stringify(old.delivery[ref])) this.readers.delete(key)
      }
      this.listeners.forEach(listener => listener())
      return value
    })
    this.queue = result.then(() => undefined, () => undefined)
    return result
  }
  canView(context: MessageHubContext) {
    return context.viewerDid === MOCK_SELF_DID && !this.revokedOwners.has(context.ownerDid) && ((context.mode === 'self' && context.ownerDid === MOCK_SELF_DID) || (context.mode === 'observe' && [MOCK_AGENT_OWNER, 'did:bns:assistant.alice'].includes(context.ownerDid)))
  }
  private requireOwn(context: MessageHubContext) {
    if (!this.canView(context) || context.mode !== 'self' || context.ownerDid !== context.viewerDid) throw Error('agent_observer')
  }
  private requireSession(next: Snapshot, context: MessageHubContext, id: string) {
    if (!this.canView(context)) throw Error('permission_denied')
    const session = next.sessions[sessionKey(context.ownerDid, id)]
    if (!session) throw Error('session_missing')
    return session
  }
  preferences(context: MessageHubContext, id: string) { return this.snapshot.preferences[viewerSessionKey(context, id)] ?? defaultPreferences }
  policy(context: MessageHubContext, entityId: string) { return this.snapshot.policies[sessionKey(context.ownerDid, entityId)] ?? 'default' }
  sessions(context: MessageHubContext, entityId?: string, lifecycle?: Session['lifecycle']) {
    if (!this.canView(context)) return []
    return sortSessions(Object.values(this.snapshot.sessions).filter(session => session.ownerDid === context.ownerDid && (!entityId || session.entityId === entityId) && (!lifecycle || session.lifecycle === lifecycle)), id => this.preferences(context, id))
  }
  entities(context: MessageHubContext): Entity[] {
    const project = (entity: Entity): Entity => {
      const sessions = this.sessions(context, entity.id)
      const latest = [...sessions].sort((a, b) => b.lastActiveAt - a.lastActiveAt || a.id.localeCompare(b.id))[0]
      const policy = this.policy(context, entity.id)
      const choices = this.connections(context, entity.id)
      const reason = choices.some(choice => !creationReason(context, entity, policy, choice.binding)) ? undefined : creationReason(context, entity, policy, choices[0]?.binding)
      return { ...entity, unreadCount: sessions.reduce((sum, session) => sum + session.unreadCount, 0), lastActiveAt: latest?.lastActiveAt ?? 0, lastMessage: latest?.lastMessage, sessionCreation: { policy, canCreate: !reason, unavailableReason: reason }, children: entity.children?.map(project) }
    }
    const dynamic = Object.keys(this.snapshot.groups).filter(id => !findEntity(id)).map(id => this.lookup(id)).filter((entity): entity is Entity => Boolean(entity))
    return [...mockEntities.map(entity => this.lookup(entity.id) ?? entity), ...dynamic].filter(entity => !isHiddenAccount({ did: entity.id, name: entity.name, tags: entity.tags }, context.ownerDid, { dids: [MOCK_SELF_DID] })).map(project).sort((a, b) => Number(!!b.isPinned) - Number(!!a.isPinned) || b.lastActiveAt - a.lastActiveAt || a.id.localeCompare(b.id))
  }
  defaultSession(context: MessageHubContext, entityId: string) {
    const entity = this.findEntity(context, entityId)
    return entity ? selectDefaultSession(entity, this.sessions(context, entityId, 'active')) : null
  }
  ensureDefaultSession(context: MessageHubContext, entityId: string) { return ensureDefaultSession(this, context, entityId) }
  connections(context: MessageHubContext, entityId: string): ConnectionChoice[] {
    const entity = this.lookup(entityId)
    const choices = new Map<string, ConnectionChoice>()
    const known = [...Object.values(this.snapshot.sessions), ...Object.values(this.snapshot.deleted).map(deleted => deleted.session)].filter(session => session.ownerDid === context.ownerDid && session.entityId === entityId)
    if (entity && (entity.type === 'agent' || entity.source === 'buckyos' || entity.domain === 'managed' || known.some(session => session.binding.kind === 'native'))) choices.set('native', { id: 'native', binding: { kind: 'native', targetDid: entityId }, label: 'BuckyOS' })
    for (const session of known) {
      if (session.binding.kind === 'tunnel') choices.set(session.binding.tunnelInstanceId, { id: session.binding.tunnelInstanceId, binding: session.binding, label: session.binding.connectionName })
    }
    return [...choices.values()]
  }
  reader(context: MessageHubContext, id: string): ConversationMessageReader {
    if (!this.canView(context)) return InMemoryConversationMessageReader.empty()
    const cacheKey = viewerSessionKey(context, id)
    const cached = this.readers.get(cacheKey)
    if (cached) return cached
    const key = sessionKey(context.ownerDid, id)
    const withoutSeed = this.snapshot.withoutSeed[key]
    const deletedAt = this.snapshot.deleted[key]?.at
    const delivery = this.snapshot.delivery[key] ?? {}
    const exists = this.snapshot.sessions[key]
    const base = exists && context.ownerDid === MOCK_SELF_DID && !this.snapshot.withoutSeed[key] ? this.seeds[id] : undefined
    const raw = this.snapshot.messages[key] ?? []
    // Relations in the appended messages may target seeded ones (a reaction to
    // Bob's seeded message), so seeds from `mockMessageSeeds` fold with them.
    const seedMessages = base && base === mockMessageReaders[id] ? mockMessageSeeds[id] : undefined
    const folded = foldMessageRelations([...(seedMessages ?? []), ...raw])
    const hidden = new Set(this.snapshot.hidden[key] ?? [])
    const visible = (rows: MessageObject[]) => hidden.size ? rows.filter(message => !hidden.has(getMessageStableId(message, 0))) : rows
    const seededRows = seedMessages ? visible(folded.slice(0, seedMessages.length)) : undefined
    const delta = visible(seedMessages ? folded.slice(seedMessages.length) : folded)
    const baseCount = seededRows?.length ?? base?.totalCount ?? 0
    // Folded relations change rows without changing the row count, so the
    // raw message count serves as the reader revision the history pane watches.
    const reader: ConversationMessageReader & { revision: number } = {
      readerKey: `mock:${cacheKey}:${this.snapshot.withoutSeed[key] ? `fresh:${deletedAt}` : 'seed'}:${JSON.stringify(delivery)}`,
      revision: raw.length + hidden.size,
      totalCount: exists ? baseCount + delta.length : 0,
      readRange: async (start, count) => {
        if (!this.canView(context) || !this.snapshot.sessions[key] || this.snapshot.withoutSeed[key] !== withoutSeed || this.snapshot.deleted[key]?.at !== deletedAt) return []
        const from = Math.max(0, start)
        const seeded = base && from < baseCount ? (seededRows ? seededRows.slice(from, from + Math.min(count, baseCount - from)) : await base.readRange(from, Math.min(count, baseCount - from))) : []
        return [...seeded, ...delta.slice(Math.max(0, from - baseCount), Math.max(0, from + count - baseCount))].map((message, offset) => { const status = delivery[getMessageStableId(message, from + offset)]; return status ? { ...message, ui_delivery_status: status } : message })
      },
    }
    this.readers.set(cacheKey, reader)
    return reader
  }
  draft(context: MessageHubContext, id: string) { return context.mode === 'self' ? this.snapshot.drafts[viewerSessionKey(context, id)] ?? '' : '' }
  attachments(context: MessageHubContext, id: string) { return context.mode === 'self' ? this.snapshot.draftAttachments[viewerSessionKey(context, id)] ?? [] : [] }
  saveAttachments(context: MessageHubContext, id: string, attachments: ComposerAttachmentInput[]) {
    return this.mutate(next => { this.requireOwn(context); this.requireSession(next, context, id); next.draftAttachments[viewerSessionKey(context, id)] = attachments }, false)
  }
  saveDraft(context: MessageHubContext, id: string, value: string) {
    return this.mutate(next => { this.requireOwn(context); this.requireSession(next, context, id); next.drafts[viewerSessionKey(context, id)] = value }, false)
  }
  setPolicy(context: MessageHubContext, entityId: string, policy: CreationPolicy) {
    return this.mutate(next => { this.requireOwn(context); if (!this.lookup(entityId)) throw Error('binding_unknown'); next.policies[sessionKey(context.ownerDid, entityId)] = policy })
  }
  create(context: MessageHubContext, input: z.infer<typeof createSessionSchema>) {
    return this.mutate(next => {
      this.requireOwn(context)
      const values = createSessionSchema.parse(input)
      const entity = this.lookup(values.entityId)
      if (!entity) throw Error('binding_unknown')
      const group = next.groups[entity.id]
      if (group && !this.groupInfo(group, context.ownerDid).can.createSession) throw Error('rejected: capability-denied')
      const binding = this.connections(context, entity.id).find(choice => choice.id === values.connection)?.binding
      const reason = creationReason(context, entity, this.policy(context, entity.id), binding)
      if (reason || !binding) throw Error(reason)
      const now = this.now(), id = crypto.randomUUID()
      const session: Session = { id, ownerDid: context.ownerDid, entityId: entity.id, title: entity.name, type: 'chat', binding: binding.kind === 'tunnel' ? { ...binding, remoteContextId: id } : binding, source: binding.kind === 'tunnel' ? binding.connectionName.split(' · ')[0].toLowerCase() : 'buckyos', origin: 'manual', lifecycle: 'active', createdAt: now, lastActiveAt: now, unreadCount: 0, shared: { title: values.title, description: '', updatedAt: now }, members: { [context.ownerDid]: { nickname: 'Me', updatedAt: now }, [entity.id]: { nickname: entity.name, updatedAt: now } } }
      next.sessions[sessionKey(context.ownerDid, id)] = session
      return session
    })
  }
  manage(context: MessageHubContext, id: string, action: 'archive' | 'restore' | 'delete') {
    return this.mutate(next => {
      this.requireOwn(context)
      const session = this.requireSession(next, context, id), key = sessionKey(context.ownerDid, id)
      if (action !== 'delete') { session.lifecycle = action === 'archive' ? 'archived' : 'active'; return }
      next.deleted[key] = { at: this.now(), session: { ...session, title: this.lookup(session.entityId)?.name ?? '', shared: { title: '', description: '', updatedAt: this.now() }, members: {}, lastMessage: undefined, unreadCount: 0 } }
      next.withoutSeed[key] = true
      delete next.sessions[key]; delete next.messages[key]; delete next.delivery[key]
      for (const collection of [next.preferences, next.drafts, next.draftAttachments]) for (const prefKey of Object.keys(collection)) {
        const [, owner, sessionId] = JSON.parse(prefKey) as string[]
        if (owner === context.ownerDid && sessionId === id) delete collection[prefKey]
      }
      this.runtime.delete(key)
    })
  }
  updatePreferences(context: MessageHubContext, id: string, patch: Partial<SessionPreferences>) {
    return this.mutate(next => {
      this.requireSession(next, context, id)
      if (Object.keys(patch).some(key => !['title', 'pinned', 'muted', 'showActions', 'pinnedMessage'].includes(key))) throw Error('permission_denied')
      if (Object.keys(patch).some(key => key !== 'showActions')) this.requireOwn(context)
      const key = viewerSessionKey(context, id)
      const preferences = { ...defaultPreferences, ...next.preferences[key], ...patch }
      presentationSchema.parse(preferences)
      if (typeof preferences.showActions !== 'boolean') throw Error('invalid_input')
      next.preferences[key] = preferences
    })
  }
  updateState(context: MessageHubContext, id: string, scope: 'shared' | 'member', input: unknown) {
    return this.mutate(next => {
      this.requireOwn(context)
      const session = this.requireSession(next, context, id)
      if (session.binding.kind !== 'native') throw Error('platform_read_only')
      const values = scope === 'shared' ? groupSharedStateSchema.strict().parse(input) : memberStateSchema.strict().parse(input)
      const group = next.groups[session.entityId]
      const groupSession = group ? group.sessions[groupSessionId(group.did, id) ?? ''] : undefined
      if (group && scope === 'shared') {
        if (!this.groupInfo(group, context.ownerDid).can.updateSharedState) throw Error('rejected: state-write-denied')
        const shared = values as { title: string; description: string; announcement?: string }
        if (groupSession) Object.assign(groupSession, { title: shared.title, description: shared.description, announcement: shared.announcement ?? groupSession.announcement, revision: crypto.randomUUID() })
      }
      const previous = scope === 'shared' ? session.shared : session.members[context.ownerDid]
      const changes = Object.entries(values).filter(([field, value]) => (previous as unknown as Record<string, unknown>)?.[field] !== value).map(([field, after]) => ({ field, before: (previous as unknown as Record<string, string>)?.[field] ?? '', after }))
      if (!changes.length) return
      if (scope === 'shared') session.shared = { ...session.shared, ...values, updatedAt: this.now() }
      else session.members[context.ownerDid] = { ...session.members[context.ownerDid], ...values, updatedAt: this.now() }
      const action = scope === 'member' ? 'session.member_state_changed' : changes.length === 1 && changes[0].field === 'title' ? 'session.title_changed' : 'session.shared_state_changed'
      const eventId = crypto.randomUUID()
      this.append(next, session, { kind: 'event', from: context.ownerDid, to: [session.entityId], created_at_ms: this.now(), ui_message_id: eventId, ui_session_id: id, content: { content: action, machine: { intent: 'buckyos.action_log', data: { schema_version: 1, event_id: eventId, action, actor_did: context.viewerDid, ...(scope === 'member' ? { subject_did: context.ownerDid } : {}), target: { kind: 'session', session_id: id, owner_did: context.ownerDid }, occurred_at_ms: this.now(), changes, source: { kind: 'native', producer_did: context.ownerDid } } } } }, false)
    })
  }
  private append(next: Snapshot, session: Session, message: MessageObject, incoming: boolean) {
    const key = sessionKey(session.ownerDid, session.id)
    const messages = next.messages[key] ?? []
    if (session.ownerDid === MOCK_SELF_DID && !next.withoutSeed[key] && this.seedIds.get(session.id)?.has(getMessageStableId(message, 0))) return
    if (messages.some(item => getMessageStableId(item, 0) === getMessageStableId(message, 0))) return
    next.messages[key] = [...messages, message]
    if (!isMessageActivity(message)) return
    if (message.created_at_ms >= session.lastActiveAt) session.lastMessage = { text: message.content.content ?? '', timestamp: message.created_at_ms }
    session.lastActiveAt = Math.max(session.lastActiveAt, message.created_at_ms)
    session.lifecycle = 'active'
    if (incoming) session.unreadCount++
  }
  send(context: MessageHubContext, id: string, payload: OutgoingPayload, confirmation: string | undefined) {
    const session = this.snapshot.sessions[sessionKey(context.ownerDid, id)]
    const message = createOutgoingMockMessage({ sessionId: id, entityId: session?.entityId ?? '', content: buildOutgoingDraftContent(payload), createdAtMs: this.now() })
    if (payload.relatesTo) {
      const { rel, target: targetId, key } = payload.relatesTo
      const group = session ? this.snapshot.groups[session.entityId] : undefined
      const timeline = this.timeline(context.ownerDid, id)
      const target = timeline.find(item => messageObjId(item) === targetId)
      if (!target) throw Error('rejected: relation-target-not-found')
      const own = target.from === context.ownerDid
      // Edits and group redacts follow the group rules (`Self-Host-Groupv2.md` §2.6);
      // reactions, replies and delete requests outside a group work in any session.
      if (rel === 'edit' && (!group || !own || !withinWindow(EDIT_WINDOW_MS, target.created_at_ms, this.now()))) throw Error('rejected: edit-window-expired')
      if (rel === 'redact' && group && !(own ? withinWindow(RECALL_WINDOW_MS, target.created_at_ms, this.now()) : this.groupInfo(group, context.ownerDid).can.redactAny)) throw Error('rejected: capability-denied')
      if (rel === 'reaction') {
        // The same (from, target, key) counts once: the host answers a repeat with the existing message.
        const redacted = new Set(timeline.filter(item => item.relates_to?.rel === 'redact').map(item => item.relates_to!.target))
        if (timeline.some(item => item.from === context.ownerDid && item.relates_to?.rel === 'reaction' && item.relates_to.target === targetId && item.relates_to.key === key && !redacted.has(messageObjId(item) ?? ''))) return Promise.resolve()
      }
      message.relates_to = payload.relatesTo
    }
    if (payload.mentions && (payload.mentions.all || payload.mentions.dids?.length)) {
      const group = session ? this.snapshot.groups[session.entityId] : undefined
      if (payload.mentions.all && !(group && this.groupInfo(group, context.ownerDid).can.mentionAll)) throw Error('rejected: capability-denied')
      message.mentions = payload.mentions
    }
    return this.sendMessage(context, id, message, confirmation)
  }
  /** Seeded plus appended raw messages of a session (relation targets may be seeded). */
  private timeline(ownerDid: string, id: string): MessageObject[] {
    const key = sessionKey(ownerDid, id)
    const seeded = ownerDid === MOCK_SELF_DID && !this.snapshot.withoutSeed[key] && this.seeds[id] === mockMessageReaders[id] ? mockMessageSeeds[id] ?? [] : []
    return [...seeded, ...(this.snapshot.messages[key] ?? [])]
  }
  resend(context: MessageHubContext, id: string, message: MessageObject, confirmation: string | undefined) {
    const session = this.snapshot.sessions[sessionKey(context.ownerDid, id)]
    const outgoing = createOutgoingMockMessage({ sessionId: id, entityId: session?.entityId ?? '', content: message.content.content ?? '', createdAtMs: this.now() })
    return this.sendMessage(context, id, { ...outgoing, content: { ...message.content } }, confirmation)
  }
  forward(context: MessageHubContext, id: string, message: MessageObject, confirmation: string | undefined) {
    const session = this.snapshot.sessions[sessionKey(context.ownerDid, id)]
    const outgoing = createOutgoingMockMessage({ sessionId: id, entityId: session?.entityId ?? '', content: displayedContent(message), createdAtMs: this.now() })
    const refs = (message.content.refs ?? []).filter(ref => ref.target.type === 'data_obj')
    return this.sendMessage(context, id, { ...outgoing, content: { ...outgoing.content, format: message.content.format ?? outgoing.content.format, ...(refs.length ? { refs } : {}) } }, confirmation)
  }
  deleteMessage(context: MessageHubContext, id: string, message: MessageObject) {
    return this.mutate(next => {
      this.requireOwn(context)
      this.requireSession(next, context, id)
      const key = sessionKey(context.ownerDid, id)
      // Lazily generated seed histories are read by range and cannot drop a row.
      if (context.ownerDid === MOCK_SELF_DID && !next.withoutSeed[key] && this.seeds[id] && this.seeds[id] !== mockMessageReaders[id]) throw Error('backend_unavailable')
      next.hidden[key] = [...(next.hidden[key] ?? []), getMessageStableId(message, 0)]
      const prefsKey = viewerSessionKey(context, id)
      if (next.preferences[prefsKey]?.pinnedMessage?.id === messageObjId(message)) next.preferences[prefsKey] = { ...next.preferences[prefsKey], pinnedMessage: null }
    })
  }
  sendMessage(context: MessageHubContext, id: string, message: MessageObject, confirmation: string | undefined) {
    return this.mutate(next => {
      this.requireOwn(context)
      const session = this.requireSession(next, context, id)
      if (this.access(context, session, confirmation === JSON.stringify(session.binding)).mode !== 'read_write') throw Error('permission_denied')
      const to = session.binding.kind === 'native' ? session.binding.targetDid : session.binding.kind === 'tunnel' ? session.binding.endpointDid : ''
      this.append(next, session, { ...message, ...(this.lookup(session.entityId)?.type === 'group' && session.binding.kind === 'native' ? { kind: 'group_msg' as const } : {}), from: context.ownerDid, to: [to], ui_session_id: id, ui_binding: session.binding }, false)
    })
  }
  runtimeFor(context: MessageHubContext, id: string) { return (this.runtime.get(sessionKey(context.ownerDid, id)) ?? []).filter(state => state.expiresAt > this.now()) }
  clearTransient(ownerDid: string) {
    for (const key of this.runtime.keys()) if ((JSON.parse(key) as string[])[0] === ownerDid) this.runtime.delete(key)
    this.emitRuntime()
  }
  configure = (options: { delayMs?: number; failNext?: string; now?: number }) => {
    if (options.delayMs !== undefined) this.delayMs = options.delayMs
    if (options.failNext !== undefined) this.failure = options.failNext
    if (options.now !== undefined) { this.clockOverride = options.now; this.tick() }
  }
  injectRuntime = (owner: string, id: string, state: RuntimeState) => {
    const key = sessionKey(owner, id), session = this.snapshot.sessions[key]
    if (!session || !session.members[state.memberDid]) return
    this.runtime.set(key, [...(this.runtime.get(key) ?? []).filter(item => item.memberDid !== state.memberDid), state])
    this.emitRuntime()
  }
  injectDelivery = (owner: string, id: string, messageId: string, status: MessageDeliveryStatus) => this.mutate(next => {
    const key = sessionKey(owner, id)
    if (!next.sessions[key]) return
    next.delivery[key] = { ...next.delivery[key], [messageId]: status }
  })
  injectState = (owner: string, id: string, input: { shared?: z.infer<typeof sharedStateSchema>; member?: { did: string; nickname: string }; actorDid?: string }) => this.mutate(next => {
    const session = next.sessions[sessionKey(owner, id)]
    if (!session) return
    const changes: Array<{ field: string; before: string; after: string }> = []
    if (input.shared) {
      const values = sharedStateSchema.strict().parse(input.shared)
      for (const field of ['title', 'description'] as const) if (session.shared[field] !== values[field]) changes.push({ field, before: session.shared[field], after: values[field] })
      if (changes.length) session.shared = { ...values, updatedAt: this.now() }
    }
    if (input.member && session.members[input.member.did]) {
      const { nickname } = memberStateSchema.parse(input.member)
      const before = session.members[input.member.did].nickname
      if (before !== nickname) { changes.push({ field: 'nickname', before, after: nickname }); session.members[input.member.did] = { nickname, updatedAt: this.now() } }
    }
    if (!changes.length) return
    const eventId = crypto.randomUUID(), action = input.member ? 'session.member_state_changed' : changes.length === 1 && changes[0].field === 'title' ? 'session.title_changed' : 'session.shared_state_changed'
    this.append(next, session, { kind: 'event', from: owner, to: [session.entityId], created_at_ms: this.now(), ui_message_id: eventId, content: { content: action, machine: { intent: 'buckyos.action_log', data: { schema_version: 1, event_id: eventId, action, ...(input.actorDid ? { actor_did: input.actorDid } : {}), ...(input.member ? { subject_did: input.member.did } : {}), changes, occurred_at_ms: this.now(), target: { kind: 'session', session_id: id }, source: { kind: session.binding.kind, producer_did: owner } } } } }, false)
  })
  injectMessage = (owner: string, id: string, message: MessageObject) => this.mutate(next => {
    const key = sessionKey(owner, id), deleted = next.deleted[key]
    let session = next.sessions[key]
    if (!session && deleted && isMessageActivity(message) && message.created_at_ms > deleted.at && deleted.session.binding.kind === 'tunnel' && deleted.session.binding.connected) {
      session = { ...deleted.session, shared: { title: '', description: '', updatedAt: message.created_at_ms }, members: {}, lastActiveAt: message.created_at_ms, createdAt: message.created_at_ms, unreadCount: 0, lastMessage: undefined, lifecycle: 'active' }
      next.sessions[key] = session
    }
    if (!session || (deleted && message.created_at_ms <= deleted.at)) return
    this.append(next, session, message, true)
  })
  discoverConnection = (owner: string, entityId: string, binding: SessionBinding) => this.mutate(next => {
    if (binding.kind !== 'tunnel') throw Error('binding_unknown')
    const id = `connection:${binding.tunnelInstanceId}:${binding.endpointDid}:${binding.remoteContextId ?? ''}`, key = sessionKey(owner, id)
    if (next.sessions[key] || next.deleted[key]) return id
    const now = this.now()
    next.sessions[key] = { id, ownerDid: owner, entityId, title: this.lookup(entityId)?.name ?? binding.connectionName, type: 'chat', source: binding.connectionName.split(' · ')[0].toLowerCase(), binding, origin: 'connection', lifecycle: 'active', createdAt: now, lastActiveAt: now, unreadCount: 0, shared: { title: '', description: '', updatedAt: now }, members: {} }
    return id
  })
  setConnection = (owner: string, id: string, patch: { connected?: boolean; canSend?: boolean }) => this.mutate(next => {
    const session = next.sessions[sessionKey(owner, id)]
    if (session?.binding.kind === 'tunnel') session.binding = { ...session.binding, ...patch, revision: (session.binding.revision ?? 0) + 1 }
  })
  denyOwner = (owner: string) => { this.revokedOwners.add(owner); this.snapshot = { ...this.snapshot }; this.listeners.forEach(listener => listener()) }
  title(context: MessageHubContext, session: Session) { return sessionTitle(session, this.preferences(context, session.id)) }

  private groupSessions(group: MockGroup): GroupSessionInfo[] {
    const named = Object.values(group.sessions).filter(session => session.lifecycle !== 'deleted').map(session => ({ key: groupSessionKey(group.did, session.sessionId), sessionId: session.sessionId, title: session.title, description: session.description, announcement: session.announcement, sharedRevision: session.revision, lifecycle: session.lifecycle, revision: session.revision, hasGuests: Object.values(session.participants).includes('guest'), receipts: session.receipts }))
    return [{ key: group.did, sessionId: null, title: '', description: '', announcement: '', sharedRevision: group.revision, lifecycle: group.lifecycle, revision: group.revision, hasGuests: false, receipts: group.did === MOCK_PRODUCT_TEAM ? 'readers' : 'hidden' }, ...named]
  }
  private groupInfo(group: MockGroup, viewerDid: string): GroupInfo {
    const me = group.members[viewerDid]
    const myRole = me?.state === 'active' && group.lifecycle !== 'deleted' ? me.role : undefined
    const manager = myRole === 'owner' || myRole === 'admin'
    const active = manager && group.lifecycle === 'active'
    const members = group.lifecycle === 'deleted' || !myRole ? null : Object.entries(group.members).map(([did, member]) => ({ did, role: member.role, state: member.state, expiresAt: member.expiresAt, invitedBy: member.invitedBy }))
    return {
      did: group.did, name: group.name, description: group.description, ownerDid: group.ownerDid, hosted: true, lifecycle: group.lifecycle, revision: group.revision, myRole, members, sessions: this.groupSessions(group),
      can: { ...noGroupCapabilities, invite: active, remove: active, createSession: active, approve: active, updateRole: myRole === 'owner' && group.lifecycle === 'active', moderate: active, updateConfig: active, manageSession: active, inviteGuest: active, updateSharedState: active, redactAny: active, mentionAll: active, transferOwner: myRole === 'owner' && group.lifecycle === 'active' },
      messageRules: { editWindowMs: EDIT_WINDOW_MS, recallWindowMs: RECALL_WINDOW_MS },
      pendingTransfer: group.pendingTransfer,
    }
  }
  group(context: MessageHubContext, groupDid: string) {
    const group = this.snapshot.groups[groupDid]
    return group && this.canView(context) && context.mode === 'self' ? this.groupInfo(group, context.ownerDid) : null
  }
  groupStatus(context: MessageHubContext, groupDid: string) { return this.group(context, groupDid) ? 'ready' as const : 'error' as const }
  ensureGroup() { return this.initialize() }
  groupSession(context: MessageHubContext, groupDid: string, sessionId: string) {
    return this.group(context, groupDid)?.sessions.find(session => session.key === sessionId) ?? null
  }
  groupSessionMembers(context: MessageHubContext, groupDid: string, sessionId: string): GroupSessionParticipants | null {
    const snapshot = this.snapshot
    let lists = this.sessionMemberLists.get(snapshot)
    if (!lists) { lists = new Map(); this.sessionMemberLists.set(snapshot, lists) }
    const key = `${context.ownerDid}\n${groupDid}\n${sessionId}`
    if (!lists.has(key)) lists.set(key, this.listSessionMembers(snapshot, context, groupDid, sessionId))
    return lists.get(key) ?? null
  }
  ensureGroupSessionMembers() { return this.initialize() }
  /**
   * Mock named sessions inherit the group's members plus their explicit
   * participants. A guest sees only explicit participants and those who
   * posted (`complete: false`), like the host.
   */
  private listSessionMembers(snapshot: Snapshot, context: MessageHubContext, groupDid: string, sessionId: string): GroupSessionParticipants | null {
    const group = snapshot.groups[groupDid]
    if (!group || group.lifecycle === 'deleted' || !this.canView(context) || context.mode !== 'self') return null
    const sid = groupSessionId(groupDid, sessionId), session = sid === undefined ? undefined : group.sessions[sid]
    if (sid !== undefined && (!session || session.lifecycle === 'deleted')) return null
    const participants = session?.participants ?? {}
    const roleOf = (did: string) => group.members[did]?.state === 'active' && !group.members[did].blocked ? group.members[did].role : undefined
    const effective = (did: string) => participants[did] !== 'removed' && (roleOf(did) !== undefined || participants[did] === 'guest')
    if (!effective(context.ownerDid)) return null
    const complete = roleOf(context.ownerDid) !== undefined
    const posters = (snapshot.messages[sessionKey(context.ownerDid, sessionId)] ?? []).map(message => message.from)
    const candidates = complete ? [...Object.keys(group.members), ...Object.keys(participants)] : [...Object.keys(participants).filter(did => participants[did] === 'member' || participants[did] === 'guest'), ...posters, context.ownerDid]
    const items: GroupSessionParticipant[] = [...new Set(candidates)].filter(effective).map(did => {
      const role = roleOf(did)
      return role ? { did, kind: 'group_member', role, state: 'included' } : { did, kind: 'guest', state: 'included' }
    })
    if (sid !== undefined && this.groupInfo(group, context.ownerDid).can.inviteGuest) for (const did of Object.keys(participants)) if (participants[did] === 'invited_guest') items.push({ did, kind: 'guest', state: 'invited' })
    return { items, complete }
  }
  private requireGroup(next: Snapshot, context: MessageHubContext, groupDid: string) {
    this.requireOwn(context)
    const group = next.groups[groupDid]
    if (!group || group.lifecycle === 'deleted') throw Error('not-found')
    return group
  }
  private requireCapability(group: MockGroup, viewerDid: string, capability: keyof GroupInfo['can']) {
    if (!this.groupInfo(group, viewerDid).can[capability]) throw Error('capability-denied')
  }
  private requireNamedSession(group: MockGroup, sessionId: string) {
    const sid = groupSessionId(group.did, sessionId), session = sid === undefined ? undefined : group.sessions[sid]
    if (!sid || !session || session.lifecycle === 'deleted') throw Error('not-found')
    return session
  }
  private mainSession(next: Snapshot, group: MockGroup, at: number): Session {
    return next.sessions[sessionKey(MOCK_SELF_DID, group.did)] ??= { id: group.did, ownerDid: MOCK_SELF_DID, entityId: group.did, title: 'Group chat', type: 'chat', source: 'buckyos', binding: { kind: 'native', targetDid: group.did }, origin: 'remote_context', lifecycle: 'active', createdAt: at, lastActiveAt: at, unreadCount: 0, shared: { title: '', description: '', updatedAt: at }, members: {} }
  }
  /** Group events go to the group's main session (the seeded Product Team keeps its legacy session ids). */
  private groupLog(next: Snapshot, group: MockGroup, action: string, actorDid: string, subjectDid?: string) {
    const sessions = Object.values(next.sessions).filter(item => item.ownerDid === MOCK_SELF_DID && item.entityId === group.did)
    const session = sessions.find(item => item.id === group.did) ?? sessions[0]
    if (session) this.append(next, session, { ...groupEvent(group, action, actorDid, this.now(), subjectDid), ui_session_id: session.id }, false)
  }
  private activate(next: Snapshot, group: MockGroup, memberDid: string) {
    group.members[memberDid].state = 'active'
    if (memberDid === MOCK_SELF_DID) this.mainSession(next, group, this.now())
    this.groupLog(next, group, 'entity.member_joined', memberDid, memberDid)
  }
  createGroup(context: MessageHubContext, input: { name: string; members: string[] }) {
    return this.mutate(next => {
      this.requireOwn(context)
      const values = createGroupSchema.parse(input)
      if (!values.name) throw Error('invalid-group-name')
      const members = [...new Set(values.members)].filter(did => did !== context.ownerDid)
      for (const did of members) {
        const entity = this.lookup(did)
        if (!entity || (entity.type !== 'person' && entity.type !== 'agent')) throw Error('member-must-be-single-entity')
      }
      const now = this.now(), did = `did:buckyos:group:${crypto.randomUUID().slice(0, 8)}`
      const group: MockGroup = { did, name: values.name, description: '', ownerDid: context.ownerDid, lifecycle: 'active', createdAt: now, revision: crypto.randomUUID(), members: { [context.ownerDid]: { role: 'owner', state: 'active' } }, sessions: {}, inviteLinks: {} }
      for (const member of members) group.members[member] = { role: 'member', state: 'invited', inviteId: crypto.randomUUID(), expiresAt: now + INVITE_TTL_MS, invitedBy: context.ownerDid }
      next.groups[did] = group
      this.mainSession(next, group, now)
      this.groupLog(next, group, 'entity.group_created', context.ownerDid)
      for (const member of members) this.groupLog(next, group, 'entity.member_invited', context.ownerDid, member)
      return did
    })
  }
  updateGroupProfile(context: MessageHubContext, groupDid: string, profile: { name: string; description: string }) {
    return this.mutate(next => {
      const group = this.requireGroup(next, context, groupDid)
      this.requireCapability(group, context.ownerDid, 'updateConfig')
      const name = profile.name.trim(), description = profile.description.trim()
      if (!name || name.length > 64) throw Error('invalid-group-name')
      if (group.name === name && group.description === description) return
      group.name = name; group.description = description; group.revision = crypto.randomUUID()
      this.groupLog(next, group, 'entity.config_changed', context.ownerDid)
    })
  }
  inviteGroupMembers(context: MessageHubContext, groupDid: string, memberDids: string[]) {
    return this.mutate(next => {
      const group = this.requireGroup(next, context, groupDid)
      if (!this.groupInfo(group, context.ownerDid).can.invite) throw Error('capability-denied')
      const failed: Array<{ did: string; reason: string }> = []
      for (const did of [...new Set(memberDids)]) {
        const entity = this.lookup(did)
        if (!entity || (entity.type !== 'person' && entity.type !== 'agent')) { failed.push({ did, reason: 'member-must-be-single-entity' }); continue }
        const current = group.members[did]
        if (current && participating({ did, ...current })) { failed.push({ did, reason: 'member-already-participating' }); continue }
        group.members[did] = { role: 'member', state: 'invited', inviteId: crypto.randomUUID(), expiresAt: this.now() + INVITE_TTL_MS, invitedBy: context.ownerDid }
        this.groupLog(next, group, 'entity.member_invited', context.ownerDid, did)
      }
      return failed
    })
  }
  removeGroupMember(context: MessageHubContext, groupDid: string, memberDid: string) {
    return this.mutate(next => {
      const group = this.requireGroup(next, context, groupDid)
      if (!this.groupInfo(group, context.ownerDid).can.remove) throw Error('capability-denied')
      const member = group.members[memberDid]
      if (!member || !participating({ did: memberDid, ...member })) throw Error('not-found')
      if (member.role === 'owner' || memberDid === context.ownerDid) throw Error('role-not-allowed')
      member.state = member.state === 'active' ? 'removed' : 'revoked'
      this.groupLog(next, group, member.state === 'removed' ? 'entity.member_removed' : 'entity.invite_revoked', context.ownerDid, memberDid)
    })
  }
  private decideMember(context: MessageHubContext, groupDid: string, memberDid: string, approve: boolean) {
    return this.mutate(next => {
      const group = this.requireGroup(next, context, groupDid)
      this.requireCapability(group, context.ownerDid, 'approve')
      const member = group.members[memberDid]
      if (member?.state !== 'pending_admin_approval') throw Error('member-not-pending')
      if (approve) this.activate(next, group, memberDid)
      else { member.state = 'rejected'; this.groupLog(next, group, 'entity.member_rejected', context.ownerDid, memberDid) }
    })
  }
  approveGroupMember(context: MessageHubContext, groupDid: string, memberDid: string) { return this.decideMember(context, groupDid, memberDid, true) }
  rejectGroupMember(context: MessageHubContext, groupDid: string, memberDid: string) { return this.decideMember(context, groupDid, memberDid, false) }
  updateGroupMemberRole(context: MessageHubContext, groupDid: string, memberDid: string, role: 'admin' | 'member') {
    return this.mutate(next => {
      const group = this.requireGroup(next, context, groupDid)
      this.requireCapability(group, context.ownerDid, 'updateRole')
      const member = group.members[memberDid]
      if (member?.state !== 'active') throw Error('not-found')
      if (member.role === 'owner') throw Error('use-transfer-owner')
      member.role = role
      this.groupLog(next, group, 'entity.member_role_changed', context.ownerDid, memberDid)
    })
  }
  moderateGroupMember(context: MessageHubContext, groupDid: string, memberDid: string, patch: { blocked?: boolean; mutedUntil?: number | null }) {
    return this.mutate(next => {
      const group = this.requireGroup(next, context, groupDid)
      this.requireCapability(group, context.ownerDid, 'moderate')
      const member = group.members[memberDid]
      if (!member) throw Error('not-found')
      if (member.role === 'owner') throw Error('cannot-moderate-owner')
      if (patch.mutedUntil !== undefined) member.mutedUntil = patch.mutedUntil ?? undefined
      if (patch.blocked !== undefined) member.blocked = patch.blocked
      if (patch.blocked && member.state === 'active') { member.state = 'removed'; this.groupLog(next, group, 'entity.member_removed', context.ownerDid, memberDid) }
      this.groupLog(next, group, 'entity.moderation_changed', context.ownerDid, memberDid)
    })
  }
  transferGroupOwner(context: MessageHubContext, groupDid: string, memberDid: string) {
    return this.mutate(next => {
      const group = this.requireGroup(next, context, groupDid)
      if (group.ownerDid !== context.ownerDid) throw Error('controller-required')
      if (group.members[memberDid]?.state !== 'active' || memberDid === context.ownerDid) throw Error('not-found')
      group.pendingTransfer = { memberDid, transferId: crypto.randomUUID(), expiresAt: this.now() + INVITE_TTL_MS }
    })
  }
  cancelGroupOwnerTransfer(context: MessageHubContext, groupDid: string) {
    return this.mutate(next => {
      const group = this.requireGroup(next, context, groupDid)
      if (group.ownerDid !== context.ownerDid) throw Error('controller-required')
      group.pendingTransfer = undefined
    })
  }
  acceptGroupOwnerTransfer(context: MessageHubContext, groupDid: string, transferId: string) {
    return this.mutate(next => {
      const group = this.requireGroup(next, context, groupDid)
      const transfer = group.pendingTransfer
      if (!transfer || transfer.transferId !== transferId || transfer.memberDid !== context.ownerDid || transfer.expiresAt <= this.now()) throw Error('transfer-mismatch')
      group.members[group.ownerDid].role = 'admin'
      group.members[context.ownerDid].role = 'owner'
      group.ownerDid = context.ownerDid
      group.pendingTransfer = undefined
      this.groupLog(next, group, 'entity.owner_changed', context.ownerDid, context.ownerDid)
    })
  }
  createGroupInviteLink(context: MessageHubContext, groupDid: string, options: { expiresAt?: number; maxUses?: number; requireApproval?: boolean }) {
    return this.mutate(next => {
      const group = this.requireGroup(next, context, groupDid)
      this.requireCapability(group, context.ownerDid, 'invite')
      if ((options.expiresAt !== undefined && options.expiresAt <= this.now()) || options.maxUses === 0) throw Error('invalid-invite-link')
      const token = crypto.randomUUID().replace(/-/g, '')
      group.inviteLinks[token] = { requireApproval: options.requireApproval ?? false, revoked: false }
      return formatInviteLink(groupDid, token)
    })
  }
  revokeGroupInviteLink(context: MessageHubContext, groupDid: string, token: string) {
    return this.mutate(next => {
      const group = this.requireGroup(next, context, groupDid)
      this.requireCapability(group, context.ownerDid, 'invite')
      const link = group.inviteLinks[token]
      if (!link) throw Error('not-found')
      link.revoked = true
    })
  }
  requestGroupJoin(context: MessageHubContext, groupDid: string, invite?: string) {
    return this.mutate(next => {
      const group = this.requireGroup(next, context, groupDid)
      const me = group.members[context.ownerDid]
      if (me?.blocked) throw Error('blocked')
      if (me && (me.state === 'active' || me.state === 'pending_admin_approval')) throw Error('member-already-participating')
      const link = invite ? group.inviteLinks[invite] : undefined
      if (invite && (!link || link.revoked)) throw Error('invalid-invite-link')
      if (!invite) throw Error('invite-required')
      group.members[context.ownerDid] = { role: 'member', state: link!.requireApproval ? 'pending_admin_approval' : 'active' }
      if (link!.requireApproval) this.groupLog(next, group, 'entity.member_requested', context.ownerDid, context.ownerDid)
      else this.activate(next, group, context.ownerDid)
      return group.members[context.ownerDid].state
    })
  }
  leaveGroup(context: MessageHubContext, groupDid: string) {
    return this.mutate(next => {
      const group = this.requireGroup(next, context, groupDid)
      const me = group.members[context.ownerDid]
      if (me?.state !== 'active') throw Error('not-found')
      if (me.role === 'owner') throw Error('owner-must-transfer-first')
      this.groupLog(next, group, 'entity.member_left', context.ownerDid, context.ownerDid)
      me.state = 'left'
    })
  }
  deleteGroup(context: MessageHubContext, groupDid: string) {
    return this.mutate(next => {
      const group = this.requireGroup(next, context, groupDid)
      if (group.ownerDid !== context.ownerDid) throw Error('owner-required')
      this.groupLog(next, group, 'entity.group_deleted', context.ownerDid)
      group.lifecycle = 'deleted'
    })
  }
  groupInvitation(context: MessageHubContext, invitation: GroupInvitation) {
    const group = this.snapshot.groups[invitation.groupDid]
    const subject = group?.members[invitation.memberDid ?? context.ownerDid]
    const state = subject?.state === 'active' || invitation.state === 'active' ? 'joined' as const
      : subject?.state === 'pending_admin_approval' || invitation.state === 'pending_admin_approval' ? 'approval' as const
      : (invitation.expiresAt !== undefined && invitation.expiresAt <= this.now()) || group?.lifecycle !== 'active' || subject?.inviteId !== invitation.inviteId ? 'expired' as const : 'pending' as const
    return { groupName: group?.name ?? invitation.groupDid, state }
  }
  /** Accepts for the viewer, or for the viewer's agent when the invitation names one (`member_did`). */
  acceptGroupInvitation(context: MessageHubContext, invitation: GroupInvitation) {
    return this.mutate(next => {
      const group = this.requireGroup(next, context, invitation.groupDid)
      const memberDid = invitation.memberDid ?? context.ownerDid
      if (memberDid !== context.ownerDid && this.lookup(memberDid)?.type !== 'agent') throw Error('agent-owner-required')
      const member = group.members[memberDid]
      if (member?.state === 'active') throw Error('member-already-participating')
      if (member?.state !== 'invited' || member.inviteId !== invitation.inviteId) throw Error('invitation-mismatch')
      if (member.expiresAt !== undefined && member.expiresAt <= this.now()) throw Error('invite-expired')
      const inviter = member.invitedBy ? group.members[member.invitedBy] : undefined
      if (inviter && inviter.role === 'member') { member.state = 'pending_admin_approval'; this.groupLog(next, group, 'entity.member_requested', memberDid, memberDid); return }
      this.activate(next, group, memberDid)
    })
  }
  manageGroupSession(context: MessageHubContext, groupDid: string, sessionId: string, action: 'archive' | 'delete') {
    return this.mutate(next => {
      const group = this.requireGroup(next, context, groupDid)
      this.requireCapability(group, context.ownerDid, 'manageSession')
      const session = this.requireNamedSession(group, sessionId)
      session.lifecycle = action === 'archive' ? 'archived' : 'deleted'
      session.revision = crypto.randomUUID()
      const local = next.sessions[sessionKey(context.ownerDid, sessionId)]
      if (local) { if (action === 'archive') local.lifecycle = 'archived'; else { delete next.sessions[sessionKey(context.ownerDid, sessionId)]; delete next.messages[sessionKey(context.ownerDid, sessionId)] } }
      if (local && action === 'archive') this.append(next, local, { ...groupEvent(group, 'session.archived', context.ownerDid, this.now()), ui_session_id: local.id }, false)
    })
  }
  addGroupSessionMembers(context: MessageHubContext, groupDid: string, sessionId: string, memberDids: string[]) {
    return this.mutate(next => {
      const group = this.requireGroup(next, context, groupDid)
      this.requireCapability(group, context.ownerDid, 'manageSession')
      const session = this.requireNamedSession(group, sessionId)
      for (const did of memberDids) { if (group.members[did]?.state !== 'active') throw Error('not-found'); session.participants[did] = 'member' }
      session.revision = crypto.randomUUID()
    })
  }
  removeGroupSessionMember(context: MessageHubContext, groupDid: string, sessionId: string, memberDid: string) {
    return this.mutate(next => {
      const group = this.requireGroup(next, context, groupDid)
      const session = this.requireNamedSession(group, sessionId)
      this.requireCapability(group, context.ownerDid, group.members[memberDid] ? 'manageSession' : 'inviteGuest')
      const state = session.participants[memberDid]
      if (state === 'removed' || state === 'invited_guest' || (!state && group.members[memberDid]?.state !== 'active')) throw Error('not-found')
      session.participants[memberDid] = 'removed'
      session.revision = crypto.randomUUID()
      const local = next.sessions[sessionKey(context.ownerDid, sessionId)]
      if (local) this.append(next, local, { ...groupEvent(group, 'session.member_removed', context.ownerDid, this.now(), memberDid), ui_session_id: local.id }, false)
    })
  }
  leaveGroupSession(context: MessageHubContext, groupDid: string, sessionId: string) {
    return this.mutate(next => {
      const group = this.requireGroup(next, context, groupDid)
      const session = this.requireNamedSession(group, sessionId)
      session.participants[context.ownerDid] = 'removed'
      session.revision = crypto.randomUUID()
      const key = sessionKey(context.ownerDid, sessionId)
      if (next.sessions[key]) next.sessions[key].lifecycle = 'archived'
    })
  }
  inviteGroupSessionGuest(context: MessageHubContext, groupDid: string, sessionId: string, memberDid: string) {
    return this.mutate(next => {
      const group = this.requireGroup(next, context, groupDid)
      this.requireCapability(group, context.ownerDid, 'inviteGuest')
      const session = this.requireNamedSession(group, sessionId)
      if (group.members[memberDid] && participating({ did: memberDid, role: 'member', state: group.members[memberDid].state })) throw Error('not-a-session-guest')
      session.participants[memberDid] = 'invited_guest'
      session.revision = crypto.randomUUID()
      this.append(next, next.sessions[sessionKey(context.ownerDid, sessionId)] ?? this.mainSession(next, group, this.now()), { ...groupEvent(group, 'session.guest_invited', context.ownerDid, this.now(), memberDid), ui_session_id: sessionId }, false)
    })
  }
  acceptGroupSessionInvitation(context: MessageHubContext, groupDid: string, sessionId: string) {
    return this.mutate(next => {
      const group = this.requireGroup(next, context, groupDid)
      const session = group.sessions[sessionId]
      if (!session || session.participants[context.ownerDid] !== 'invited_guest') throw Error('not-found')
      session.participants[context.ownerDid] = 'guest'
      session.revision = crypto.randomUUID()
      const now = this.now(), key = groupSessionKey(group.did, sessionId)
      next.sessions[sessionKey(context.ownerDid, key)] ??= { id: key, ownerDid: context.ownerDid, entityId: group.did, title: session.title || 'Session', type: 'chat', source: 'buckyos', binding: { kind: 'native', targetDid: group.did }, origin: 'remote_context', lifecycle: 'active', createdAt: now, lastActiveAt: now, unreadCount: 0, shared: { title: session.title, description: session.description, updatedAt: now }, members: {} }
      this.append(next, next.sessions[sessionKey(context.ownerDid, key)], { ...groupEvent(group, 'session.member_added', context.ownerDid, now, context.ownerDid), ui_session_id: key }, false)
    })
  }
  /** The seeded Product Team shows readers for the latest own message; every other session hides receipts. */
  readReceipt(context: MessageHubContext, sessionId: string, message: MessageObject): ReadReceipt | null {
    const session = this.snapshot.sessions[sessionKey(context.ownerDid, sessionId)]
    const group = session ? this.snapshot.groups[session.entityId] : undefined
    if (!group || message.from !== context.ownerDid || this.groupSession(context, group.did, sessionId)?.receipts === 'hidden') return null
    const own = (this.snapshot.messages[sessionKey(context.ownerDid, sessionId)] ?? []).filter(item => item.from === context.ownerDid && !item.relates_to)
    if (own.at(-1) !== undefined && messageObjId(own.at(-1)!) !== messageObjId(message)) return null
    const readers = Object.entries(group.members).filter(([did, member]) => did !== context.ownerDid && member.state === 'active').map(([did]) => did).slice(0, 2)
    return { count: readers.length, readers }
  }
  /** Test hook: an invited member accepts. */
  simulateJoin = (groupDid: string, memberDid: string) => this.mutate(next => {
    const group = next.groups[groupDid], member = group?.members[memberDid]
    if (!group || member?.state !== 'invited') return
    member.state = 'active'
    this.groupLog(next, group, 'entity.member_joined', memberDid, memberDid)
  })
  /** Test hook: another member posts a relation (edit / redact / reaction / reply) into a group session. */
  injectRelation = (owner: string, id: string, from: string, target: string, relation: { rel: string; key?: string }, content = '') => this.mutate(next => {
    const session = next.sessions[sessionKey(owner, id)]
    if (!session) return
    const message: MessageObject = { from, to: [session.entityId], kind: 'group_msg', created_at_ms: this.now(), ui_message_id: `msg-rel-${crypto.randomUUID()}`, ui_session_id: id, ui_sender_name: this.lookup(from)?.name ?? from, relates_to: { rel: relation.rel, target, ...(relation.key ? { key: relation.key } : {}) }, content: { format: 'text/plain', content } }
    this.append(next, session, message, true)
  })
}

function buildOutgoingDraftContent({ attachments, content }: OutgoingPayload): string {
  const textContent = content.trim()
  if (attachments.length === 0) return textContent
  const names = attachments.map(attachment => attachment.relativePath || attachment.file.name)
  const visibleNames = names.slice(0, 3).join(', ')
  const remainingCount = names.length - 3
  const attachmentLine = remainingCount > 0
    ? `[Mock attachments] ${attachments.length} items: ${visibleNames}, +${remainingCount} more`
    : `[Mock attachments] ${attachments.length} items: ${visibleNames}`
  return textContent ? `${textContent}\n\n${attachmentLine}` : attachmentLine
}
