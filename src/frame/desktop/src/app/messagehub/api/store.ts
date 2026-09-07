/**
 * MessageHub data layer backed by the msg-center service. Implements
 * `MessageHubStore` on top of the real RPC surface:
 *
 * - session list / timeline projection with owner-scoped lifecycle,
 *   activity ordering and delete watermarks (server side),
 * - record-level read marking, `PostSendResult` handling, attachment
 *   upload / access, request-box admission actions,
 * - viewer → owner isolation: caches, drafts, readers and late responses are
 *   keyed by `(viewerDid, ownerDid, sessionId)`; observers never write.
 */
import { buckyos } from 'buckyos'
import type { z } from 'zod'
import { fetchAgentList, fetchUserDetail } from '../../../api/user_mgr'
import { dictionaries } from '../../../i18n/dictionaries'
import type { ComposerAttachmentInput } from '../conversation/input/attachmentDraft'
import { InMemoryConversationMessageReader } from '../conversation/history/data-source'
import { registerObjectAccess } from '../conversation/history/objectAccess'
import type { ConversationMessageReader } from '../conversation/history/types'
import {
  archiveSession, blockContact, checkGroupAccess, createSession, deleteSession, fetchOwnerDid, listContacts, listGroupsByMember, listSessionMessages, listSessions, listUiSessionState, MessageHubApiError, postSendMessage, restoreSession, updateContact, updateRecordState, updateUiSessionState,
  type Contact, type GroupSummary, type SessionSummary, type UiSessionStateEntry,
} from '../datamodel/sessionApi'
import type { MessageObject, MsgObject, RefItem } from '../protocol/msgobj'
import { createSessionSchema, creationReason, defaultPreferences, memberStateSchema, presentationSchema, sessionKey, sessionTitle, sharedStateSchema, sortSessions, viewerSessionKey } from '../sessionModel'
import type { CreationPolicy, Entity, EntityDetail, MessageHubContext, RuntimeState, Session, SessionAccess, SessionBinding, SessionPreferences } from '../types'
import type { ConnectionChoice, EntityAdmission, ManageAction, MessageHubStore, OutgoingPayload, OwnerStatus } from '../store/types'
import { LocalStateStore } from './local'
import { apiObjectAccess } from './objects'
import { projectOwner, UNASSIGNED_ENTITY_ID, type ProjectedOwner, type ProjectionLabels } from './projection'
import { emptyHistory, itemToMessage, recordMeta, removeMessage, SessionApiReader, upsertMessages, type SessionHistory } from './reader'
import { uploadAttachments } from './upload'

const SESSION_PAGE_SIZE = 50
const HISTORY_PAGE_SIZE = 64
const SUMMARY_POLL_MS = 20_000
const RUNTIME_POLL_MS = 5_000
const TYPING_TTL_MS = 30_000
const STATUS_LINE_TTL_MS = 10 * 60_000
const GROUP_POST_ACTION = 'group.post_message'
const LOCALE_STORAGE_KEY = 'buckyos.prototype.locale.v1'

interface GroupAccessCache {
  post?: boolean
  reason?: string
  pending?: boolean
}

interface OwnerData {
  status: OwnerStatus
  summaries: SessionSummary[]
  nextCursor?: { value: number; sessionId: string }
  contacts: Contact[]
  groups: GroupSummary[]
  agentDids: string[]
  prefs: Record<string, { title: string; pinned: boolean; muted: boolean }>
  prefsLoaded: Set<string>
  version: number
  projected?: { version: number; value: ProjectedOwner }
  groupAccess: Record<string, GroupAccessCache>
  histories: Map<string, SessionHistory>
  historyStatus: Map<string, 'idle' | 'loading' | 'ready' | 'error'>
  runtime: Map<string, RuntimeState[]>
  epoch: number
  loading?: Promise<void>
}

function translate(key: string, fallback?: string): string {
  let locale = 'en'
  try { locale = window.localStorage.getItem(LOCALE_STORAGE_KEY) ?? 'en' } catch { /* no storage */ }
  const table = (dictionaries as Record<string, Record<string, string>>)[locale] ?? dictionaries.en
  return table[key] ?? dictionaries.en[key] ?? fallback ?? key
}

function labels(): ProjectionLabels {
  return {
    direct: translate('messagehub.session.direct', 'Direct Message'),
    untitled: translate('messagehub.session.untitled', 'Untitled session'),
    unassigned: translate('messagehub.unassigned', 'Unassigned sessions'),
    unassignedDescription: translate('messagehub.unassignedDescription', ''),
    you: translate('messagehub.you', 'You'),
    previewImage: translate('messagehub.preview.image', '[Image]'),
    previewAttachment: translate('messagehub.preview.attachment', '[Attachment]'),
    previewUnavailable: translate('messagehub.preview.unavailable', 'Message unavailable'),
  }
}

/**
 * Zone host used to recognise zone-hosted agents (`did:web:<agent>.<zone>`).
 * Behind the dev proxy the SDK's zone host is the dev origin, so the user's
 * DID document (`binded_zone_list`) is the authoritative source; the env
 * override and the SDK value are fallbacks.
 */
async function resolveZoneHost(): Promise<string | undefined> {
  const override = String(import.meta.env.VITE_ZONE_HOST ?? '').trim()
  if (override) return override.replace(/^sys\./, '')
  try {
    const detail = await fetchUserDetail()
    const document = detail.data?.did_document as { binded_zone_list?: unknown } | undefined
    const zone = Array.isArray(document?.binded_zone_list) ? document?.binded_zone_list.find((item): item is string => typeof item === 'string' && item.startsWith('did:web:')) : undefined
    if (zone) return zone.slice('did:web:'.length)
  } catch {
    /* fall through */
  }
  const sdk = buckyos.getZoneHostName()?.trim()
  return sdk && !/^(localhost|127\.|\[?::1)/.test(sdk) ? sdk : undefined
}

function ownerToken(did: string): string {
  const parts = did.split(':')
  const method = parts[1] ?? ''
  const id = parts.slice(2).join(':').split(':')[0] ?? ''
  return method === 'web' ? id : `${id}.${method}.did`
}

function isUuid(value: string): boolean {
  return /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(value)
}

async function hashKey(input: string): Promise<string> {
  const bytes = new TextEncoder().encode(input)
  if (typeof crypto !== 'undefined' && crypto.subtle) {
    const digest = await crypto.subtle.digest('SHA-256', bytes)
    return [...new Uint8Array(digest)].map(byte => byte.toString(16).padStart(2, '0')).join('')
  }
  let hash = 0
  for (const byte of bytes) hash = (hash * 31 + byte) >>> 0
  return hash.toString(16)
}

export class MessageHubApiStore implements MessageHubStore {
  readonly isMock = false
  private selfDid = ''
  private zoneHost: string | undefined
  private owners = new Map<string, OwnerData>()
  private listeners = new Set<() => void>()
  private timeListeners = new Set<() => void>()
  private runtimeListeners = new Set<() => void>()
  private snapshotVersion = 0
  private runtimeVersion = 0
  private clockValue = Date.now()
  private initializePromise?: Promise<void>
  private readonly local = new LocalStateStore()
  private readers = new Map<string, { revision: number; reader: ConversationMessageReader }>()
  private pendingSendKeys = new Map<string, string>()
  private writeConfirmations = new Set<string>()

  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener) } }
  getSnapshot = () => this.snapshotVersion
  subscribeTime = (listener: () => void) => { this.timeListeners.add(listener); return () => { this.timeListeners.delete(listener) } }
  getTime = () => this.clockValue
  subscribeRuntime = (listener: () => void) => { this.runtimeListeners.add(listener); return () => { this.runtimeListeners.delete(listener) } }
  getRuntimeVersion = () => this.runtimeVersion
  now = () => Date.now()
  tick = () => {
    const now = this.now()
    if (Math.floor(now / 60_000) !== Math.floor(this.clockValue / 60_000)) { this.clockValue = now; this.timeListeners.forEach(listener => listener()) }
    let expired = false
    for (const owner of this.owners.values()) {
      for (const [key, states] of owner.runtime) {
        if (states.some(state => state.expiresAt <= now)) { owner.runtime.set(key, states.filter(state => state.expiresAt > now)); expired = true }
      }
    }
    if (expired) this.emitRuntime()
  }

  private notify() { this.snapshotVersion++; this.listeners.forEach(listener => listener()) }
  private emitRuntime() { this.runtimeVersion++; this.runtimeListeners.forEach(listener => listener()) }

  initialize = () => {
    this.initializePromise ??= this.load().catch(error => { this.initializePromise = undefined; throw error })
    return this.initializePromise
  }

  private async load() {
    await this.local.load()
    const selfDid = await fetchOwnerDid()
    if (!selfDid) throw new MessageHubApiError('not logged in', 'permission_denied')
    this.selfDid = selfDid
    this.zoneHost = await resolveZoneHost()
    registerObjectAccess(apiObjectAccess)
    await this.ensureOwner(this.defaultContext())
    this.notify()
  }

  defaultContext(): MessageHubContext {
    return { viewerDid: this.selfDid, ownerDid: this.selfDid, mode: 'self' }
  }

  private owner(ownerDid: string): OwnerData {
    let data = this.owners.get(ownerDid)
    if (!data) {
      data = { status: { phase: 'idle' }, summaries: [], contacts: [], groups: [], agentDids: [], prefs: {}, prefsLoaded: new Set(), version: 0, groupAccess: {}, histories: new Map(), historyStatus: new Map(), runtime: new Map(), epoch: 0 }
      this.owners.set(ownerDid, data)
    }
    return data
  }

  private bump(data: OwnerData) { data.version++; this.notify() }

  canView(context: MessageHubContext) {
    if (!this.selfDid || context.viewerDid !== this.selfDid) return false
    if (context.mode === 'self' && context.ownerDid !== this.selfDid) return false
    if (context.mode === 'observe' && context.ownerDid === this.selfDid) return false
    return this.owner(context.ownerDid).status.phase !== 'denied'
  }

  ownerStatus(context: MessageHubContext): OwnerStatus { return this.owner(context.ownerDid).status }

  private requireOwn(context: MessageHubContext) {
    if (!this.selfDid || context.viewerDid !== this.selfDid || context.mode !== 'self' || context.ownerDid !== this.selfDid) throw new Error('agent_observer')
  }

  ensureOwner(context: MessageHubContext, refresh = false): Promise<void> {
    const data = this.owner(context.ownerDid)
    if (data.loading) return data.loading
    if (data.status.phase === 'ready' && !refresh) return Promise.resolve()
    if (data.status.phase === 'denied' && !refresh) return Promise.resolve()
    const epoch = ++data.epoch
    data.status = { phase: 'loading' }
    this.notify()
    data.loading = (async () => {
      try {
        const ownerDid = context.ownerDid
        const [page, ownerContacts, systemContacts, groups, agents] = await Promise.all([
          listSessions({ owner: ownerDid, limit: SESSION_PAGE_SIZE, with_object: true, lifecycle: 'all', order_by: 'activity' }),
          listContacts(ownerDid).catch(() => [] as Contact[]),
          ownerDid === this.selfDid ? listContacts(undefined).catch(() => [] as Contact[]) : Promise.resolve([] as Contact[]),
          listGroupsByMember(ownerDid).catch(() => [] as GroupSummary[]),
          ownerDid === this.selfDid ? fetchAgentList().then(result => result.data?.agents ?? []).catch(() => []) : Promise.resolve([]),
        ])
        if (data.epoch !== epoch) return
        const merged = new Map<string, Contact>()
        for (const contact of systemContacts) merged.set(contact.did, contact)
        for (const contact of ownerContacts) merged.set(contact.did, contact)
        data.contacts = [...merged.values()]
        data.groups = groups
        data.agentDids = agents.map(agent => {
          const raw = agent as Record<string, unknown>
          return [raw.did, raw.agent_did, raw.id].find(value => typeof value === 'string' && value.startsWith('did:')) as string | undefined
        }).filter((value): value is string => Boolean(value))
        data.summaries = page.items ?? []
        data.nextCursor = page.next_cursor_updated_at_ms !== undefined && page.next_cursor_session_id !== undefined ? { value: page.next_cursor_updated_at_ms, sessionId: page.next_cursor_session_id } : undefined
        data.status = { phase: 'ready' }
        this.bump(data)
      } catch (error) {
        if (data.epoch !== epoch) return
        const apiError = error instanceof MessageHubApiError ? error : new MessageHubApiError(error instanceof Error ? error.message : String(error))
        data.status = apiError.kind === 'permission_denied' ? { phase: 'denied' } : { phase: 'error', message: apiError.message }
        this.bump(data)
      } finally {
        data.loading = undefined
      }
    })()
    return data.loading
  }

  hasMoreEntities(context: MessageHubContext) { return Boolean(this.owner(context.ownerDid).nextCursor) }

  async loadMoreEntities(context: MessageHubContext) {
    const data = this.owner(context.ownerDid)
    const cursor = data.nextCursor
    if (!cursor || data.status.phase !== 'ready') return
    const epoch = data.epoch
    const page = await listSessions({ owner: context.ownerDid, limit: SESSION_PAGE_SIZE, with_object: true, lifecycle: 'all', order_by: 'activity', cursor_updated_at_ms: cursor.value, cursor_session_id: cursor.sessionId })
    if (data.epoch !== epoch) return
    this.mergeSummaries(data, page.items ?? [])
    data.nextCursor = page.next_cursor_updated_at_ms !== undefined && page.next_cursor_session_id !== undefined ? { value: page.next_cursor_updated_at_ms, sessionId: page.next_cursor_session_id } : undefined
    this.bump(data)
  }

  private mergeSummaries(data: OwnerData, incoming: SessionSummary[]) {
    const byId = new Map(data.summaries.map(summary => [summary.session_id, summary]))
    for (const summary of incoming) byId.set(summary.session_id, summary)
    data.summaries = [...byId.values()]
  }

  private async refreshSummaries(context: MessageHubContext) {
    const data = this.owner(context.ownerDid)
    if (data.status.phase !== 'ready') return
    const epoch = data.epoch
    try {
      const page = await listSessions({ owner: context.ownerDid, limit: SESSION_PAGE_SIZE, with_object: true, lifecycle: 'all', order_by: 'activity' })
      if (data.epoch !== epoch) return
      const before = JSON.stringify(data.summaries)
      this.mergeSummaries(data, page.items ?? [])
      if (JSON.stringify(data.summaries) !== before) this.bump(data)
    } catch (error) {
      console.warn('MessageHub summary refresh failed.', error)
    }
  }

  startSync(context: MessageHubContext, activeSessionId: string | null) {
    if (!this.canView(context)) return () => {}
    let stopped = false
    const data = this.owner(context.ownerDid)
    const summaryTimer = setInterval(() => { void this.refreshSummaries(context) }, SUMMARY_POLL_MS)
    const tailTimer = activeSessionId ? setInterval(() => { void this.reconcileTail(context, activeSessionId) }, SUMMARY_POLL_MS) : null
    const runtimeTimer = activeSessionId ? setInterval(() => { void this.refreshRuntime(context, activeSessionId) }, RUNTIME_POLL_MS) : null
    if (activeSessionId) { void this.refreshRuntime(context, activeSessionId); void this.ensureSessionPrefs(context, activeSessionId) }
    let subscription: { close(): Promise<void> } | null = null
    const token = ownerToken(context.ownerDid)
    const patterns = ['box_in', 'box_sent', 'box_group_in', 'box_request'].map(prefix => `/msg_center/${token}/${prefix}_${token}/changed`)
    void buckyos.subscribeKEvent(patterns, () => {
      if (stopped || data.epoch === -1) return
      void this.refreshSummaries(context)
      if (activeSessionId) void this.reconcileTail(context, activeSessionId)
    }).then(handle => { if (stopped) void handle.close(); else subscription = handle }).catch(() => { /* polling remains the baseline */ })
    return () => {
      stopped = true
      clearInterval(summaryTimer)
      if (tailTimer) clearInterval(tailTimer)
      if (runtimeTimer) clearInterval(runtimeTimer)
      if (subscription) void subscription.close()
    }
  }

  private projected(context: MessageHubContext): ProjectedOwner {
    const data = this.owner(context.ownerDid)
    if (data.projected?.version === data.version) return data.projected.value
    const value = projectOwner({
      ownerDid: context.ownerDid,
      summaries: data.summaries,
      contacts: data.contacts,
      groups: data.groups,
      agentDids: data.agentDids,
      zoneHost: this.zoneHost,
      personalTitles: Object.fromEntries(Object.entries(data.prefs).map(([id, prefs]) => [id, prefs.title])),
      policies: {},
      labels: labels(),
    })
    data.projected = { version: data.version, value }
    return value
  }

  findEntity(context: MessageHubContext, id: string) { return this.projected(context).entities.find(entity => entity.id === id) }

  entities(context: MessageHubContext): Entity[] {
    if (!this.canView(context)) return []
    const { entities } = this.projected(context)
    return entities.map(entity => {
      const policy = this.policy(context, entity.id)
      const choices = this.connections(context, entity.id)
      const reason = entity.id === UNASSIGNED_ENTITY_ID ? 'binding_unknown' : choices.some(choice => !creationReason(context, entity, policy, choice.binding)) ? undefined : creationReason(context, entity, policy, choices[0]?.binding)
      const sessionPrefs = this.sessions(context, entity.id)
      return { ...entity, isPinned: sessionPrefs.some(session => this.preferences(context, session.id).pinned), isMuted: sessionPrefs.length > 0 && sessionPrefs.every(session => this.preferences(context, session.id).muted), sessionCreation: { policy, canCreate: !reason, unavailableReason: reason } }
    }).sort((a, b) => Number(!!b.isPinned) - Number(!!a.isPinned) || b.lastActiveAt - a.lastActiveAt || a.name.localeCompare(b.name))
  }

  entityDetail(context: MessageHubContext, id: string): EntityDetail | null {
    const entity = this.entities(context).find(item => item.id === id)
    const detail = this.projected(context).details[id]
    return entity && detail ? { ...detail, ...entity } : null
  }

  admission(context: MessageHubContext, entityId: string): EntityAdmission | null {
    if (entityId === UNASSIGNED_ENTITY_ID || entityId === context.ownerDid) return null
    const contact = this.owner(context.ownerDid).contacts.find(item => item.did === entityId)
    const grant = contact?.temp_grants?.filter(item => item.expires_at * 1000 > this.now() || item.expires_at > this.now()).sort((a, b) => b.expires_at - a.expires_at)[0]
    return {
      accessLevel: contact?.access_level ?? 'stranger',
      temporaryExpiresAt: grant ? (grant.expires_at > 1e12 ? grant.expires_at : grant.expires_at * 1000) : undefined,
      canChange: context.mode === 'self' && context.ownerDid === this.selfDid && !this.groupDid(context, entityId),
    }
  }

  async setAdmission(context: MessageHubContext, entityId: string, action: 'accept' | 'block') {
    this.requireOwn(context)
    if (action === 'accept') await updateContact(entityId, { access_level: 'friend' }, context.ownerDid)
    else await blockContact(entityId, context.ownerDid)
    await this.ensureOwner(context, true)
  }

  private groupDid(context: MessageHubContext, entityId: string) {
    return this.projected(context).entities.find(entity => entity.id === entityId)?.type === 'group'
  }

  sessions(context: MessageHubContext, entityId?: string, lifecycle?: Session['lifecycle']): Session[] {
    if (!this.canView(context)) return []
    const { sessions } = this.projected(context)
    return sortSessions(sessions.filter(session => (!entityId || session.entityId === entityId) && (!lifecycle || session.lifecycle === lifecycle)), id => this.preferences(context, id))
  }

  connections(context: MessageHubContext, entityId: string): ConnectionChoice[] {
    const entity = this.findEntity(context, entityId)
    const choices = new Map<string, ConnectionChoice>()
    if (!entity || entityId === UNASSIGNED_ENTITY_ID) return []
    const known = this.projected(context).sessions.filter(session => session.entityId === entityId)
    if (entity.domain !== 'external' || known.some(session => session.binding.kind === 'native')) choices.set('native', { id: 'native', binding: { kind: 'native', targetDid: entityId }, label: 'BuckyOS' })
    for (const session of known) {
      if (session.binding.kind === 'tunnel') choices.set(session.binding.tunnelInstanceId, { id: session.binding.tunnelInstanceId, binding: session.binding, label: session.binding.connectionName })
    }
    return [...choices.values()]
  }

  historyStatus(context: MessageHubContext, sessionId: string) {
    return this.owner(context.ownerDid).historyStatus.get(sessionId) ?? 'idle'
  }

  reader(context: MessageHubContext, sessionId: string): ConversationMessageReader {
    if (!this.canView(context)) return InMemoryConversationMessageReader.empty()
    const data = this.owner(context.ownerDid)
    const key = viewerSessionKey(context, sessionId)
    const history = data.histories.get(key) ?? emptyHistory
    if (!history.loaded && (data.historyStatus.get(sessionId) ?? 'idle') === 'idle') void this.loadLatest(context, sessionId)
    const cached = this.readers.get(key)
    if (cached && cached.revision === history.revision) return cached.reader
    const reader = new SessionApiReader(`msg-center:${encodeURIComponent(context.viewerDid)}:${encodeURIComponent(context.ownerDid)}:${encodeURIComponent(sessionId)}`, history)
    this.readers.set(key, { revision: history.revision, reader })
    return reader
  }

  private senderName(context: MessageHubContext, did: string): string | undefined {
    if (did === context.ownerDid) return labels().you
    return this.projected(context).names[did]
  }

  private async loadLatest(context: MessageHubContext, sessionId: string) {
    const data = this.owner(context.ownerDid)
    const key = viewerSessionKey(context, sessionId)
    const epoch = data.epoch
    data.historyStatus.set(sessionId, 'loading')
    this.notify()
    try {
      const page = await listSessionMessages({ owner: context.ownerDid, session_id: sessionId, limit: HISTORY_PAGE_SIZE, descending: true, with_object: true })
      if (data.epoch !== epoch) { data.historyStatus.set(sessionId, 'idle'); this.notify(); return }
      const messages = (page.items ?? []).map(item => itemToMessage(item, context.ownerDid, sessionId, this.senderName(context, item.from), translate('messagehub.messageUnavailable', 'Message unavailable')))
      const previous = data.histories.get(key) ?? emptyHistory
      const next = upsertMessages(previous, messages, { loaded: true, hasOlder: page.next_cursor_sort_key !== undefined && page.next_cursor_record_id !== undefined, oldestCursor: page.next_cursor_sort_key !== undefined && page.next_cursor_record_id !== undefined ? { sortKey: page.next_cursor_sort_key, recordId: page.next_cursor_record_id } : previous.oldestCursor, error: undefined })
      data.histories.set(key, next)
      data.historyStatus.set(sessionId, 'ready')
    } catch (error) {
      if (data.epoch !== epoch) { data.historyStatus.set(sessionId, 'idle'); this.notify(); return }
      data.historyStatus.set(sessionId, 'error')
      data.histories.set(key, { ...(data.histories.get(key) ?? emptyHistory), error: error instanceof Error ? error.message : String(error) })
    }
    this.notify()
  }

  private async reconcileTail(context: MessageHubContext, sessionId: string) {
    const data = this.owner(context.ownerDid)
    const key = viewerSessionKey(context, sessionId)
    const history = data.histories.get(key)
    if (!history?.loaded) return
    const epoch = data.epoch
    try {
      const page = await listSessionMessages({ owner: context.ownerDid, session_id: sessionId, limit: 32, descending: true, with_object: true })
      if (data.epoch !== epoch) return
      const messages = (page.items ?? []).map(item => itemToMessage(item, context.ownerDid, sessionId, this.senderName(context, item.from), translate('messagehub.messageUnavailable', 'Message unavailable')))
      const current = data.histories.get(key) ?? history
      const next = upsertMessages(current, messages)
      if (next !== current) { data.histories.set(key, next); this.notify() }
    } catch (error) {
      console.warn('MessageHub timeline reconcile failed.', error)
    }
  }

  hasOlder(context: MessageHubContext, sessionId: string) {
    return this.owner(context.ownerDid).histories.get(viewerSessionKey(context, sessionId))?.hasOlder ?? false
  }

  async loadOlder(context: MessageHubContext, sessionId: string) {
    const data = this.owner(context.ownerDid)
    const key = viewerSessionKey(context, sessionId)
    const history = data.histories.get(key)
    if (!history?.hasOlder || !history.oldestCursor || data.historyStatus.get(sessionId) === 'loading') return false
    const epoch = data.epoch
    data.historyStatus.set(sessionId, 'loading')
    try {
      const page = await listSessionMessages({ owner: context.ownerDid, session_id: sessionId, limit: HISTORY_PAGE_SIZE, descending: true, with_object: true, cursor_sort_key: history.oldestCursor.sortKey, cursor_record_id: history.oldestCursor.recordId })
      if (data.epoch !== epoch) return false
      const messages = (page.items ?? []).map(item => itemToMessage(item, context.ownerDid, sessionId, this.senderName(context, item.from), translate('messagehub.messageUnavailable', 'Message unavailable')))
      const current = data.histories.get(key) ?? history
      const more = page.next_cursor_sort_key !== undefined && page.next_cursor_record_id !== undefined
      data.histories.set(key, upsertMessages(current, messages, { hasOlder: more, oldestCursor: more ? { sortKey: page.next_cursor_sort_key!, recordId: page.next_cursor_record_id! } : current.oldestCursor }))
      data.historyStatus.set(sessionId, 'ready')
      this.notify()
      return messages.length > 0
    } catch (error) {
      if (data.epoch === epoch) data.historyStatus.set(sessionId, 'ready')
      console.warn('MessageHub load older failed.', error)
      return false
    }
  }

  async markRead(context: MessageHubContext, sessionId: string, recordIds: string[]) {
    if (context.mode !== 'self' || context.ownerDid !== this.selfDid || context.viewerDid !== this.selfDid) return
    const data = this.owner(context.ownerDid)
    const key = viewerSessionKey(context, sessionId)
    const history = data.histories.get(key)
    if (!history) return
    const targets = history.messages.filter(message => {
      const meta = recordMeta(message)
      return meta && recordIds.includes(meta.recordId) && meta.direction === 'in' && meta.recipientState === 'UNREAD'
    })
    if (targets.length === 0) return
    let changed = 0
    for (const message of targets) {
      const meta = recordMeta(message)!
      try {
        const record = await updateRecordState(meta.recordId, 'READ')
        const current = data.histories.get(key)
        if (!current) return
        data.histories.set(key, upsertMessages(current, [{ ...message, ui_record: { ...meta, recipientState: record.state } }]))
        changed++
      } catch (error) {
        console.warn('MessageHub mark read failed.', error)
      }
    }
    if (changed > 0) {
      const summary = data.summaries.find(item => item.session_id === sessionId)
      if (summary) summary.unread_count = Math.max(0, summary.unread_count - changed)
      this.bump(data)
      void this.refreshSummaries(context)
    }
  }

  private async ensureGroupAccess(context: MessageHubContext, groupDid: string) {
    const data = this.owner(context.ownerDid)
    const cache = data.groupAccess[groupDid]
    if (cache && (cache.pending || cache.post !== undefined)) return
    data.groupAccess[groupDid] = { pending: true }
    try {
      const decision = await checkGroupAccess(groupDid, context.ownerDid, GROUP_POST_ACTION)
      data.groupAccess[groupDid] = { post: decision.allowed, reason: decision.reason }
    } catch (error) {
      data.groupAccess[groupDid] = { post: false, reason: error instanceof Error ? error.message : String(error) }
    }
    this.bump(data)
  }

  access(context: MessageHubContext, session: Session, confirmed: boolean): SessionAccess {
    const own = context.mode === 'self' && context.ownerDid === this.selfDid && context.viewerDid === this.selfDid
    const base: SessionAccess = { mode: 'read_only', canManage: own, canEnableWrite: false, canEditPresentation: own, canEditSharedState: false, canEditOwnMemberState: false }
    if (!own) return { ...base, canManage: false, canEditPresentation: false, readOnlyReason: 'agent_observer' }
    if (session.binding.kind === 'unknown') return { ...base, readOnlyReason: 'binding_unknown' }
    if (session.binding.kind === 'tunnel') {
      if (!session.binding.connected || !session.binding.canSend) return { ...base, readOnlyReason: 'transport_unavailable' }
      return confirmed ? { ...base, mode: 'read_write' } : { ...base, canEnableWrite: true, readOnlyReason: 'tunnel_default' }
    }
    const entity = this.findEntity(context, session.entityId)
    if (entity?.type === 'group') {
      const cache = this.owner(context.ownerDid).groupAccess[session.entityId]
      if (!cache || cache.post === undefined) { void this.ensureGroupAccess(context, session.entityId); return { ...base, readOnlyReason: 'permission_pending' } }
      return cache.post ? { ...base, mode: 'read_write' } : { ...base, readOnlyReason: 'permission_denied' }
    }
    return { ...base, mode: 'read_write' }
  }

  draft(context: MessageHubContext, sessionId: string) { return context.mode === 'self' ? this.local.get().drafts[viewerSessionKey(context, sessionId)] ?? '' : '' }
  attachments(context: MessageHubContext, sessionId: string) { return context.mode === 'self' ? this.local.get().draftAttachments[viewerSessionKey(context, sessionId)] ?? [] : [] }
  saveDraft(context: MessageHubContext, sessionId: string, value: string) { this.requireOwn(context); return this.local.update(next => { next.drafts[viewerSessionKey(context, sessionId)] = value }) }
  saveAttachments(context: MessageHubContext, sessionId: string, attachments: ComposerAttachmentInput[]) { this.requireOwn(context); return this.local.update(next => { next.draftAttachments[viewerSessionKey(context, sessionId)] = attachments }) }

  policy(context: MessageHubContext, entityId: string): CreationPolicy { return this.local.get().policies[sessionKey(context.ownerDid, entityId)] ?? 'default' }
  async setPolicy(context: MessageHubContext, entityId: string, policy: CreationPolicy) {
    this.requireOwn(context)
    if (!this.findEntity(context, entityId)) throw new Error('binding_unknown')
    await this.local.update(next => { next.policies[sessionKey(context.ownerDid, entityId)] = policy })
    this.bump(this.owner(context.ownerDid))
  }

  async create(context: MessageHubContext, input: z.infer<typeof createSessionSchema>): Promise<Session> {
    this.requireOwn(context)
    const values = createSessionSchema.parse(input)
    const entity = this.findEntity(context, values.entityId)
    if (!entity) throw new Error('binding_unknown')
    const binding = this.connections(context, entity.id).find(choice => choice.id === values.connection)?.binding
    const reason = creationReason(context, entity, this.policy(context, entity.id), binding)
    if (reason || !binding) throw new Error(reason ?? 'binding_unknown')
    if (binding.kind !== 'native') throw new Error('platform_read_only')
    const data = this.owner(context.ownerDid)
    const epoch = data.epoch
    const state = await createSession({ owner: context.ownerDid, peer_did: entity.id, title: values.title || undefined, binding: { kind: 'native', targetDid: binding.targetDid }, origin: 'manual' })
    if (data.epoch !== epoch) throw new Error('context_changed')
    this.mergeSummaries(data, [{ session_id: state.session_id, unread_count: 0, updated_at_ms: state.updated_at_ms, last_activity_ms: state.created_at_ms, request_count: 0, lifecycle: 'active', state }])
    data.histories.set(viewerSessionKey(context, state.session_id), { ...emptyHistory, loaded: true })
    data.historyStatus.set(state.session_id, 'ready')
    this.bump(data)
    const session = this.projected(context).sessions.find(item => item.id === state.session_id)
    if (!session) throw new Error('session_missing')
    return session
  }

  async manage(context: MessageHubContext, sessionId: string, action: ManageAction) {
    this.requireOwn(context)
    const data = this.owner(context.ownerDid)
    const summary = data.summaries.find(item => item.session_id === sessionId)
    if (!summary) throw new Error('session_missing')
    const epoch = data.epoch
    const state = action === 'archive' ? await archiveSession(context.ownerDid, sessionId) : action === 'restore' ? await restoreSession(context.ownerDid, sessionId) : await deleteSession(context.ownerDid, sessionId)
    if (data.epoch !== epoch) return
    if (action === 'delete') {
      data.summaries = data.summaries.filter(item => item.session_id !== sessionId)
      data.histories.delete(viewerSessionKey(context, sessionId))
      data.historyStatus.delete(sessionId)
      data.runtime.delete(sessionId)
      delete data.prefs[sessionId]
      data.prefsLoaded.delete(sessionId)
      await this.local.update(next => {
        const key = viewerSessionKey(context, sessionId)
        delete next.drafts[key]; delete next.draftAttachments[key]; delete next.showActions[key]
      })
    } else {
      summary.lifecycle = state.lifecycle
      summary.state = state
    }
    this.bump(data)
  }

  private async ensureSessionPrefs(context: MessageHubContext, sessionId: string) {
    const data = this.owner(context.ownerDid)
    if (data.prefsLoaded.has(sessionId) || !this.canView(context)) return
    data.prefsLoaded.add(sessionId)
    try {
      const entries = await listUiSessionState(sessionId, context.ownerDid)
      const prefs = { ...defaultPreferences }
      for (const entry of entries) this.applyPref(prefs, entry)
      data.prefs[sessionId] = { title: prefs.title, pinned: prefs.pinned, muted: prefs.muted }
      this.bump(data)
    } catch (error) {
      data.prefsLoaded.delete(sessionId)
      console.warn('MessageHub session preferences unavailable.', error)
    }
  }

  private applyPref(prefs: SessionPreferences, entry: UiSessionStateEntry) {
    if (entry.key === 'ui.title' && typeof entry.value === 'string' && entry.value.trim().length <= 64) prefs.title = entry.value.trim()
    if (entry.key === 'ui.pinned' && typeof entry.value === 'boolean') prefs.pinned = entry.value
    if (entry.key === 'ui.muted' && typeof entry.value === 'boolean') prefs.muted = entry.value
  }

  preferences(context: MessageHubContext, sessionId: string): SessionPreferences {
    const data = this.owner(context.ownerDid)
    const stored = data.prefs[sessionId]
    const showActions = this.local.get().showActions[viewerSessionKey(context, sessionId)]
    return { ...defaultPreferences, ...stored, showActions: showActions ?? true }
  }

  async updatePreferences(context: MessageHubContext, sessionId: string, patch: Partial<SessionPreferences>) {
    if (Object.keys(patch).some(key => !['title', 'pinned', 'muted', 'showActions'].includes(key))) throw new Error('permission_denied')
    const data = this.owner(context.ownerDid)
    if (patch.showActions !== undefined) {
      if (typeof patch.showActions !== 'boolean') throw new Error('invalid_input')
      await this.local.update(next => { next.showActions[viewerSessionKey(context, sessionId)] = patch.showActions as boolean })
    }
    const ownerPatch = { ...patch }
    delete ownerPatch.showActions
    if (Object.keys(ownerPatch).length > 0) {
      this.requireOwn(context)
      const merged = { ...this.preferences(context, sessionId), ...ownerPatch }
      presentationSchema.parse(merged)
      for (const [key, value] of Object.entries(ownerPatch)) await updateUiSessionState(sessionId, `ui.${key}`, value, context.ownerDid)
      data.prefs[sessionId] = { title: merged.title, pinned: merged.pinned, muted: merged.muted }
      data.prefsLoaded.add(sessionId)
    }
    this.bump(data)
  }

  async updateState(context: MessageHubContext, _sessionId: string, scope: 'shared' | 'member', input: unknown) {
    this.requireOwn(context)
    if (scope === 'shared') sharedStateSchema.strict().parse(input)
    else memberStateSchema.strict().parse(input)
    // The authoritative shared / member state contract is not implemented by
    // msg-center yet (`Session State and Action Log.md` §6); the UI keeps these
    // editors disabled and never fabricates an Action Log locally.
    throw new Error('backend_unavailable')
  }

  private sendTarget(session: Session): { to: string; topic?: string; kind: MsgObject['kind'] } {
    const binding = session.binding as SessionBinding
    if (binding.kind === 'tunnel') return { to: binding.endpointDid, kind: 'chat' }
    if (binding.kind !== 'native') throw new Error('binding_unknown')
    const isGroup = this.projected({ viewerDid: this.selfDid, ownerDid: session.ownerDid, mode: 'self' }).entities.find(entity => entity.id === session.entityId)?.type === 'group'
    const keepsTopic = !session.id.startsWith('dm:') && session.id !== session.entityId
    return { to: binding.targetDid, kind: isGroup ? 'group_msg' : 'chat', topic: keepsTopic || isUuid(session.id) ? session.id : undefined }
  }

  async send(context: MessageHubContext, sessionId: string, payload: OutgoingPayload, confirmation: string | undefined) {
    this.requireOwn(context)
    const data = this.owner(context.ownerDid)
    const session = this.projected(context).sessions.find(item => item.id === sessionId)
    if (!session) throw new Error('session_missing')
    if (this.access(context, session, confirmation === JSON.stringify(session.binding)).mode !== 'read_write') throw new Error('permission_denied')
    const target = this.sendTarget(session)
    const text = payload.content.trim()
    const attachmentSignature = payload.attachments.map(item => `${item.relativePath ?? item.file.name}:${item.file.size}:${item.file.lastModified}`).join('|')
    const pendingKey = `${sessionKey(context.ownerDid, sessionId)}:${await hashKey(`${text}\n${attachmentSignature}`)}`
    // The idempotency key is reused when the composer retries the same payload,
    // so an unknown result (timeout) can never produce a second message.
    const idempotencyKey = this.pendingSendKeys.get(pendingKey) ?? crypto.randomUUID()
    this.pendingSendKeys.set(pendingKey, idempotencyKey)
    const uploads = await uploadAttachments(payload.attachments)
    const refs: RefItem[] = uploads.map(upload => ({ role: 'input', target: { type: 'data_obj', obj_id: upload.objId, uri_hint: `cyfs://${upload.objId}` }, label: upload.name }))
    const message: MsgObject = {
      from: context.ownerDid,
      to: [target.to],
      kind: target.kind,
      created_at_ms: this.now(),
      content: { format: 'text/plain', content: text, ...(refs.length ? { refs } : {}) },
      ...(target.topic ? { thread: { topic: target.topic } } : {}),
    }
    const key = viewerSessionKey(context, sessionId)
    const optimisticId = `local:${idempotencyKey}`
    const optimistic: MessageObject = { ...message, ui_message_id: optimisticId, ui_session_id: sessionId, ui_delivery_status: 'sending', ui_sender_name: labels().you }
    data.histories.set(key, upsertMessages(data.histories.get(key) ?? { ...emptyHistory, loaded: true }, [optimistic]))
    this.notify()
    const epoch = data.epoch
    let result
    try {
      result = await postSendMessage(message, idempotencyKey)
    } catch (error) {
      // Unknown outcome: keep the optimistic item marked failed; the same
      // idempotency key is reused on retry and the tail reconcile resolves it.
      if (data.epoch === epoch) {
        data.histories.set(key, upsertMessages(data.histories.get(key) ?? emptyHistory, [{ ...optimistic, ui_delivery_status: 'failed' }]))
        this.notify()
        void this.reconcileTail(context, sessionId)
      }
      throw new Error(`result_unknown: ${error instanceof Error ? error.message : String(error)}`)
    }
    if (data.epoch !== epoch) return
    if (!result.ok) {
      data.histories.set(key, removeMessage(data.histories.get(key) ?? emptyHistory, optimisticId))
      this.pendingSendKeys.delete(pendingKey)
      this.notify()
      throw new Error(`rejected: ${result.reason ?? 'unknown'}`)
    }
    this.pendingSendKeys.delete(pendingKey)
    data.histories.set(key, removeMessage(data.histories.get(key) ?? emptyHistory, optimisticId))
    await this.local.update(next => { next.drafts[key] = ''; next.draftAttachments[key] = [] })
    await this.reconcileTail(context, sessionId)
    const summary = data.summaries.find(item => item.session_id === sessionId)
    if (summary) { summary.last_activity_ms = Math.max(summary.last_activity_ms, message.created_at_ms); summary.lifecycle = 'active' }
    this.bump(data)
    void this.refreshSummaries(context)
  }

  private async refreshRuntime(context: MessageHubContext, sessionId: string) {
    const data = this.owner(context.ownerDid)
    if (!this.canView(context)) return
    try {
      const entries = await listUiSessionState(sessionId)
      const now = this.now()
      const states: RuntimeState[] = []
      const typing = entries.find(entry => entry.key === 'typing')
      if (typing?.value === true && typing.updated_at_ms + TYPING_TTL_MS > now) states.push({ memberDid: context.ownerDid, status: 'typing', expiresAt: typing.updated_at_ms + TYPING_TTL_MS })
      const active = entries.find(entry => entry.key === 'active')
      const statusLine = entries.find(entry => entry.key === 'status_line')
      const line = statusLine && typeof statusLine.value === 'object' && statusLine.value ? (statusLine.value as { value?: unknown }).value : statusLine?.value
      if (typeof line === 'string' && line.trim() && (statusLine!.updated_at_ms + STATUS_LINE_TTL_MS > now) && (active?.value === true || typing?.value === true)) {
        states.push({ memberDid: context.ownerDid, status: 'processing', statusLine: line.trim(), expiresAt: statusLine!.updated_at_ms + STATUS_LINE_TTL_MS })
      }
      const previous = data.runtime.get(sessionId) ?? []
      if (JSON.stringify(previous) !== JSON.stringify(states)) { data.runtime.set(sessionId, states); this.emitRuntime() }
    } catch {
      /* runtime state is best effort */
    }
  }

  runtimeFor(context: MessageHubContext, sessionId: string) {
    return (this.owner(context.ownerDid).runtime.get(sessionId) ?? []).filter(state => state.expiresAt > this.now())
  }

  /**
   * Leaving a context drops its runtime state and write confirmations. Loaded
   * data stays keyed by owner (no cross-owner reuse is possible) and in-flight
   * loads keep their epoch: React StrictMode mounts twice in development.
   */
  clearTransient(ownerDid: string) {
    const data = this.owner(ownerDid)
    data.runtime.clear()
    this.writeConfirmations.clear()
    this.emitRuntime()
  }

  title(context: MessageHubContext, session: Session) { return sessionTitle(session, this.preferences(context, session.id)) }
}
