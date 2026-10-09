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
import { fetchAgentList } from '../../../api/user_mgr'
import { dictionaries } from '../../../i18n/dictionaries'
import type { ComposerAttachmentInput } from '../conversation/input/attachmentDraft'
import { InMemoryConversationMessageReader } from '../conversation/history/data-source'
import { registerObjectAccess } from '../conversation/history/objectAccess'
import type { ConversationMessageReader } from '../conversation/history/types'
import {
  acceptGroupInvitation, acceptGroupOwnerTransfer, acceptGroupSessionInvitation, applyGroupConfig, approveGroupMember, archiveGroupSession, archiveSession, blockContact, cancelGroupOwnerTransfer, checkGroupAccess, createGroup, createGroupInviteLink, createGroupSession, createSession, deleteGroup, deleteGroupSession, deleteSession, fetchAccountUsername, fetchOwnerDid, getGroupConfig, getGroupDoc, getGroupMemberState, getGroupReadMarkers, getGroupSharedState, groupErrorReason, inviteGroupMember, inviteGroupSessionGuest, leaveGroup, leaveGroupSession, listContacts, listGroupMembers, listGroupMessages, listGroupsByMember, listGroupSessionMembers, listGroupSessions, listSessionMessages, listSessions, listUiSessionState, MessageHubApiError, moderateGroupMember, postSendMessage, rejectGroupMember, removeGroupMember, removeGroupSessionMember, requestGroupJoin, restoreSession, revokeGroupInviteLink, transferGroupOwner, updateContact, updateGroupMemberRole, updateGroupMemberState, updateGroupReadMarker, updateGroupSession, updateGroupSharedState, updateRecordState, updateUiSessionState,
  type Contact, type GroupDoc, type GroupDocEnvelope, type GroupSessionItem, type SessionSummary, type UiSessionStateEntry,
} from '../datamodel/sessionApi'
import { createGroupSchema, formatInviteLink, groupSessionId } from '../groupModel'
import { editsOf, effectiveContent, messageObjId } from '../conversation/history/relations'
import { isValidMsgSessionId, randomMsgNonce, type MessageObject, type MsgObject, type RefItem } from '../protocol/msgobj'
import { createSessionSchema, creationReason, defaultPreferences, groupSharedStateSchema, memberStateSchema, pinnedMessageSchema, presentationSchema, selectDefaultSession, sessionKey, sessionTitle, sharedStateSchema, sortSessions, viewerSessionKey } from '../sessionModel'
import { ensureDefaultSession } from '../store/defaultSession'
import { noGroupCapabilities, type CreationPolicy, type Entity, type EntityDetail, type GroupCapabilities, type GroupInfo, type GroupInvitation, type GroupInvitationView, type GroupSessionInfo, type GroupSessionParticipants, type MessageHubContext, type PinnedMessage, type ReadReceipt, type RuntimeState, type Session, type SessionAccess, type SessionBinding, type SessionPreferences } from '../types'
import type { ConnectionChoice, EntityAdmission, ManageAction, MessageHubStore, OutgoingPayload, OwnerStatus } from '../store/types'
import { LocalStateStore } from './local'
import { apiObjectAccess } from './objects'
import { parseTunnelDid, projectOwner, shortDid, UNASSIGNED_ENTITY_ID, type ProjectedOwner, type ProjectionLabels } from './projection'
import { emptyHistory, itemToMessage, messageId as messageIdOf, recordMeta, removeMessage, SessionApiReader, upsertMessages, type SessionHistory } from './reader'
import { uploadAttachments } from './upload'

const SESSION_PAGE_SIZE = 50
const HISTORY_PAGE_SIZE = 64
const SUMMARY_POLL_MS = 20_000
/** Older pages a message lookup may load before giving up. */
const LOCATE_MAX_PAGES = 50
const RUNTIME_POLL_MS = 5_000
const DELIVERY_FOLLOW_DELAYS_MS = [300, 700, 1_500, 3_000, 5_000, 8_000]
const TYPING_TTL_MS = 30_000
const STATUS_LINE_TTL_MS = 10 * 60_000
const LOCALE_STORAGE_KEY = 'buckyos.prototype.locale.v1'

interface GroupSummaryEntry {
  name: string
  description: string
  ownerDid: string
  lifecycle: GroupInfo['lifecycle']
  /** false: joined on a remote host, which this zone only syncs. */
  hosted: boolean
}

interface OwnerData {
  status: OwnerStatus
  summaries: SessionSummary[]
  nextCursor?: { value: number; sessionId: string }
  contacts: Contact[]
  agentDids: string[]
  /** Groups the viewer participates in (`group.list_by_member`, self only). */
  groups: Record<string, GroupSummaryEntry>
  groupInfo: Map<string, { status: 'loading' | 'ready' | 'error'; value?: GroupInfo; loading?: Promise<void> }>
  groupSessionTitles: Record<string, string>
  /** `group.check_access(session.post)` per local group session key. */
  postAccess: Map<string, { allowed: boolean; reason?: string } | 'pending'>
  /** Read receipts of own group messages per local group session key. */
  receipts: Map<string, ReceiptState>
  /** `group.list_session_members` per local group session key; `value` stays undefined when the host refuses. */
  sessionMembers: Map<string, { groupDid: string; loaded: boolean; value?: GroupSessionParticipants; loading?: Promise<void> }>
  invitationNames: Map<string, string | null>
  /** Group DIDs of local group sessions already looked up in `group.list_by_member`. */
  groupLookups: Set<string>
  prefs: Record<string, { title: string; pinned: boolean; muted: boolean; pinnedMessage?: PinnedMessage | null }>
  prefsLoaded: Set<string>
  version: number
  projected?: { version: number; value: ProjectedOwner }
  /** Enriched `entities()` per `(viewerDid, mode)`, keyed on owner + local state versions. */
  entitiesCache: Map<string, { version: number; localVersion: number; value: Entity[] }>
  histories: Map<string, SessionHistory>
  historyStatus: Map<string, 'idle' | 'loading' | 'ready' | 'error'>
  runtime: Map<string, RuntimeState[]>
  epoch: number
  loading?: Promise<void>
}

/** `seq` ↔ `msg_id` of a group session plus the receipt of the latest own message. */
interface ReceiptState {
  seqs: Map<string, number>
  nextAfterSeq: number
  hidden: boolean
  result?: { msgId: string } & ReadReceipt
  loading?: Promise<void>
}

const RECEIPT_PAGE = 256

function translate(key: string, fallback?: string): string {
  let locale = 'en'
  try { locale = window.localStorage.getItem(LOCALE_STORAGE_KEY) ?? 'en' } catch { /* no storage */ }
  const table = (dictionaries as Record<string, Record<string, string>>)[locale] ?? dictionaries.en
  return table[key] ?? dictionaries.en[key] ?? fallback ?? key
}

function labels(): ProjectionLabels {
  return {
    direct: translate('messagehub.session.direct', 'Direct Message'),
    groupMain: translate('messagehub.session.groupMain', 'Group chat'),
    untitled: translate('messagehub.session.untitled', 'Untitled session'),
    unassigned: translate('messagehub.unassigned', 'Unassigned sessions'),
    unassignedDescription: translate('messagehub.unassignedDescription', ''),
    you: translate('messagehub.you', 'You'),
    previewImage: translate('messagehub.preview.image', '[Image]'),
    previewAttachment: translate('messagehub.preview.attachment', '[Attachment]'),
    previewUnavailable: translate('messagehub.preview.unavailable', 'Message unavailable'),
  }
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

function groupEntry(doc: GroupDoc, hosted: boolean): GroupSummaryEntry {
  const text = (value: unknown) => typeof value === 'string' ? value.trim() : ''
  return { name: text(doc.profile?.name), description: text(doc.profile?.description), ownerDid: doc.owner, lifecycle: doc.lifecycle ?? 'active', hosted }
}

function docOf(value: GroupDocEnvelope | GroupDoc | null | undefined): GroupDoc | null {
  if (!value || typeof value !== 'object') return null
  return 'doc' in value && value.doc && typeof value.doc === 'object' ? value.doc : 'id' in value ? value as GroupDoc : null
}

/** A rejection the host gives for a group operation, as a reason code the UI translates. */
function groupFailure(error: unknown): Error {
  return new Error(groupErrorReason(error))
}

function readOnlyGroupReason(reason: string | undefined): string {
  if (reason?.includes('archived')) return 'group_archived'
  if (reason?.includes('post-not-allowed') || reason?.includes('muted')) return 'group_post_denied'
  return 'group_not_member'
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
  private selfUsername = ''
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
  private deliveryFollowers = new Map<string, ReturnType<typeof setTimeout>>()
  private runtimeDenied = new Set<string>()
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
    this.selfUsername = await fetchAccountUsername().catch(() => '')
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
      data = { status: { phase: 'idle' }, summaries: [], contacts: [], agentDids: [], groups: {}, groupInfo: new Map(), groupSessionTitles: {}, postAccess: new Map(), receipts: new Map(), sessionMembers: new Map(), invitationNames: new Map(), groupLookups: new Set(), prefs: {}, prefsLoaded: new Set(), version: 0, entitiesCache: new Map(), histories: new Map(), historyStatus: new Map(), runtime: new Map(), epoch: 0 }
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
    // A failed timeline load is retried once the owner is refreshed (idle
    // histories are loaded again on the next read).
    for (const [sessionId, status] of data.historyStatus) if (status === 'error') data.historyStatus.set(sessionId, 'idle')
    data.status = { phase: 'loading' }
    this.notify()
    data.loading = (async () => {
      try {
        const ownerDid = context.ownerDid
        const [page, ownerContacts, systemContacts, agents, groups] = await Promise.all([
          listSessions({ owner: ownerDid, limit: SESSION_PAGE_SIZE, with_object: true, lifecycle: 'all', order_by: 'activity' }),
          listContacts(ownerDid).catch(() => [] as Contact[]),
          ownerDid === this.selfDid ? listContacts(undefined).catch(() => [] as Contact[]) : Promise.resolve([] as Contact[]),
          ownerDid === this.selfDid ? fetchAgentList().then(result => result.data?.agents ?? []).catch(() => []) : Promise.resolve([]),
          ownerDid === this.selfDid ? this.fetchGroups().catch(() => ({})) : Promise.resolve({}),
        ])
        if (data.epoch !== epoch) return
        data.groups = groups
        const merged = new Map<string, Contact>()
        for (const contact of systemContacts) merged.set(contact.did, contact)
        for (const contact of ownerContacts) merged.set(contact.did, contact)
        data.contacts = [...merged.values()]
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
      const previous = new Map(data.summaries.map(summary => [summary.session_id, summary.updated_at_ms]))
      this.mergeSummaries(data, page.items ?? [])
      // A group projection changed (`box_changed`, including the member-removed
      // event projected to the removed member): the cached post permission of
      // that group's sessions may be stale, so the next send asks the host again.
      const projected = this.projected(context)
      for (const summary of data.summaries) {
        if (previous.get(summary.session_id) === summary.updated_at_ms) continue
        const session = projected.sessions.find(item => item.id === summary.session_id)
        const groupDid = session && projected.entityById.get(session.entityId)?.type === 'group' ? session.entityId : undefined
        if (groupDid) this.invalidatePostAccess(data, groupDid)
      }
      const unknownGroups = context.ownerDid === this.selfDid ? [...new Set(data.summaries.map(summary => summary.state?.origin === 'group' ? summary.state.peer_did : undefined).filter((did): did is string => Boolean(did) && !data.groups[did!] && !data.groupLookups.has(did!)))] : []
      const unknownGroup = unknownGroups.length > 0
      if (unknownGroup) {
        unknownGroups.forEach(did => data.groupLookups.add(did))
        data.groups = await this.fetchGroups().catch(() => data.groups)
      }
      if (data.epoch !== epoch) return
      if (unknownGroup || JSON.stringify(data.summaries) !== before) this.bump(data)
    } catch (error) {
      console.warn('MessageHub summary refresh failed.', error)
    }
  }

  private invalidatePostAccess(data: OwnerData, groupDid: string) {
    for (const key of [...data.postAccess.keys()]) if (key === groupDid || key.startsWith(`${groupDid}/`)) data.postAccess.delete(key)
  }

  startSync(context: MessageHubContext, activeSessionId: string | null) {
    if (!this.canView(context)) return () => {}
    let stopped = false
    const data = this.owner(context.ownerDid)
    const summaryTimer = setInterval(() => { void this.refreshSummaries(context) }, SUMMARY_POLL_MS)
    const tailTimer = activeSessionId ? setInterval(() => { void this.reconcileTail(context, activeSessionId) }, SUMMARY_POLL_MS) : null
    const runtimeTimer = activeSessionId ? setInterval(() => { void this.refreshRuntime(context, activeSessionId) }, RUNTIME_POLL_MS) : null
    if (activeSessionId) {
      // (Re)selecting a session whose timeline failed to load retries it once.
      if (data.historyStatus.get(activeSessionId) === 'error') void this.loadLatest(context, activeSessionId)
      void this.refreshRuntime(context, activeSessionId); void this.ensureSessionPrefs(context, activeSessionId)
    }
    let subscription: { close(): Promise<void> } | null = null
    const token = ownerToken(context.ownerDid)
    const patterns = ['INBOX', 'SENT', 'GROUP_INBOX', 'REQUEST_BOX'].map(kind => `/msg_center/${token}/${kind}/**`)
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
      agentDids: data.agentDids,
      personalTitles: Object.fromEntries(Object.entries(data.prefs).map(([id, prefs]) => [id, prefs.title])),
      groups: Object.fromEntries(Object.entries(data.groups).map(([did, group]) => [did, { name: group.name }])),
      groupSessionTitles: data.groupSessionTitles,
      policies: {},
      labels: labels(),
      hiddenAccounts: { dids: [this.selfDid], usernames: [this.selfUsername] },
    })
    data.projected = { version: data.version, value }
    return value
  }

  findEntity(context: MessageHubContext, id: string) { return this.projected(context).entityById.get(id) }

  entities(context: MessageHubContext): Entity[] {
    if (!this.canView(context)) return []
    const data = this.owner(context.ownerDid)
    const cacheKey = `${context.viewerDid}\n${context.mode}`
    const cached = data.entitiesCache.get(cacheKey)
    if (cached && cached.version === data.version && cached.localVersion === this.local.version) return cached.value
    const { entities } = this.projected(context)
    const value = entities.map(entity => {
      const policy = this.policy(context, entity.id)
      const choices = this.connections(context, entity.id)
      const reason = entity.id === UNASSIGNED_ENTITY_ID ? 'binding_unknown' : choices.some(choice => !creationReason(context, entity, policy, choice.binding)) ? undefined : creationReason(context, entity, policy, choices[0]?.binding)
      const sessionPrefs = this.sessions(context, entity.id).map(session => this.preferences(context, session.id))
      return { ...entity, isPinned: sessionPrefs.some(prefs => prefs.pinned), isMuted: sessionPrefs.length > 0 && sessionPrefs.every(prefs => prefs.muted), sessionCreation: { policy, canCreate: !reason, unavailableReason: reason } }
    }).sort((a, b) => Number(!!b.isPinned) - Number(!!a.isPinned) || b.lastActiveAt - a.lastActiveAt || a.name.localeCompare(b.name))
    data.entitiesCache.set(cacheKey, { version: data.version, localVersion: this.local.version, value })
    return value
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
    if (action === 'accept') {
      // Accepting a sender that only reached the request box creates its
      // contact, which is named after what the conversation already shows.
      const known = this.owner(context.ownerDid).contacts.some(contact => contact.did === entityId)
      const name = this.findEntity(context, entityId)?.name
      await updateContact(entityId, { access_level: 'friend', ...(!known && name && name !== entityId ? { name } : {}) }, context.ownerDid)
    }
    else await blockContact(entityId, context.ownerDid)
    await this.ensureOwner(context, true)
  }

  private groupDid(context: MessageHubContext, entityId: string) {
    return this.findEntity(context, entityId)?.type === 'group'
  }

  sessions(context: MessageHubContext, entityId?: string, lifecycle?: Session['lifecycle']): Session[] {
    if (!this.canView(context)) return []
    const projected = this.projected(context)
    const source = entityId ? projected.sessionsByEntity.get(entityId) ?? [] : projected.sessions
    const filtered = lifecycle ? source.filter(session => session.lifecycle === lifecycle) : source
    // Preferences are resolved once per session, not once per comparison.
    const prefs = new Map(filtered.map(session => [session.id, this.preferences(context, session.id)]))
    return sortSessions(filtered, id => prefs.get(id) ?? this.preferences(context, id))
  }

  defaultSession(context: MessageHubContext, entityId: string) {
    const entity = this.findEntity(context, entityId)
    return entity ? selectDefaultSession(entity, this.sessions(context, entityId, 'active')) : null
  }

  ensureDefaultSession(context: MessageHubContext, entityId: string) { return ensureDefaultSession(this, context, entityId) }

  connections(context: MessageHubContext, entityId: string): ConnectionChoice[] {
    const entity = this.findEntity(context, entityId)
    const choices = new Map<string, ConnectionChoice>()
    if (!entity || entityId === UNASSIGNED_ENTITY_ID) return []
    const known = this.projected(context).sessionsByEntity.get(entityId) ?? []
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
    // `reader()` kicks this off during render; listeners must not run inside
    // another component's render, so the first notification is deferred.
    queueMicrotask(() => this.notify())
    try {
      const page = await listSessionMessages({ owner: context.ownerDid, session_id: sessionId, limit: HISTORY_PAGE_SIZE, descending: true, with_object: true })
      if (data.epoch !== epoch) { data.historyStatus.set(sessionId, 'idle'); this.notify(); return }
      const messages = (page.items ?? []).map(item => itemToMessage(item, context.ownerDid, sessionId, this.senderName(context, item.from), translate('messagehub.messageUnavailable', 'Message unavailable')))
      const previous = data.histories.get(key) ?? emptyHistory
      const next = upsertMessages(previous, messages, { loaded: true, hasOlder: page.next_cursor_sort_key !== undefined && page.next_cursor_record_id !== undefined, oldestCursor: page.next_cursor_sort_key !== undefined && page.next_cursor_record_id !== undefined ? { sortKey: page.next_cursor_sort_key, recordId: page.next_cursor_record_id } : previous.oldestCursor, error: undefined })
      data.histories.set(key, next)
      data.historyStatus.set(sessionId, 'ready')
      void this.refreshReceipts(context, sessionId)
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
      if (next !== current) { data.histories.set(key, next); this.notify(); void this.refreshReceipts(context, sessionId) }
    } catch (error) {
      console.warn('MessageHub timeline reconcile failed.', error)
    }
  }

  /**
   * Delivery finishes asynchronously after `post_send`. While an own message
   * of the session is still sending, its tail is re-read on a short backoff
   * so the delivered state does not wait for the next summary poll.
   */
  private followDelivery(context: MessageHubContext, sessionId: string, attempt = 0) {
    const key = viewerSessionKey(context, sessionId)
    const pending = this.deliveryFollowers.get(key)
    if (pending) clearTimeout(pending)
    this.deliveryFollowers.delete(key)
    if (attempt >= DELIVERY_FOLLOW_DELAYS_MS.length) return
    this.deliveryFollowers.set(key, setTimeout(() => {
      this.deliveryFollowers.delete(key)
      const history = this.owner(context.ownerDid).histories.get(key)
      const sending = history?.messages.some(message => message.from === context.ownerDid && message.ui_delivery_status === 'sending')
      if (!sending) return
      void this.reconcileTail(context, sessionId).finally(() => this.followDelivery(context, sessionId, attempt + 1))
    }, DELIVERY_FOLLOW_DELAYS_MS[attempt]))
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
      if (data.epoch !== epoch) { data.historyStatus.set(sessionId, 'ready'); this.notify(); return false }
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

  async locateMessage(context: MessageHubContext, sessionId: string, messageId: string) {
    if (!this.canView(context)) return false
    const data = this.owner(context.ownerDid)
    const key = viewerSessionKey(context, sessionId)
    const loaded = () => (data.histories.get(key)?.messages ?? []).some(message => messageObjId(message) === messageId)
    this.reader(context, sessionId)
    for (let wait = 0; wait < 100 && data.historyStatus.get(sessionId) === 'loading'; wait++) await new Promise(resolve => setTimeout(resolve, 100))
    for (let page = 0; page < LOCATE_MAX_PAGES && !loaded(); page++) if (!await this.loadOlder(context, sessionId)) break
    return loaded()
  }

  async markRead(context: MessageHubContext, sessionId: string, recordIds: string[]) {
    if (context.mode !== 'self' || context.ownerDid !== this.selfDid || context.viewerDid !== this.selfDid) return
    const data = this.owner(context.ownerDid)
    const key = viewerSessionKey(context, sessionId)
    const history = data.histories.get(key)
    if (!history) return
    const wanted = new Set(recordIds)
    // An edit has no row of its own: its record is read with the bubble that
    // shows it, so a placeholder and its final edit count as one unread message.
    const shown = history.messages.filter(message => wanted.has(recordMeta(message)?.recordId ?? ''))
    const targets = [...shown, ...editsOf(history.messages, shown)].filter(message => {
      const meta = recordMeta(message)
      return meta && meta.direction === 'in' && meta.recipientState === 'UNREAD'
    })
    if (targets.length === 0) return
    // Records are marked in parallel and merged in one revision so the
    // history pane rebuilds once per batch instead of once per record.
    const results = await Promise.allSettled(targets.map(async message => {
      const meta = recordMeta(message)!
      const record = await updateRecordState(meta.recordId, 'READ')
      return { ...message, ui_record: { ...meta, recipientState: record.state } }
    }))
    const updated: MessageObject[] = []
    for (const result of results) {
      if (result.status === 'fulfilled') updated.push(result.value)
      else console.warn('MessageHub mark read failed.', result.reason)
    }
    if (updated.length === 0) return
    const current = data.histories.get(key)
    if (!current) return
    data.histories.set(key, upsertMessages(current, updated))
    const summary = data.summaries.find(item => item.session_id === sessionId)
    if (summary) summary.unread_count = Math.max(0, summary.unread_count - updated.length)
    this.bump(data)
    void this.refreshSummaries(context)
    // The host also advances the marker from the READ state; this only covers
    // messages whose sequence number is already known from a receipt lookup.
    const groupDid = this.groupOfSession(context, sessionId)
    const receipts = groupDid ? data.receipts.get(sessionId) : undefined
    if (groupDid && receipts) {
      const seq = Math.max(0, ...updated.map(message => receipts.seqs.get(recordMeta(message)?.msgId ?? '') ?? 0))
      if (seq > 0) void updateGroupReadMarker(groupDid, groupSessionId(groupDid, sessionId), seq).catch(() => undefined)
    }
  }

  /** The group DID of a local group session, or undefined for any other session. */
  private groupOfSession(context: MessageHubContext, sessionId: string): string | undefined {
    const session = this.projected(context).sessions.find(item => item.id === sessionId)
    return session && this.findEntity(context, session.entityId)?.type === 'group' && session.binding.kind === 'native' ? session.entityId : undefined
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
      const data = this.owner(context.ownerDid)
      const group = data.groups[entity.id]
      if (!group) return { ...base, readOnlyReason: 'group_not_member' }
      if (session.id !== entity.id && groupSessionId(entity.id, session.id) === undefined) return { ...base, readOnlyReason: 'binding_unknown' }
      if (!group.hosted) return { ...base, mode: 'read_write' }
      const info = data.groupInfo.get(entity.id)?.value
      const stateAccess = { canEditSharedState: info?.can.updateSharedState ?? false, canEditOwnMemberState: Boolean(info?.myRole) }
      const post = data.postAccess.get(session.id)
      if (!post) void this.loadPostAccess(context, entity.id, session.id)
      if (!post || post === 'pending') return { ...base, ...stateAccess, readOnlyReason: 'permission_pending' }
      return post.allowed ? { ...base, ...stateAccess, mode: 'read_write' } : { ...base, ...stateAccess, readOnlyReason: readOnlyGroupReason(post.reason) }
    }
    return { ...base, mode: 'read_write' }
  }

  private async loadPostAccess(context: MessageHubContext, groupDid: string, sessionId: string) {
    const data = this.owner(context.ownerDid)
    data.postAccess.set(sessionId, 'pending')
    try {
      const result = await checkGroupAccess(groupDid, 'session.post', groupSessionId(groupDid, sessionId))
      data.postAccess.set(sessionId, { allowed: result.allowed, reason: result.reason ?? undefined })
    } catch (error) {
      data.postAccess.set(sessionId, { allowed: false, reason: groupErrorReason(error) })
    }
    this.bump(data)
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
    if (entity.type === 'group') return this.createGroupSessionFor(context, entity.id, values.title)
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

  /** A new session of a hosted group is a named Group Session (`group.create_session`), visible to its members. */
  private async createGroupSessionFor(context: MessageHubContext, groupDid: string, title: string): Promise<Session> {
    const data = this.owner(context.ownerDid)
    if (!data.groups[groupDid]?.hosted) throw new Error('platform_read_only')
    let result
    try {
      result = await createGroupSession(groupDid, { ...(title ? { title } : {}), idempotency_key: crypto.randomUUID() })
    } catch (error) {
      throw new Error(`rejected: ${groupErrorReason(error)}`)
    }
    if (title) data.groupSessionTitles[result.session] = title
    await this.refreshSummaries(context)
    this.bump(data)
    const session = this.projected(context).sessions.find(item => item.id === result.session)
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
      data.prefs[sessionId] = { title: prefs.title, pinned: prefs.pinned, muted: prefs.muted, pinnedMessage: prefs.pinnedMessage }
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
    if (entry.key === 'ui.pinned_message') { const pin = pinnedMessageSchema.safeParse(entry.value); prefs.pinnedMessage = pin.success ? pin.data : null }
  }

  preferences(context: MessageHubContext, sessionId: string): SessionPreferences {
    const data = this.owner(context.ownerDid)
    const stored = data.prefs[sessionId]
    const showActions = this.local.get().showActions[viewerSessionKey(context, sessionId)]
    return { ...defaultPreferences, ...stored, showActions: showActions ?? true }
  }

  async updatePreferences(context: MessageHubContext, sessionId: string, patch: Partial<SessionPreferences>) {
    if (Object.keys(patch).some(key => !['title', 'pinned', 'muted', 'showActions', 'pinnedMessage'].includes(key))) throw new Error('permission_denied')
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
      if (merged.pinnedMessage) pinnedMessageSchema.parse(merged.pinnedMessage)
      for (const [key, value] of Object.entries(ownerPatch)) await updateUiSessionState(sessionId, key === 'pinnedMessage' ? 'ui.pinned_message' : `ui.${key}`, value ?? null, context.ownerDid)
      data.prefs[sessionId] = { title: merged.title, pinned: merged.pinned, muted: merged.muted, pinnedMessage: merged.pinnedMessage ?? null }
      data.prefsLoaded.add(sessionId)
    }
    this.bump(data)
  }

  /**
   * Group Sessions keep their shared state and member states on the host
   * (`group.update_shared_state` / `group.update_member_state`, revisioned).
   * Other sessions have no authoritative state contract yet (`Session State
   * and Action Log.md` §6), so their editors stay disabled.
   */
  async updateState(context: MessageHubContext, sessionId: string, scope: 'shared' | 'member', input: unknown) {
    this.requireOwn(context)
    const values = scope === 'shared' ? groupSharedStateSchema.strict().parse(input) : memberStateSchema.strict().parse(input)
    const groupDid = this.groupOfSession(context, sessionId)
    if (!groupDid || !this.owner(context.ownerDid).groups[groupDid]?.hosted) {
      if (scope === 'shared') sharedStateSchema.parse(input)
      throw new Error('backend_unavailable')
    }
    const sid = groupSessionId(groupDid, sessionId)
    const set: Record<string, string> = {}, unset: string[] = []
    for (const [field, value] of Object.entries(values)) { if (typeof value !== 'string') continue; if (value.trim()) set[field] = value.trim(); else unset.push(field) }
    try {
      if (scope === 'shared') {
        const current = await getGroupSharedState(groupDid, sid)
        await updateGroupSharedState(groupDid, sid, current.revision, set, unset.filter(field => current[field as keyof typeof current] !== undefined), crypto.randomUUID())
      } else {
        const current = await getGroupMemberState(groupDid, sid)
        await updateGroupMemberState(groupDid, sid, current.revision, set, current.nickname !== undefined ? unset : [], crypto.randomUUID())
      }
    } catch (error) { throw groupFailure(error) }
    await this.ensureGroup(context, groupDid, true)
  }

  /**
   * Native named sessions are addressed by `to_session` (MsgObject v2); the
   * default session (`dm:*`, or the session named after the entity) omits it.
   * `thread.topic` is a semantic hint only and never carries the session id.
   */
  private sendTarget(session: Session): { to: string; toSession?: string; kind: MsgObject['kind'] } {
    const binding = session.binding as SessionBinding
    if (binding.kind === 'tunnel') return { to: binding.endpointDid, kind: 'chat' }
    if (binding.kind !== 'native') throw new Error('binding_unknown')
    const isGroup = this.projected({ viewerDid: this.selfDid, ownerDid: session.ownerDid, mode: 'self' }).entityById.get(session.entityId)?.type === 'group'
    if (isGroup) {
      const toSession = groupSessionId(binding.targetDid, session.id)
      if (toSession !== undefined && !isValidMsgSessionId(toSession)) throw new Error('rejected: invalid-to-session')
      return { to: binding.targetDid, kind: 'group_msg', toSession }
    }
    const isNamedSession = !session.id.startsWith('dm:') && session.id !== session.entityId
    const toSession = isNamedSession || isUuid(session.id) ? session.id : undefined
    // The backend rejects an invalid `to_session`; fail before the optimistic bubble.
    if (toSession !== undefined && !isValidMsgSessionId(toSession)) throw new Error('rejected: invalid-to-session')
    return { to: binding.targetDid, kind: isGroup ? 'group_msg' : 'chat', toSession }
  }

  private writableSession(context: MessageHubContext, sessionId: string, confirmation: string | undefined): Session {
    this.requireOwn(context)
    const session = this.projected(context).sessions.find(item => item.id === sessionId)
    if (!session) throw new Error('session_missing')
    if (this.access(context, session, confirmation === JSON.stringify(session.binding)).mode !== 'read_write') throw new Error('permission_denied')
    return session
  }

  async send(context: MessageHubContext, sessionId: string, payload: OutgoingPayload, confirmation: string | undefined) {
    const session = this.writableSession(context, sessionId, confirmation)
    const text = payload.content.trim()
    const attachmentSignature = payload.attachments.map(item => `${item.relativePath ?? item.file.name}:${item.file.size}:${item.file.lastModified}`).join('|')
    const relationSignature = JSON.stringify([payload.relatesTo ?? null, payload.mentions ?? null])
    const pendingKey = `${sessionKey(context.ownerDid, sessionId)}:${await hashKey(`${text}\n${attachmentSignature}\n${relationSignature}`)}`
    // The idempotency key is reused when the composer retries the same payload,
    // so an unknown result (timeout) can never produce a second message.
    const idempotencyKey = this.pendingSendKeys.get(pendingKey) ?? crypto.randomUUID()
    this.pendingSendKeys.set(pendingKey, idempotencyKey)
    const uploads = await uploadAttachments(payload.attachments)
    const refs: RefItem[] = uploads.map(upload => ({ role: 'input', target: { type: 'data_obj', obj_id: upload.objId, uri_hint: `cyfs://${upload.objId}` }, label: upload.name }))
    const meta: Pick<MsgObject, 'relates_to' | 'mentions'> = { ...(payload.relatesTo ? { relates_to: payload.relatesTo } : {}), ...(payload.mentions && (payload.mentions.all || payload.mentions.dids?.length) ? { mentions: payload.mentions } : {}) }
    await this.postOutgoing(context, session, { format: 'text/plain', content: text, ...(refs.length ? { refs } : {}) }, pendingKey, idempotencyKey, meta)
  }

  /** A failed outgoing message is posted again as a new message with the same content and attachment refs. */
  async resend(context: MessageHubContext, sessionId: string, message: MessageObject, confirmation: string | undefined) {
    const session = this.writableSession(context, sessionId, confirmation)
    const content: MsgObject['content'] = { format: message.content.format ?? 'text/plain', content: message.content.content ?? '', ...(message.content.refs?.length ? { refs: message.content.refs } : {}) }
    const pendingKey = `${sessionKey(context.ownerDid, sessionId)}:resend:${messageIdOf(message)}`
    const idempotencyKey = this.pendingSendKeys.get(pendingKey) ?? crypto.randomUUID()
    this.pendingSendKeys.set(pendingKey, idempotencyKey)
    await this.postOutgoing(context, session, content, pendingKey, idempotencyKey)
  }

  async forward(context: MessageHubContext, sessionId: string, message: MessageObject, confirmation: string | undefined) {
    const session = this.writableSession(context, sessionId, confirmation)
    const effective = effectiveContent(message)
    const refs = (effective.refs ?? []).filter(ref => ref.target.type === 'data_obj')
    const content: MsgObject['content'] = { format: effective.format ?? 'text/plain', content: effective.content ?? '', ...(refs.length ? { refs } : {}) }
    const pendingKey = `${sessionKey(context.ownerDid, sessionId)}:forward:${messageIdOf(message)}`
    const idempotencyKey = this.pendingSendKeys.get(pendingKey) ?? crypto.randomUUID()
    this.pendingSendKeys.set(pendingKey, idempotencyKey)
    await this.postOutgoing(context, session, content, pendingKey, idempotencyKey)
  }

  /**
   * Deleting is local to the owner: the mailbox record becomes `DELETED`
   * (hidden from `msg.list_session`), the message object and the other
   * participants' records stay. A message that never reached the mailbox (a
   * failed send) only leaves the timeline.
   */
  async deleteMessage(context: MessageHubContext, sessionId: string, message: MessageObject) {
    this.requireOwn(context)
    const data = this.owner(context.ownerDid)
    const key = viewerSessionKey(context, sessionId)
    const meta = recordMeta(message)
    if (meta) await updateRecordState(meta.recordId, 'DELETED')
    data.histories.set(key, removeMessage(data.histories.get(key) ?? emptyHistory, messageIdOf(message)))
    if (this.preferences(context, sessionId).pinnedMessage?.id === messageObjId(message)) await this.updatePreferences(context, sessionId, { pinnedMessage: null })
    this.bump(data)
  }

  private async postOutgoing(context: MessageHubContext, session: Session, content: MsgObject['content'], pendingKey: string, idempotencyKey: string, meta: Pick<MsgObject, 'relates_to' | 'mentions'> = {}) {
    const data = this.owner(context.ownerDid)
    const sessionId = session.id
    const target = this.sendTarget(session)
    // A resend is a new message: fresh created_at_ms and nonce. A retry of an
    // unknown outcome reuses the idempotency key, which the backend dedupes on.
    const message: MsgObject = {
      from: context.ownerDid,
      to: [target.to],
      kind: target.kind,
      ...(target.toSession !== undefined ? { to_session: target.toSession } : {}),
      ...meta,
      created_at_ms: this.now(),
      nonce: randomMsgNonce(),
      content,
    }
    const key = viewerSessionKey(context, sessionId)
    const optimisticId = `local:${idempotencyKey}`
    const optimistic: MessageObject = { ...message, ui_message_id: optimisticId, ui_session_id: sessionId, ui_delivery_status: 'sending', ui_sender_name: labels().you }
    data.histories.set(key, upsertMessages(data.histories.get(key) ?? { ...emptyHistory, loaded: true }, [optimistic]))
    this.notify()
    // Histories survive an owner refresh (epoch bump), so the outcome is always
    // applied: otherwise the optimistic bubble and its pending key would leak.
    let result
    try {
      result = await postSendMessage(message, idempotencyKey)
    } catch (error) {
      // Unknown outcome: keep the optimistic item marked failed; the same
      // idempotency key is reused on retry and the tail reconcile resolves it.
      data.histories.set(key, upsertMessages(data.histories.get(key) ?? emptyHistory, [{ ...optimistic, ui_delivery_status: 'failed' }]))
      this.notify()
      void this.reconcileTail(context, sessionId)
      throw new Error(`result_unknown: ${error instanceof Error ? error.message : String(error)}`)
    }
    if (!result.ok) {
      data.histories.set(key, removeMessage(data.histories.get(key) ?? emptyHistory, optimisticId))
      this.pendingSendKeys.delete(pendingKey)
      this.notify()
      throw new Error(`rejected: ${result.reason ?? 'unknown'}`)
    }
    this.pendingSendKeys.delete(pendingKey)
    // The composer owns the draft (it may already hold the next message), so
    // only the timeline changes here. The optimistic bubble stays until a
    // reconcile brings in the stored record with the same msg_id.
    data.histories.set(key, upsertMessages(data.histories.get(key) ?? emptyHistory, [{ ...optimistic, ui_sent_msg_id: result.msg_id }]))
    this.notify()
    await this.reconcileTail(context, sessionId)
    this.followDelivery(context, sessionId)
    const summary = data.summaries.find(item => item.session_id === sessionId)
    if (summary) { summary.last_activity_ms = Math.max(summary.last_activity_ms, message.created_at_ms); summary.lifecycle = 'active' }
    this.bump(data)
    void this.refreshSummaries(context)
  }

  /**
   * Where the runtime state (typing / processing line) of a session lives.
   * Only agents publish it, into their own owner-scoped UI state under their
   * side of the session id; human conversations have none and are not polled.
   */
  private runtimeSource(context: MessageHubContext, sessionId: string): { owner: string; sessionId: string } | null {
    if (context.mode === 'observe') return { owner: context.ownerDid, sessionId }
    const session = this.projected(context).sessions.find(item => item.id === sessionId)
    if (!session || session.binding.kind !== 'native') return null
    const entity = this.findEntity(context, session.entityId)
    const isAgent = entity?.type === 'agent' || this.owner(context.ownerDid).agentDids.includes(session.entityId)
    if (!isAgent) return null
    return { owner: session.entityId, sessionId: sessionId.startsWith('dm:') ? `dm:${context.ownerDid}` : sessionId }
  }

  private async refreshRuntime(context: MessageHubContext, sessionId: string) {
    const data = this.owner(context.ownerDid)
    if (!this.canView(context)) return
    const source = this.runtimeSource(context, sessionId)
    const blockedKey = `${context.ownerDid}\n${sessionId}`
    if (!source || this.runtimeDenied.has(blockedKey)) return
    try {
      const entries = await listUiSessionState(source.sessionId, source.owner)
      const now = this.now()
      const states: RuntimeState[] = []
      const typing = entries.find(entry => entry.key === 'typing')
      if (typing?.value === true && typing.updated_at_ms + TYPING_TTL_MS > now) states.push({ memberDid: source.owner, status: 'typing', expiresAt: typing.updated_at_ms + TYPING_TTL_MS })
      const active = entries.find(entry => entry.key === 'active')
      const statusLine = entries.find(entry => entry.key === 'status_line')
      const line = statusLine && typeof statusLine.value === 'object' && statusLine.value ? (statusLine.value as { value?: unknown }).value : statusLine?.value
      if (typeof line === 'string' && line.trim() && (statusLine!.updated_at_ms + STATUS_LINE_TTL_MS > now) && (active?.value === true || typing?.value === true)) {
        states.push({ memberDid: source.owner, status: 'processing', statusLine: line.trim(), expiresAt: statusLine!.updated_at_ms + STATUS_LINE_TTL_MS })
      }
      const previous = data.runtime.get(sessionId) ?? []
      if (JSON.stringify(previous) !== JSON.stringify(states)) { data.runtime.set(sessionId, states); this.emitRuntime() }
    } catch (error) {
      // Runtime state is best effort; a session whose state may not be read
      // is not asked again every few seconds.
      if (error instanceof MessageHubApiError && error.kind === 'permission_denied') this.runtimeDenied.add(blockedKey)
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
    this.runtimeDenied.clear()
    this.writeConfirmations.clear()
    this.emitRuntime()
  }

  title(context: MessageHubContext, session: Session) { return sessionTitle(session, this.preferences(context, session.id)) }

  private async fetchGroups(): Promise<Record<string, GroupSummaryEntry>> {
    const result = await listGroupsByMember()
    const groups: Record<string, GroupSummaryEntry> = {}
    for (const joined of result.joined ?? []) {
      if (joined.stopped) continue
      const doc = docOf(joined.doc_cache)
      groups[joined.group_did] = doc ? groupEntry(doc, false) : { name: '', description: '', ownerDid: '', lifecycle: 'active', hosted: false }
    }
    for (const item of result.items ?? []) if (item.doc?.id) groups[item.doc.id] = groupEntry(item.doc, true)
    return groups
  }

  private async reloadGroups(context: MessageHubContext) {
    const data = this.owner(context.ownerDid)
    data.groups = await this.fetchGroups()
    this.bump(data)
  }

  private own(context: MessageHubContext) {
    return context.mode === 'self' && context.ownerDid === this.selfDid && context.viewerDid === this.selfDid
  }

  group(context: MessageHubContext, groupDid: string): GroupInfo | null {
    if (!this.own(context)) return null
    return this.owner(context.ownerDid).groupInfo.get(groupDid)?.value ?? null
  }

  groupStatus(context: MessageHubContext, groupDid: string) {
    if (!this.own(context)) return 'error' as const
    return this.owner(context.ownerDid).groupInfo.get(groupDid)?.status ?? 'idle'
  }

  ensureGroup(context: MessageHubContext, groupDid: string, refresh = false): Promise<void> {
    if (!this.own(context)) return Promise.resolve()
    const data = this.owner(context.ownerDid)
    const current = data.groupInfo.get(groupDid)
    if (current?.loading) return current.loading
    if (current?.status === 'ready' && !refresh) return Promise.resolve()
    const loading = this.loadGroup(context, groupDid).finally(() => {
      const entry = data.groupInfo.get(groupDid)
      if (entry) entry.loading = undefined
    })
    data.groupInfo.set(groupDid, { ...current, status: current?.value ? current.status : 'loading', loading })
    return loading
  }

  private async loadGroup(context: MessageHubContext, groupDid: string) {
    const data = this.owner(context.ownerDid)
    const summary = data.groups[groupDid]
    if (parseTunnelDid(groupDid)) {
      data.groupInfo.set(groupDid, { status: 'error' })
      this.bump(data)
      return
    }
    this.invalidatePostAccess(data, groupDid)
    if (summary && !summary.hosted) {
      data.groupInfo.set(groupDid, { status: 'ready', value: { did: groupDid, name: summary.name, description: summary.description, ownerDid: summary.ownerDid, hosted: false, lifecycle: summary.lifecycle, revision: '', members: null, sessions: [], can: noGroupCapabilities, messageRules: {} } })
      this.bump(data)
      return
    }
    const allowed = (action: string) => checkGroupAccess(groupDid, action).then(result => result.allowed, () => false)
    const capabilityActions: Array<[keyof GroupCapabilities, string]> = [['invite', 'group.invite_member'], ['remove', 'group.remove_member'], ['createSession', 'session.create'], ['approve', 'group.approve_member'], ['updateRole', 'group.update_role'], ['moderate', 'group.moderate'], ['updateConfig', 'group.update_config'], ['manageSession', 'session.manage'], ['inviteGuest', 'session.invite_guest'], ['updateSharedState', 'session.update_shared_state'], ['redactAny', 'message.redact_any'], ['mentionAll', 'session.mention_all']]
    try {
      const [envelope, config, membership, sessions, ...granted] = await Promise.all([
        getGroupDoc(groupDid),
        getGroupConfig(groupDid).catch(() => null),
        listGroupMembers(groupDid).catch(() => null),
        listGroupSessions(groupDid).catch(() => [] as GroupSessionItem[]),
        ...capabilityActions.map(([, action]) => allowed(action)),
      ])
      const entry = groupEntry(envelope.doc, true)
      const members = membership?.items ?? null
      const mine = members?.find(member => member.member_did === this.selfDid && member.state === 'active')
      const can = { ...noGroupCapabilities, transferOwner: envelope.doc.controller === this.selfDid }
      capabilityActions.forEach(([name], index) => { can[name] = granted[index] })
      const defaultRules = config?.default_session ?? undefined
      const sessionInfos: GroupSessionInfo[] = sessions.map(session => {
        const shared = session.shared_state ?? {}
        const receipts = session.rules?.receipts ?? defaultRules?.receipts
        return {
          key: session.session, sessionId: session.session_id, title: typeof shared.title === 'string' ? shared.title : '', description: typeof shared.description === 'string' ? shared.description : '', announcement: typeof shared.announcement === 'string' ? shared.announcement : '',
          sharedRevision: typeof shared.revision === 'string' ? shared.revision : '', lifecycle: session.lifecycle, revision: session.revision, hasGuests: session.has_guests === true,
          receipts: receipts === 'count' || receipts === 'readers' ? receipts : 'hidden',
        }
      })
      for (const session of sessionInfos) if (session.sessionId !== null && session.title) data.groupSessionTitles[session.key] = session.title
      const transfer = membership?.pendingTransfer
      const previous = data.groupInfo.get(groupDid)?.value
      data.groupInfo.set(groupDid, {
        status: 'ready',
        value: {
          did: groupDid, name: entry.name, description: entry.description, ownerDid: entry.ownerDid, hosted: true, lifecycle: entry.lifecycle, revision: config?.revision ?? envelope.doc.revision,
          myRole: mine?.role ?? (data.groups[groupDid] && entry.ownerDid === this.selfDid ? 'owner' : undefined),
          members: members ? members.map(member => ({ did: member.member_did, role: member.role, state: member.state, expiresAt: member.expires_at_ms ?? undefined, invitedBy: member.invited_by ?? undefined })) : null,
          sessions: sessionInfos,
          can,
          messageRules: { editWindowMs: defaultRules?.edit?.edit_window_ms ?? undefined, recallWindowMs: defaultRules?.edit?.recall_window_ms ?? undefined },
          pendingTransfer: transfer ? { memberDid: transfer.member_did, transferId: transfer.transfer_id, expiresAt: transfer.expires_at_ms } : previous?.pendingTransfer && previous.ownerDid === entry.ownerDid ? previous.pendingTransfer : undefined,
        },
      })
      // Membership may have changed with whatever made the group reload.
      for (const [sessionId, entry] of data.sessionMembers) if (entry.groupDid === groupDid && entry.loaded) void this.ensureGroupSessionMembers(context, groupDid, sessionId, true)
    } catch (error) {
      const reason = groupErrorReason(error)
      const previous = data.groupInfo.get(groupDid)?.value
      data.groupInfo.set(groupDid, previous && reason === 'unknown' ? { status: 'ready', value: previous } : { status: 'error', value: previous ? { ...previous, lifecycle: 'deleted', members: null, myRole: undefined, can: noGroupCapabilities, pendingTransfer: undefined } : undefined })
    }
    this.bump(data)
  }

  private groupInfoOf(context: MessageHubContext, groupDid: string): GroupInfo {
    const info = this.owner(context.ownerDid).groupInfo.get(groupDid)?.value
    if (!info) throw new Error('not-found')
    return info
  }

  /** Runs a group RPC, maps its failure to the host's reason code and reloads the group afterwards. */
  private async groupOperation(context: MessageHubContext, groupDid: string, operation: () => Promise<unknown>) {
    this.requireOwn(context)
    try { await operation() } catch (error) { throw groupFailure(error) }
    await this.ensureGroup(context, groupDid, true)
  }

  async createGroup(context: MessageHubContext, input: z.infer<typeof createGroupSchema>): Promise<string> {
    this.requireOwn(context)
    const values = createGroupSchema.parse(input)
    if (!values.name) throw new Error('invalid-group-name')
    let result
    try {
      result = await createGroup({ idempotency_key: crypto.randomUUID(), profile: { name: values.name }, invitations: [...new Set(values.members)].filter(did => did !== this.selfDid).map(did => ({ member_did: did })) })
    } catch (error) {
      throw groupFailure(error)
    }
    const data = this.owner(context.ownerDid)
    data.groups[result.group_did] = { name: values.name, description: '', ownerDid: this.selfDid, lifecycle: 'active', hosted: true }
    this.bump(data)
    await Promise.all([this.reloadGroups(context).catch(() => undefined), this.refreshSummaries(context)])
    return result.group_did
  }

  async updateGroupProfile(context: MessageHubContext, groupDid: string, profile: { name: string; description: string }) {
    const name = profile.name.trim(), description = profile.description.trim()
    if (!name) throw new Error('invalid-group-name')
    const revision = this.groupInfoOf(context, groupDid).revision
    await this.groupOperation(context, groupDid, () => applyGroupConfig(groupDid, revision, { profile: { name, description } }, crypto.randomUUID()))
    const data = this.owner(context.ownerDid)
    if (data.groups[groupDid]) data.groups[groupDid] = { ...data.groups[groupDid], name, description }
    this.bump(data)
    await this.reloadGroups(context).catch(() => undefined)
  }

  async inviteGroupMembers(context: MessageHubContext, groupDid: string, memberDids: string[]) {
    this.requireOwn(context)
    const failed: Array<{ did: string; reason: string }> = []
    for (const did of [...new Set(memberDids)]) {
      try { await inviteGroupMember(groupDid, did, crypto.randomUUID()) } catch (error) { failed.push({ did, reason: groupErrorReason(error) }) }
    }
    await this.ensureGroup(context, groupDid, true)
    return failed
  }

  removeGroupMember(context: MessageHubContext, groupDid: string, memberDid: string) {
    return this.groupOperation(context, groupDid, () => removeGroupMember(groupDid, memberDid, crypto.randomUUID()))
  }

  approveGroupMember(context: MessageHubContext, groupDid: string, memberDid: string) {
    return this.groupOperation(context, groupDid, () => approveGroupMember(groupDid, memberDid, crypto.randomUUID()))
  }

  rejectGroupMember(context: MessageHubContext, groupDid: string, memberDid: string) {
    return this.groupOperation(context, groupDid, () => rejectGroupMember(groupDid, memberDid, crypto.randomUUID()))
  }

  updateGroupMemberRole(context: MessageHubContext, groupDid: string, memberDid: string, role: 'admin' | 'member') {
    return this.groupOperation(context, groupDid, () => updateGroupMemberRole(groupDid, memberDid, role, crypto.randomUUID()))
  }

  moderateGroupMember(context: MessageHubContext, groupDid: string, memberDid: string, patch: { blocked?: boolean; mutedUntil?: number | null }) {
    return this.groupOperation(context, groupDid, () => moderateGroupMember(groupDid, memberDid, { ...(patch.blocked !== undefined ? { blocked: patch.blocked } : {}), ...(patch.mutedUntil !== undefined ? { muted_until_ms: patch.mutedUntil } : {}) }, crypto.randomUUID()))
  }

  async transferGroupOwner(context: MessageHubContext, groupDid: string, memberDid: string) {
    this.requireOwn(context)
    let result
    try { result = await transferGroupOwner(groupDid, memberDid, crypto.randomUUID()) } catch (error) { throw groupFailure(error) }
    const data = this.owner(context.ownerDid)
    const entry = data.groupInfo.get(groupDid)
    if (entry?.value) entry.value = { ...entry.value, pendingTransfer: { memberDid: result.member_did, transferId: result.transfer_id, expiresAt: result.expires_at_ms } }
    this.bump(data)
    await this.ensureGroup(context, groupDid, true)
  }

  async cancelGroupOwnerTransfer(context: MessageHubContext, groupDid: string) {
    this.requireOwn(context)
    try { await cancelGroupOwnerTransfer(groupDid, crypto.randomUUID()) } catch (error) { throw groupFailure(error) }
    const data = this.owner(context.ownerDid)
    const entry = data.groupInfo.get(groupDid)
    if (entry?.value) entry.value = { ...entry.value, pendingTransfer: undefined }
    this.bump(data)
    await this.ensureGroup(context, groupDid, true)
  }

  async acceptGroupOwnerTransfer(context: MessageHubContext, groupDid: string, transferId: string) {
    this.requireOwn(context)
    try { await acceptGroupOwnerTransfer(groupDid, transferId) } catch (error) { throw groupFailure(error) }
    await this.reloadGroups(context).catch(() => undefined)
    await this.ensureGroup(context, groupDid, true)
  }

  async createGroupInviteLink(context: MessageHubContext, groupDid: string, options: { expiresAt?: number; maxUses?: number; requireApproval?: boolean }) {
    this.requireOwn(context)
    try {
      const link = await createGroupInviteLink(groupDid, { ...(options.expiresAt !== undefined ? { expires_at_ms: options.expiresAt } : {}), ...(options.maxUses !== undefined ? { max_uses: options.maxUses } : {}), ...(options.requireApproval !== undefined ? { require_approval: options.requireApproval } : {}) }, crypto.randomUUID())
      return formatInviteLink(groupDid, link.token)
    } catch (error) { throw groupFailure(error) }
  }

  async revokeGroupInviteLink(context: MessageHubContext, groupDid: string, token: string) {
    this.requireOwn(context)
    try { await revokeGroupInviteLink(groupDid, token, crypto.randomUUID()) } catch (error) { throw groupFailure(error) }
  }

  async requestGroupJoin(context: MessageHubContext, groupDid: string, invite?: string) {
    this.requireOwn(context)
    let record
    try { record = await requestGroupJoin(groupDid, invite) } catch (error) { throw groupFailure(error) }
    await Promise.all([this.reloadGroups(context).catch(() => undefined), this.refreshSummaries(context)])
    return record.state
  }

  async leaveGroup(context: MessageHubContext, groupDid: string) {
    this.requireOwn(context)
    try { await leaveGroup(groupDid, crypto.randomUUID()) } catch (error) { throw groupFailure(error) }
    await this.reloadGroups(context).catch(() => undefined)
    await this.ensureGroup(context, groupDid, true)
  }

  async deleteGroup(context: MessageHubContext, groupDid: string) {
    this.requireOwn(context)
    try { await deleteGroup(groupDid, crypto.randomUUID()) } catch (error) { throw groupFailure(error) }
    const data = this.owner(context.ownerDid)
    delete data.groups[groupDid]
    const previous = data.groupInfo.get(groupDid)?.value
    if (previous) data.groupInfo.set(groupDid, { status: 'ready', value: { ...previous, lifecycle: 'deleted', members: null, myRole: undefined, can: noGroupCapabilities, pendingTransfer: undefined } })
    for (const key of [...data.postAccess.keys()]) if (key === groupDid || key.startsWith(`${groupDid}/`)) data.postAccess.set(key, { allowed: false, reason: 'group-deleted' })
    this.bump(data)
    await this.reloadGroups(context).catch(() => undefined)
  }

  groupInvitation(context: MessageHubContext, invitation: GroupInvitation): GroupInvitationView {
    const data = this.owner(context.ownerDid)
    if (this.own(context) && !data.invitationNames.has(invitation.groupDid) && !data.groups[invitation.groupDid]) {
      data.invitationNames.set(invitation.groupDid, null)
      void getGroupDoc(invitation.groupDid).then(envelope => {
        data.invitationNames.set(invitation.groupDid, groupEntry(envelope.doc, true).name)
        this.notify()
      }, () => undefined)
    }
    // An invitation for the viewer's agent is only known to be accepted from
    // its own state; the viewer's group directory says nothing about the agent.
    const joined = invitation.state === 'active' || (!invitation.memberDid && Boolean(data.groups[invitation.groupDid]))
    const groupName = data.groups[invitation.groupDid]?.name || data.invitationNames.get(invitation.groupDid) || shortDid(invitation.groupDid)
    const state = joined ? 'joined' : invitation.state === 'pending_admin_approval' ? 'approval' : invitation.expiresAt !== undefined && invitation.expiresAt <= this.now() ? 'expired' : 'pending'
    return { groupName, state }
  }

  /** Accepts a direct invitation (`invitation_id` guards against a re-issued one); `memberDid` is the viewer's agent when accepting for it. */
  async acceptGroupInvitation(context: MessageHubContext, invitation: GroupInvitation) {
    this.requireOwn(context)
    try { await acceptGroupInvitation(invitation.groupDid, invitation.inviteId, invitation.memberDid) } catch (error) { throw groupFailure(error) }
    await Promise.all([this.reloadGroups(context).catch(() => undefined), this.refreshSummaries(context)])
  }

  groupSession(context: MessageHubContext, groupDid: string, sessionId: string): GroupSessionInfo | null {
    if (!this.own(context)) return null
    return this.owner(context.ownerDid).groupInfo.get(groupDid)?.value?.sessions.find(session => session.key === sessionId) ?? null
  }

  groupSessionMembers(context: MessageHubContext, groupDid: string, sessionId: string): GroupSessionParticipants | null {
    if (!this.own(context)) return null
    const entry = this.owner(context.ownerDid).sessionMembers.get(sessionId)
    return entry?.groupDid === groupDid ? entry.value ?? null : null
  }

  ensureGroupSessionMembers(context: MessageHubContext, groupDid: string, sessionId: string, refresh = false): Promise<void> {
    if (!this.own(context)) return Promise.resolve()
    const data = this.owner(context.ownerDid)
    if (!data.groups[groupDid]?.hosted) return Promise.resolve()
    const current = data.sessionMembers.get(sessionId)
    // A refresh asked for while a load is in flight may follow a change that load predates.
    if (current?.loading) return refresh ? current.loading.then(() => this.ensureGroupSessionMembers(context, groupDid, sessionId, true)) : current.loading
    if (current?.loaded && !refresh) return Promise.resolve()
    const loading = listGroupSessionMembers(groupDid, groupSessionId(groupDid, sessionId)).then(
      result => ({ items: result.items.map(item => ({ did: item.member_did, kind: item.kind === 'guest' ? 'guest' as const : 'group_member' as const, ...(item.role ? { role: item.role } : {}), state: item.state === 'invited' ? 'invited' as const : 'included' as const })), complete: result.complete }),
      () => undefined,
    ).then(value => {
      data.sessionMembers.set(sessionId, { groupDid, loaded: true, value })
      this.bump(data)
    })
    data.sessionMembers.set(sessionId, { groupDid, loaded: current?.loaded ?? false, value: current?.value, loading })
    return loading
  }

  private namedSession(context: MessageHubContext, groupDid: string, sessionId: string): { sid: string; revision: string } {
    const sid = groupSessionId(groupDid, sessionId)
    const info = this.groupSession(context, groupDid, sessionId)
    if (sid === undefined || !info) throw new Error('not-found')
    return { sid, revision: info.revision }
  }

  async manageGroupSession(context: MessageHubContext, groupDid: string, sessionId: string, action: 'archive' | 'delete') {
    const { sid, revision } = this.namedSession(context, groupDid, sessionId)
    await this.groupOperation(context, groupDid, () => action === 'archive' ? archiveGroupSession(groupDid, sid, revision, crypto.randomUUID()) : deleteGroupSession(groupDid, sid, revision, crypto.randomUUID()))
    await this.refreshSummaries(context)
  }

  async addGroupSessionMembers(context: MessageHubContext, groupDid: string, sessionId: string, memberDids: string[]) {
    const { sid, revision } = this.namedSession(context, groupDid, sessionId)
    await this.groupOperation(context, groupDid, () => updateGroupSession(groupDid, sid, revision, { add_members: [...new Set(memberDids)] }, crypto.randomUUID()))
  }

  async removeGroupSessionMember(context: MessageHubContext, groupDid: string, sessionId: string, memberDid: string) {
    const { sid, revision } = this.namedSession(context, groupDid, sessionId)
    await this.groupOperation(context, groupDid, () => removeGroupSessionMember(groupDid, sid, memberDid, revision, crypto.randomUUID()))
  }

  async leaveGroupSession(context: MessageHubContext, groupDid: string, sessionId: string) {
    const { sid } = this.namedSession(context, groupDid, sessionId)
    await this.groupOperation(context, groupDid, () => leaveGroupSession(groupDid, sid, crypto.randomUUID()))
    await this.refreshSummaries(context)
  }

  async inviteGroupSessionGuest(context: MessageHubContext, groupDid: string, sessionId: string, memberDid: string) {
    const { sid } = this.namedSession(context, groupDid, sessionId)
    await this.groupOperation(context, groupDid, () => inviteGroupSessionGuest(groupDid, sid, memberDid, crypto.randomUUID()))
  }

  async acceptGroupSessionInvitation(context: MessageHubContext, groupDid: string, sessionId: string) {
    this.requireOwn(context)
    try { await acceptGroupSessionInvitation(groupDid, sessionId) } catch (error) { throw groupFailure(error) }
    await Promise.all([this.reloadGroups(context).catch(() => undefined), this.refreshSummaries(context)])
    await this.ensureGroup(context, groupDid, true)
  }

  readReceipt(context: MessageHubContext, sessionId: string, message: MessageObject): ReadReceipt | null {
    if (!this.own(context)) return null
    const result = this.owner(context.ownerDid).receipts.get(sessionId)?.result
    if (!result) return null
    const id = messageObjId(message)
    return id && (id === result.msgId || message.ui_sent_msg_id === result.msgId) ? { count: result.count, readers: result.readers } : null
  }

  /**
   * Receipt of the latest own message of a group session: sequence numbers
   * come from `group.list_messages`, the readers from `group.get_read_markers`.
   * A session whose `receipts` rule is `hidden` is asked once and then left alone.
   */
  private async refreshReceipts(context: MessageHubContext, sessionId: string) {
    if (!this.own(context)) return
    const data = this.owner(context.ownerDid)
    const groupDid = this.groupOfSession(context, sessionId)
    if (!groupDid || !data.groups[groupDid]?.hosted) return
    const rule = this.groupSession(context, groupDid, sessionId)?.receipts
    if (rule === 'hidden') return
    const state = data.receipts.get(sessionId) ?? { seqs: new Map<string, number>(), nextAfterSeq: 0, hidden: false }
    data.receipts.set(sessionId, state)
    if (state.hidden || state.loading) return
    const history = data.histories.get(viewerSessionKey(context, sessionId))
    const own = [...(history?.messages ?? [])].reverse().find(message => message.from === context.ownerDid && recordMeta(message)?.direction === 'out' && !message.relates_to)
    const msgId = own ? recordMeta(own)?.msgId : undefined
    if (!msgId || state.result?.msgId === msgId) return
    state.loading = (async () => {
      try {
        const sid = groupSessionId(groupDid, sessionId)
        for (let page = 0; page < 8 && !state.seqs.has(msgId); page++) {
          const result = await listGroupMessages(groupDid, sid, state.nextAfterSeq, RECEIPT_PAGE)
          for (const item of result.items ?? []) state.seqs.set(item.obj_id, item.seq)
          state.nextAfterSeq = result.next_after_seq ?? state.nextAfterSeq
          if (!result.limited) break
        }
        const seq = state.seqs.get(msgId)
        if (seq === undefined) return
        const markers = await getGroupReadMarkers(groupDid, sid, seq)
        if (markers.visibility === 'hidden') { state.hidden = true; state.result = undefined }
        else state.result = { msgId, count: markers.count ?? 0, readers: markers.readers }
        this.bump(data)
      } catch (error) {
        console.warn('MessageHub read receipts unavailable.', error)
      } finally {
        state.loading = undefined
      }
    })()
  }
}
