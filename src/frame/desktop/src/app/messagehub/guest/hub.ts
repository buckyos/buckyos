/**
 * State of the guest page for one group (and one of its Sessions): which
 * step the visitor is at (invited, in the Session, choosing one, refused),
 * the Session timeline polled from the host group store, and own sends.
 * Everything here belongs to one subject; the page builds a new hub for
 * another group, Session or identity.
 */
import { identityKey, type AccessDenial, type CurrentIdentity } from 'buckyos'
import { denialOf } from '../datamodel/sessionApi'
import { ObjectRouteError, setObjectViewer } from '../api/objects'
import { resetObjectCaches } from '../conversation/history/objectAccess'
import { SessionApiReader } from '../api/reader'
import { InMemoryConversationMessageReader } from '../conversation/history/data-source'
import { friendlyDidName } from '../api/projection'
import type { ConversationMessageReader } from '../conversation/history/types'
import type { OutgoingPayload } from '../store/types'
import { randomMsgNonce, type MessageObject, type MsgObject } from '../protocol/msgobj'
import type { GuestApi } from './api'
import { guestEntryAllows, guestTimeline, guestTimelineMessage, type GuestMessageRef, type GuestSession } from './model'

const POLL_MS = 4_000
const MEMBERS_POLL_MS = 30_000
const PAGE_SIZE = 200
/** Pages one refresh reads before the next poll continues. */
const MAX_PAGES = 10
const FETCH_CONCURRENCY = 6

export type GuestStep =
  | { kind: 'loading' }
  /** No browser session: the page sends the visitor to the group's entry. */
  | { kind: 'anonymous' }
  | { kind: 'invited'; joining: boolean; failed: boolean; denial: AccessDenial | null }
  | { kind: 'lobby'; sessions: GuestSession[]; entryOpen: boolean; starting: boolean; failed: boolean; denial: AccessDenial | null }
  | { kind: 'session'; session: GuestSession }
  /** Refused, or the access ended (removed, banned, grant revoked): nothing of the Session stays on the page. */
  | { kind: 'denied'; denial: AccessDenial | null }
  /** The browser session now speaks for someone else (or nobody): the page starts over. */
  | { kind: 'identity_changed' }
  | { kind: 'error' }

export interface GuestMember {
  did: string
  name: string
  kind: 'group_member' | 'guest'
  role?: string
}

export interface GuestSnapshot {
  step: GuestStep
  identity: CurrentIdentity | null
  groupName: string
  reader: ConversationMessageReader
  /** The Session timeline has been read once. */
  historyLoaded: boolean
  members: GuestMember[]
  /** The last refresh did not reach the host; it is retried. */
  offline: boolean
}

type Mapped<T> = { ok: true; value: T } | { ok: false; error: unknown }

async function mapLimit<T, R>(items: readonly T[], limit: number, run: (item: T) => Promise<R>): Promise<Mapped<R>[]> {
  const results: Mapped<R>[] = new Array(items.length)
  let next = 0
  const worker = async () => {
    while (next < items.length) {
      const index = next++
      try { results[index] = { ok: true, value: await run(items[index]) } } catch (error) { results[index] = { ok: false, error } }
    }
  }
  await Promise.all(Array.from({ length: Math.min(limit, items.length) }, worker))
  return results
}

/** The host has no such object for this visitor (gone, redacted, outside the visible history): skipped, not retried. */
const isFinalObjectError = (error: unknown) => error instanceof ObjectRouteError && error.status === 404

export class GuestHub {
  private listeners = new Set<() => void>()
  private snapshot: GuestSnapshot
  private identity: CurrentIdentity | null = null
  private stored = new Map<string, MessageObject>()
  private skipped = new Set<string>()
  private pending = new Map<string, MessageObject>()
  private sendKeys = new Map<string, string>()
  private memberMap = new Map<string, GuestMember>()
  private nextAfterSeq = 0
  private readSeq = 0
  private revision = 0
  private acceptKey?: string
  private requestId?: string
  private stopped = false
  private refreshing?: Promise<void>
  private pollTimer?: ReturnType<typeof setTimeout>
  private membersTimer?: ReturnType<typeof setInterval>
  private membersLoading?: Promise<void>

  private readonly api: GuestApi
  readonly groupDid: string
  readonly sessionId: string | null
  private readonly youLabel: string

  constructor(api: GuestApi, groupDid: string, sessionId: string | null, youLabel: string) {
    this.api = api
    this.groupDid = groupDid
    this.sessionId = sessionId
    this.youLabel = youLabel
    this.snapshot = { step: { kind: 'loading' }, identity: null, groupName: '', reader: InMemoryConversationMessageReader.empty(), historyLoaded: false, members: [], offline: false }
  }

  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener) } }
  getSnapshot = () => this.snapshot

  private publish(patch: Partial<GuestSnapshot>) {
    this.snapshot = { ...this.snapshot, ...patch }
    this.listeners.forEach(listener => listener())
  }

  private get step() { return this.snapshot.step }

  /** The visitor's DID: Account DIDs come from the account's OwnerDocument, External ones are the session subject. */
  get selfDid(): string {
    return this.identity && this.identity.kind !== 'anonymous' ? this.identity.did : ''
  }

  get sessionAddress(): string {
    return this.step.kind === 'session' ? this.step.session.address : ''
  }

  /** Runs once; the page builds a new hub to start over. */
  async start() {
    try {
      const identity = await this.api.identity()
      if (this.stopped) return
      this.identity = identity
      if (identity.kind === 'anonymous') { this.publish({ identity, step: { kind: 'anonymous' } }); return }
      setObjectViewer(identityKey(identity))
      this.publish({ identity })
      void this.api.groupName(this.groupDid).then(groupName => { if (!this.stopped) this.publish({ groupName }) }, () => undefined)
      if (this.sessionId) await this.openSession()
      else await this.openLobby()
    } catch {
      if (!this.stopped) this.publish({ step: { kind: 'error' } })
    }
  }

  stop() {
    this.stopped = true
    clearTimeout(this.pollTimer)
    clearInterval(this.membersTimer)
  }

  /** The Session the page names: the visitor reads it, or is asked to accept the invitation first. */
  private async openSession() {
    const sessionId = this.sessionId!
    let sessions: GuestSession[] | null = null
    let refused: AccessDenial | null = null
    try {
      sessions = await this.api.sessions(this.groupDid)
    } catch (error) {
      refused = denialOf(error)
    }
    if (this.stopped) return
    const session = sessions?.find(item => item.sessionId === sessionId)
    if (!session) {
      // A guest who is only invited has no Session to read yet; the host lists the invitation instead.
      const invited = await this.api.invited(this.groupDid, sessionId).catch(() => refused === null)
      if (this.stopped) return
      if (invited) { this.publish({ step: { kind: 'invited', joining: false, failed: false, denial: null } }); return }
      if (refused) { this.deny(refused); return }
      this.publish({ step: { kind: 'invited', joining: false, failed: false, denial: null } })
      return
    }
    this.publish({ step: { kind: 'session', session } })
    this.membersTimer = setInterval(() => { void this.refreshMembers() }, MEMBERS_POLL_MS)
    void this.refreshMembers()
    await this.poll()
  }

  private async openLobby() {
    const [sessions, entry] = await Promise.all([
      this.api.sessions(this.groupDid).then(value => ({ value }), error => ({ error })),
      this.api.entry(this.groupDid).catch(() => null),
    ])
    if (this.stopped) return
    const entryOpen = guestEntryAllows(entry, this.identity && this.identity.kind !== 'anonymous' ? this.identity.auth_method : undefined)
    if ('error' in sessions) {
      const denial = denialOf(sessions.error)
      if (!entryOpen) { this.deny(denial); return }
      this.publish({ step: { kind: 'lobby', sessions: [], entryOpen, starting: false, failed: false, denial: null } })
      return
    }
    if (sessions.value.length === 0 && !entryOpen) { this.deny(null); return }
    this.publish({ step: { kind: 'lobby', sessions: sessions.value, entryOpen, starting: false, failed: false, denial: null } })
  }

  /**
   * Accepts the Session invitation. The idempotency key stays for this page,
   * so retrying after a lost answer returns the original result; a retry the
   * host no longer recognises still finds the visitor listed in the Session.
   */
  async accept() {
    const step = this.step
    if (step.kind !== 'invited' || step.joining || !this.sessionId) return
    this.publish({ step: { ...step, joining: true, failed: false, denial: null } })
    this.acceptKey ??= crypto.randomUUID()
    try {
      await this.api.accept(this.groupDid, this.sessionId, this.acceptKey)
    } catch (error) {
      const listed = await this.api.sessions(this.groupDid).then(sessions => sessions.some(item => item.sessionId === this.sessionId), () => false)
      if (this.stopped) return
      if (!listed) { this.publish({ step: { kind: 'invited', joining: false, failed: true, denial: denialOf(error) } }); return }
    }
    if (this.stopped) return
    await this.openSession()
  }

  /** `group.submit_guest_request` through the public entry; resolves to the new Session id. */
  async startConversation(): Promise<string | null> {
    const step = this.step
    if (step.kind !== 'lobby' || step.starting || !step.entryOpen) return null
    this.publish({ step: { ...step, starting: true, failed: false, denial: null } })
    this.requestId ??= crypto.randomUUID()
    try {
      const sessionId = await this.api.request(this.groupDid, this.requestId)
      this.requestId = undefined
      return sessionId
    } catch (error) {
      const current = this.step
      if (current.kind === 'lobby') this.publish({ step: { ...current, starting: false, failed: true, denial: denialOf(error) } })
      return null
    }
  }

  /** The access ended or was refused: the Session's content leaves the page. */
  private deny(denial: AccessDenial | null) {
    this.leave({ kind: 'denied', denial })
  }

  /** Stops and drops everything read for this subject. */
  private leave(step: GuestStep) {
    this.stop()
    this.stored.clear()
    this.pending.clear()
    this.memberMap.clear()
    this.revision++
    this.publish({ step, reader: this.buildReader(), members: [], historyLoaded: false })
  }

  /** Whether the browser session still speaks for the subject this hub was started for. */
  private async sameSubject(): Promise<boolean> {
    const current = await this.api.identity()
    if (this.stopped || !this.identity || identityKey(current) === identityKey(this.identity)) return true
    resetObjectCaches()
    this.leave({ kind: 'identity_changed' })
    return false
  }

  /** Reads the Session again now (the page became visible, the network came back). */
  sync() {
    if (this.stopped || this.step.kind !== 'session') return Promise.resolve()
    clearTimeout(this.pollTimer)
    return this.poll()
  }

  private schedule() {
    clearTimeout(this.pollTimer)
    if (!this.stopped) this.pollTimer = setTimeout(() => { void this.poll() }, POLL_MS)
  }

  private async poll() {
    if (this.stopped || this.step.kind !== 'session') return
    try {
      if (!await this.sameSubject()) return
      await this.refresh()
      if (this.snapshot.offline) this.publish({ offline: false })
    } catch (error) {
      if (this.stopped) return
      const denial = denialOf(error)
      if (denial) { this.deny(denial); return }
      // Removed from the Session, or the Session is gone: it is no longer listed.
      const listed = await this.api.sessions(this.groupDid).then(sessions => sessions.some(item => item.sessionId === this.sessionId), error => denialOf(error) ? false : null)
      if (this.stopped) return
      if (listed === false) { this.deny(null); return }
      if (!this.snapshot.offline) this.publish({ offline: true })
    }
    this.schedule()
  }

  /** Reads the messages after the last one known, in host order. */
  private refresh(): Promise<void> {
    this.refreshing ??= this.readNewMessages().finally(() => { this.refreshing = undefined })
    return this.refreshing
  }

  private async readNewMessages() {
    const sessionId = this.sessionId!, address = this.sessionAddress
    let changed = false
    for (let page = 0; page < MAX_PAGES; page++) {
      const result = await this.api.messages(this.groupDid, sessionId, this.nextAfterSeq, PAGE_SIZE)
      if (this.stopped) return
      const wanted = result.items.filter(item => !item.redacted && !this.stored.has(item.objId) && !this.skipped.has(item.objId))
      const bodies = await mapLimit(wanted, FETCH_CONCURRENCY, item => this.api.message(address, item.objId))
      if (this.stopped) return
      let blocked: { ref: GuestMessageRef; error: unknown } | undefined
      for (const [index, item] of wanted.entries()) {
        const body = bodies[index]
        if (body.ok) { this.stored.set(item.objId, guestTimelineMessage(body.value, item, address)); changed = true }
        else if (isFinalObjectError(body.error)) this.skipped.add(item.objId)
        else if (!blocked || item.seq < blocked.ref.seq) blocked = { ref: item, error: body.error }
      }
      if (blocked) {
        // The next poll resumes at the first message that could not be read for now.
        this.nextAfterSeq = Math.max(this.nextAfterSeq, blocked.ref.seq - 1)
        if (changed) this.rebuild(true)
        throw blocked.error
      }
      this.nextAfterSeq = Math.max(this.nextAfterSeq, result.nextAfterSeq)
      if (!result.limited) break
    }
    const firstLoad = !this.snapshot.historyLoaded
    if (changed || firstLoad) this.rebuild(true)
    const unknownSender = [...this.stored.values()].some(message => !this.memberMap.has(message.from))
    if (changed && unknownSender) void this.refreshMembers()
    if (this.nextAfterSeq > this.readSeq && (typeof document === 'undefined' || document.visibilityState === 'visible')) {
      this.readSeq = this.nextAfterSeq
      void this.api.markRead(this.groupDid, sessionId, this.readSeq).catch(() => undefined)
    }
  }

  private refreshMembers(): Promise<void> {
    if (!this.sessionId || this.stopped) return Promise.resolve()
    this.membersLoading ??= this.api.members(this.groupDid, this.sessionId).then(items => {
      if (this.stopped) return
      this.memberMap = new Map(items.filter(item => item.state === 'included').map(item => [item.member_did, {
        did: item.member_did,
        name: friendlyDidName(item.member_did, item.entity_kind === 'agent'),
        kind: item.kind,
        ...(item.role ? { role: item.role } : {}),
      }]))
      this.rebuild()
    }, () => undefined).finally(() => { this.membersLoading = undefined })
    return this.membersLoading
  }

  nameOf = (did: string): string => {
    if (did === this.selfDid) return this.youLabel
    return this.memberMap.get(did)?.name ?? friendlyDidName(did, false)
  }

  private buildReader(): ConversationMessageReader {
    const messages = guestTimeline(this.stored.values(), this.pending.values(), this.nameOf)
    return new SessionApiReader(`guest:${this.identity && this.identity.kind !== 'anonymous' ? identityKey(this.identity) : ''}:${this.sessionAddress}`, { messages, hasOlder: false, revision: this.revision, loaded: true })
  }

  private rebuild(loaded = false) {
    this.revision++
    this.publish({ reader: this.buildReader(), members: [...this.memberMap.values()], ...(loaded ? { historyLoaded: true } : {}) })
  }

  /**
   * Posts a group message to the open Session, `from` the visitor's DID.
   * The same text retried reuses its idempotency key, so an unknown outcome
   * never produces a second message.
   */
  async send(payload: OutgoingPayload) {
    const step = this.step
    if (step.kind !== 'session' || !this.selfDid) throw new Error('permission_denied')
    if (payload.attachments.length > 0) throw new Error('rejected: attachments-not-supported')
    const content = payload.content.trim()
    const message: MsgObject = {
      from: this.selfDid,
      to: [this.groupDid],
      kind: 'group_msg',
      to_session: step.session.sessionId,
      ...(payload.relatesTo ? { relates_to: payload.relatesTo } : {}),
      ...(payload.mentions && (payload.mentions.all || payload.mentions.dids?.length) ? { mentions: payload.mentions } : {}),
      created_at_ms: payload.submittedAt ?? Date.now(),
      nonce: randomMsgNonce(),
      content: { format: 'text/plain', content, ...(payload.machine ? { machine: payload.machine } : {}) },
    }
    const signature = JSON.stringify([content, payload.relatesTo ?? null, payload.mentions ?? null, payload.machine ?? null])
    const key = this.sendKeys.get(signature) ?? crypto.randomUUID()
    this.sendKeys.set(signature, key)
    const optimistic: MessageObject = { ...message, ui_message_id: `local:${key}`, ui_session_id: step.session.address, ui_delivery_status: 'sending' }
    this.pending.set(key, optimistic)
    this.rebuild()
    let result
    try {
      result = await this.api.send(message, key)
    } catch (error) {
      this.pending.delete(key)
      this.rebuild()
      const denial = denialOf(error)
      if (denial) { this.deny(denial); throw new Error('permission_denied') }
      throw new Error(`result_unknown: ${error instanceof Error ? error.message : String(error)}`)
    }
    this.sendKeys.delete(signature)
    if (!result.ok) {
      this.pending.delete(key)
      this.rebuild()
      throw new Error(`rejected: ${result.reason ?? 'unknown'}`)
    }
    this.pending.set(key, { ...optimistic, ui_sent_msg_id: result.msgId })
    this.rebuild()
    await this.refresh().catch(() => undefined)
    for (const [id, item] of this.pending) if (typeof item.ui_sent_msg_id === 'string' && this.stored.has(item.ui_sent_msg_id)) this.pending.delete(id)
    this.rebuild()
  }
}
