/* ── Unread conversations from msg-center alone ──
 *
 * For a caller without the MessageHub store: reads the session list and the names around it and projects them as
 * MessageHub does, so the rows are the ones the Shell's tray shows.
 */

import type { DID } from 'buckyos'
import type { ZoneChannel } from '../channel.ts'
import { readAgentDids } from '../controlPanel/account.ts'
import { projectOwner, type ProjectedOwner, type ProjectionLabels } from '../messagehub/projection.ts'
import { applySessionPref, defaultSessionPrefs, groupName, participatedGroups, sessionTitle, type SessionPrefs } from '../messagehub/session.ts'
import { collectUnreadRecords, markRecordsRead } from '../messagehub/unread.ts'
import type {
  Contact,
  GroupsByMember,
  ListSessionRequest,
  MailboxRecord,
  SessionMessageItem,
  SessionMessagePage,
  SessionSummary,
  SessionSummaryPage,
  UiSessionStateEntry,
} from '../messagehub/wire.ts'
import { unreadConversations, type UnreadConversations } from './messages.ts'

const SESSION_PAGE_SIZE = 50
/** Sessions are listed by activity; unread ones further back than this only count in `notify.summary`. */
const MAX_SESSION_PAGES = 4
/** How often the pages after the first are read again. */
const FULL_READ_MS = 60_000
/** How long names, pictures, groups and session settings are reused. */
const CACHE_MS = 60_000
const PARALLEL_READS = 8

export interface ConversationReaderOptions {
  channel: ZoneChannel
  userId: string
  /** The DID the account's mailbox belongs to (`readAccountDid`). */
  owner: DID
  /** The words the projection needs, in the viewer's language. */
  labels: () => ProjectionLabels
  now?: () => number
}

interface People {
  contacts: Contact[]
  agentDids: string[]
  groups: Record<string, { name: string }>
}

export class ConversationReader {
  private readonly options: ConversationReaderOptions
  private people?: { at: number; value: Promise<People> }
  private prefs = new Map<string, { at: number; value: SessionPrefs }>()
  private summaries: SessionSummary[] = []
  private fullReadAt = -Infinity

  constructor(options: ConversationReaderOptions) {
    this.options = options
  }

  private now() { return this.options.now?.() ?? Date.now() }

  private call<T>(method: string, params: Record<string, unknown>): Promise<T> {
    return this.options.channel.rpc<T>('msg-center', method, params)
  }

  async read(): Promise<UnreadConversations> {
    const full = this.now() - this.fullReadAt >= FULL_READ_MS
    const [listed, people] = await Promise.all([this.readSummaries(full ? MAX_SESSION_PAGES : 1), this.readPeople()])
    if (full) this.fullReadAt = this.now()
    const fresh = new Set(listed.map(summary => summary.session_id))
    const summaries = full ? listed : [...listed, ...this.summaries.filter(summary => !fresh.has(summary.session_id))]
    this.summaries = summaries
    const projected = this.project(summaries, people)
    const candidates = projected.sessions.filter(session => session.unreadCount > 0 && !projected.entityById.get(session.entityId)?.systemNotify)
    await this.loadPrefs(candidates.map(session => session.id))
    const prefsOf = (sessionId: string) => this.prefs.get(sessionId)?.value ?? defaultSessionPrefs
    const byId = new Map(candidates.map(session => [session.id, session]))
    return unreadConversations({
      sessions: candidates,
      entity: entityId => projected.entityById.get(entityId),
      muted: sessionId => prefsOf(sessionId).muted,
      title: sessionId => sessionTitle(byId.get(sessionId)!, prefsOf(sessionId)),
      sessionCount: entityId => projected.sessionsByEntity.get(entityId)?.length ?? 0,
    })
  }

  /** Marks what the conversation held when it was last read. */
  async markRead(sessionId: string): Promise<void> {
    const records: SessionMessageItem[] = []
    await collectUnreadRecords(
      request => this.listSession(request),
      { owner: this.options.owner, sessionId, boundary: this.summaries.find(summary => summary.session_id === sessionId)?.last_record?.record },
      records,
    )
    this.fullReadAt = -Infinity
    const failures: unknown[] = []
    await markRecordsRead(
      (recordId, state) => this.call<MailboxRecord>('msg.update_record_state', { record_id: recordId, new_state: state }),
      records,
      results => results.forEach(result => { if (result.status === 'rejected') failures.push(result.reason) }),
    )
    if (failures.length > 0) throw failures[0]
  }

  private async listSession(request: ListSessionRequest): Promise<SessionMessagePage> {
    return await this.call<SessionMessagePage | null>('msg.list_session', { ...request }) ?? {}
  }

  private async readSummaries(maxPages: number): Promise<SessionSummary[]> {
    const summaries: SessionSummary[] = []
    let cursor: { value: number; sessionId: string } | undefined
    for (let page = 0; page < maxPages; page++) {
      const result: SessionSummaryPage = await this.call<SessionSummaryPage | null>('msg.list_sessions', {
        owner: this.options.owner, limit: SESSION_PAGE_SIZE, with_object: true, lifecycle: 'all', order_by: 'activity',
        ...(cursor ? { cursor_updated_at_ms: cursor.value, cursor_session_id: cursor.sessionId } : {}),
      }) ?? {}
      summaries.push(...result.items ?? [])
      if (result.next_cursor_updated_at_ms === undefined || result.next_cursor_session_id === undefined) break
      cursor = { value: result.next_cursor_updated_at_ms, sessionId: result.next_cursor_session_id }
    }
    return summaries
  }

  private readPeople(): Promise<People> {
    if (this.people && this.now() - this.people.at < CACHE_MS) return this.people.value
    const value = Promise.all([
      this.call<Contact[] | null>('contact.list_contacts', { query: {}, contact_mgr_owner: this.options.owner }).catch(() => null),
      readAgentDids(this.options.channel).catch(() => []),
      this.call<GroupsByMember | null>('group.list_by_member', {}).catch(() => null),
    ]).then(([contacts, agentDids, groups]): People => ({
      contacts: contacts ?? [],
      agentDids,
      groups: Object.fromEntries(participatedGroups(groups ?? {}).map(({ did, doc }) => [did, { name: groupName(doc) }])),
    }))
    this.people = { at: this.now(), value }
    return value
  }

  private async loadPrefs(sessionIds: readonly string[]) {
    const stale = sessionIds.filter(sessionId => this.now() - (this.prefs.get(sessionId)?.at ?? -Infinity) >= CACHE_MS)
    for (let offset = 0; offset < stale.length; offset += PARALLEL_READS) {
      await Promise.all(stale.slice(offset, offset + PARALLEL_READS).map(async sessionId => {
        const entries = await this.call<UiSessionStateEntry[] | null>('ui_session.list_state', { session_id: sessionId, owner: this.options.owner }).catch(() => undefined)
        if (entries === undefined) return
        const value = { ...defaultSessionPrefs }
        for (const entry of entries ?? []) applySessionPref(value, entry)
        this.prefs.set(sessionId, { at: this.now(), value })
      }))
    }
  }

  private project(summaries: SessionSummary[], people: People): ProjectedOwner {
    return projectOwner({
      ownerDid: this.options.owner,
      summaries,
      contacts: people.contacts,
      agentDids: people.agentDids,
      personalTitles: {},
      groups: people.groups,
      groupSessionTitles: {},
      policies: {},
      labels: this.options.labels(),
      hiddenAccounts: { dids: [this.options.owner], usernames: [this.options.userId] },
    })
  }
}
