import type { ComposerAttachmentInput } from '../conversation/input/attachmentDraft'
import type { z } from 'zod'
import { createCodeAssistantMockReaders } from '../../codeassistant/mockHistory'
import { InMemoryConversationMessageReader } from '../conversation/history/data-source'
import type { ConversationMessageReader } from '../conversation/history/types'
import { getMessageStableId, type MessageObject, type MessageDeliveryStatus } from '../protocol/msgobj'
import { createSessionSchema, creationReason, defaultPreferences, isMessageActivity, memberStateSchema, presentationSchema, sessionAccess, sessionKey, sessionTitle, sharedStateSchema, sortSessions, viewerSessionKey } from '../sessionModel'
import type { CreationPolicy, Entity, EntityDetail, MessageHubContext, RuntimeState, Session, SessionAccess, SessionBinding, SessionPreferences } from '../types'
import { createOutgoingMockMessage, getMockEntityDid, MOCK_SELF_DID, mockEntities, mockEntityDetails, mockMessageReaders, mockSessions } from './data'
import type { ConnectionChoice, EntityAdmission, MessageHubStore, OutgoingPayload, OwnerStatus } from '../store/types'

type Snapshot = {
  sessions: Record<string, Session>
  deleted: Record<string, { at: number; session: Session }>
  withoutSeed: Record<string, boolean>
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
  return { sessions, deleted: {}, withoutSeed: {}, messages: {}, delivery: {}, preferences: {}, policies: {}, drafts: {}, draftAttachments: {} }
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
  constructor() {
    if (import.meta.env.DEV && typeof window !== 'undefined') Object.assign(window, { __messageHubMock: this })
  }
  defaultContext() { return defaultContext }
  ownerStatus(context: MessageHubContext): OwnerStatus { return this.canView(context) ? { phase: 'ready' } : { phase: 'denied' } }
  ensureOwner() { return this.initialize() }
  startSync() { return () => {} }
  findEntity(_context: MessageHubContext, id: string) { return findEntity(id) }
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
  access(context: MessageHubContext, session: Session, confirmed: boolean): SessionAccess { return sessionAccess(context, session, confirmed) }
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
    if (stored) this.snapshot = stored
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
        if (!next.sessions[ref] || JSON.stringify(next.messages[ref]) !== JSON.stringify(old.messages[ref]) || next.withoutSeed[ref] !== old.withoutSeed[ref] || next.deleted[ref]?.at !== old.deleted[ref]?.at || JSON.stringify(next.delivery[ref]) !== JSON.stringify(old.delivery[ref])) this.readers.delete(key)
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
    return mockEntities.map(project).sort((a, b) => Number(!!b.isPinned) - Number(!!a.isPinned) || b.lastActiveAt - a.lastActiveAt || a.id.localeCompare(b.id))
  }
  connections(context: MessageHubContext, entityId: string): ConnectionChoice[] {
    const entity = findEntity(entityId)
    const choices = new Map<string, ConnectionChoice>()
    const known = [...Object.values(this.snapshot.sessions), ...Object.values(this.snapshot.deleted).map(deleted => deleted.session)].filter(session => session.ownerDid === context.ownerDid && session.entityId === entityId)
    if (entity && (entity.type === 'agent' || known.some(session => session.binding.kind === 'native'))) choices.set('native', { id: 'native', binding: { kind: 'native', targetDid: entityId }, label: 'BuckyOS' })
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
    const delta = this.snapshot.messages[key] ?? []
    const baseCount = base?.totalCount ?? 0
    const reader: ConversationMessageReader = {
      readerKey: `mock:${cacheKey}:${this.snapshot.withoutSeed[key] ? `fresh:${deletedAt}` : 'seed'}:${JSON.stringify(delivery)}`,
      totalCount: exists ? baseCount + delta.length : 0,
      readRange: async (start, count) => {
        if (!this.canView(context) || !this.snapshot.sessions[key] || this.snapshot.withoutSeed[key] !== withoutSeed || this.snapshot.deleted[key]?.at !== deletedAt) return []
        const from = Math.max(0, start)
        const seeded = base && from < baseCount ? await base.readRange(from, Math.min(count, baseCount - from)) : []
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
    return this.mutate(next => { this.requireOwn(context); if (!findEntity(entityId)) throw Error('binding_unknown'); next.policies[sessionKey(context.ownerDid, entityId)] = policy })
  }
  create(context: MessageHubContext, input: z.infer<typeof createSessionSchema>) {
    return this.mutate(next => {
      this.requireOwn(context)
      const values = createSessionSchema.parse(input)
      const entity = findEntity(values.entityId)
      if (!entity) throw Error('binding_unknown')
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
      next.deleted[key] = { at: this.now(), session: { ...session, title: findEntity(session.entityId)?.name ?? '', shared: { title: '', description: '', updatedAt: this.now() }, members: {}, lastMessage: undefined, unreadCount: 0 } }
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
      if (Object.keys(patch).some(key => !['title', 'pinned', 'muted', 'showActions'].includes(key))) throw Error('permission_denied')
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
      const values = scope === 'shared' ? sharedStateSchema.strict().parse(input) : memberStateSchema.strict().parse(input)
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
    return this.sendMessage(context, id, message, confirmation)
  }
  sendMessage(context: MessageHubContext, id: string, message: MessageObject, confirmation: string | undefined) {
    return this.mutate(next => {
      this.requireOwn(context)
      const session = this.requireSession(next, context, id)
      if (sessionAccess(context, session, confirmation === JSON.stringify(session.binding)).mode !== 'read_write') throw Error('permission_denied')
      const to = session.binding.kind === 'native' ? session.binding.targetDid : session.binding.kind === 'tunnel' ? session.binding.endpointDid : ''
      this.append(next, session, { ...message, from: context.ownerDid, to: [to], ui_session_id: id, ui_binding: session.binding }, false)
      next.drafts[viewerSessionKey(context, id)] = ''
      next.draftAttachments[viewerSessionKey(context, id)] = []
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
    next.sessions[key] = { id, ownerDid: owner, entityId, title: findEntity(entityId)?.name ?? binding.connectionName, type: 'chat', source: binding.connectionName.split(' · ')[0].toLowerCase(), binding, origin: 'connection', lifecycle: 'active', createdAt: now, lastActiveAt: now, unreadCount: 0, shared: { title: '', description: '', updatedAt: now }, members: {} }
    return id
  })
  setConnection = (owner: string, id: string, patch: { connected?: boolean; canSend?: boolean }) => this.mutate(next => {
    const session = next.sessions[sessionKey(owner, id)]
    if (session?.binding.kind === 'tunnel') session.binding = { ...session.binding, ...patch, revision: (session.binding.revision ?? 0) + 1 }
  })
  denyOwner = (owner: string) => { this.revokedOwners.add(owner); this.snapshot = { ...this.snapshot }; this.listeners.forEach(listener => listener()) }
  title(context: MessageHubContext, session: Session) { return sessionTitle(session, this.preferences(context, session.id)) }
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
