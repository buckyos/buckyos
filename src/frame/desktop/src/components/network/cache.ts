/* ── Thin cache over msg-center results ──
 *
 * Holds the last answer of each list call, shares in-flight loads and reloads what a change
 * event (contact kevent, mailbox events, or an action of this page) touched. It makes no
 * relationship decisions; display rules live in relation.ts.
 */

import { ensureNetworkBackend } from './backend'
import type {
  Contact,
  FriendRequest,
  FriendRequestDecision,
  GroupInvitationItem,
  GroupRef,
  JoinRequestItem,
  LookupKind,
  LookupResult,
  NetworkBackend,
  NetworkChange,
  RequestSession,
} from './types'

export interface ResourceData {
  contacts: Contact[]
  friendRequests: FriendRequest[]
  requestSessions: RequestSession[]
  groups: GroupRef[]
  invitations: GroupInvitationItem[]
  joinRequests: JoinRequestItem[]
}

export type ResourceKey = keyof ResourceData

export interface ResourceState<T> {
  status: 'loading' | 'ready' | 'error'
  data?: T
  error?: unknown
}

const LOADERS: { [K in ResourceKey]: (backend: NetworkBackend) => Promise<ResourceData[K]> } = {
  contacts: (backend) => backend.listContacts(),
  friendRequests: (backend) => backend.listFriendRequests(),
  requestSessions: (backend) => backend.listRequestSessions(),
  groups: (backend) => backend.listGroups(),
  invitations: (backend) => backend.listGroupInvitations(),
  joinRequests: (backend) => backend.listJoinRequests(),
}

const AFFECTED: Record<NetworkChange, ResourceKey[]> = {
  contacts: ['contacts'],
  requests: ['friendRequests'],
  mailbox: ['requestSessions', 'invitations'],
  groups: ['groups', 'joinRequests', 'invitations'],
}

const LOOKUP_TTL_MS = 15_000
const INITIAL: ResourceState<never> = { status: 'loading' }

export class NetworkCache {
  private states = new Map<ResourceKey, ResourceState<unknown>>()
  private inflight = new Map<ResourceKey, Promise<void>>()
  private reloadAfter = new Set<ResourceKey>()
  private lookups = new Map<string, { at: number; promise: Promise<LookupResult> }>()
  private listeners = new Set<() => void>()
  private detach: (() => void) | null = null
  private owner: Promise<string> | null = null
  /** Bumped on every change; lookups of mounted components re-run on it. */
  epoch = 0

  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => { this.listeners.delete(listener) }
  }

  getState = <K extends ResourceKey>(key: K): ResourceState<ResourceData[K]> =>
    (this.states.get(key) as ResourceState<ResourceData[K]> | undefined) ?? INITIAL

  getEpoch = () => this.epoch

  ownerDid(): Promise<string> {
    this.owner ??= this.backend().then(backend => backend.ownerDid()).catch((error) => {
      this.owner = null
      throw error
    })
    return this.owner
  }

  shareText(): Promise<string> {
    return this.backend().then(backend => backend.shareText())
  }

  ensure(key: ResourceKey) {
    const state = this.states.get(key)
    if (state && state.status !== 'error') return
    void this.load(key)
  }

  refresh(key: ResourceKey) {
    void this.load(key)
  }

  /** Raw lookup through msg-center; identical questions within a short window share one answer. */
  lookup(input: string, options: { kinds?: LookupKind[]; limit?: number } = {}): Promise<LookupResult> {
    const key = JSON.stringify([input.trim(), options.kinds ?? [], options.limit ?? 0])
    const cached = this.lookups.get(key)
    if (cached && Date.now() - cached.at < LOOKUP_TTL_MS) return cached.promise
    const promise = this.backend().then(backend => backend.lookup(input.trim(), options))
    promise.catch(() => { if (this.lookups.get(key)?.promise === promise) this.lookups.delete(key) })
    this.lookups.set(key, { at: Date.now(), promise })
    return promise
  }

  /** Runs an action and reloads what it changed, without waiting for the change event. */
  async run<T>(action: (backend: NetworkBackend) => Promise<T>, changes: NetworkChange[]): Promise<T> {
    const backend = await this.backend()
    try {
      return await action(backend)
    } finally {
      if (changes.length > 0) this.changed(changes)
    }
  }

  private async backend() {
    const backend = await ensureNetworkBackend()
    this.detach ??= backend.subscribe(change => this.changed([change]))
    return backend
  }

  private changed(changes: NetworkChange[]) {
    this.lookups.clear()
    this.epoch += 1
    const keys = new Set(changes.flatMap(change => AFFECTED[change]))
    for (const key of keys) if (this.states.has(key)) void this.load(key)
    this.notify()
  }

  private load(key: ResourceKey): Promise<void> {
    const running = this.inflight.get(key)
    if (running) {
      this.reloadAfter.add(key)
      return running
    }
    const previous = this.states.get(key)
    if (!previous || previous.status === 'error') {
      this.states.set(key, { status: 'loading', data: previous?.data })
      this.notify()
    }
    const loader = LOADERS[key] as (backend: NetworkBackend) => Promise<unknown>
    const promise = this.backend()
      .then(backend => loader(backend))
      .then((data) => { this.states.set(key, { status: 'ready', data }) })
      .catch((error) => { this.states.set(key, { status: 'error', error, data: this.states.get(key)?.data }) })
      .finally(() => {
        this.inflight.delete(key)
        this.notify()
        if (this.reloadAfter.delete(key)) void this.load(key)
      })
    this.inflight.set(key, promise)
    return promise
  }

  private notify() {
    this.listeners.forEach(listener => listener())
  }
}

export const networkCache = new NetworkCache()

/** Every relationship action the components offer; each reloads the lists it touches. */
export const networkActions = {
  saveContact: (did: string, name?: string) => networkCache.run(backend => backend.saveContact(did, name), ['contacts']),
  updateTags: (did: string, change: { add?: string[]; remove?: string[] }) => networkCache.run(backend => backend.updateTags(did, change), ['contacts']),
  removeContact: (did: string) => networkCache.run(backend => backend.removeContact(did), ['contacts']),
  blockContact: (did: string) => networkCache.run(backend => backend.blockContact(did), ['contacts', 'requests']),
  unblockContact: (did: string) => networkCache.run(backend => backend.unblockContact(did), ['contacts']),
  allowMessages: (did: string, name?: string) => networkCache.run(backend => backend.allowMessages(did, name), ['contacts']),
  sendFriendRequest: (did: string, message?: string) => networkCache.run(backend => backend.sendFriendRequest(did, message), ['contacts', 'requests']),
  respondFriendRequest: (requestId: string, decision: FriendRequestDecision) => networkCache.run(backend => backend.respondFriendRequest(requestId, decision), ['contacts', 'requests']),
  cancelFriendRequest: (requestId: string) => networkCache.run(backend => backend.cancelFriendRequest(requestId), ['contacts', 'requests']),
  removeFriend: (did: string, notify: boolean) => networkCache.run(backend => backend.removeFriend(did, notify), ['contacts', 'requests']),
  getGroupDoc: (groupDid: string) => networkCache.run(backend => backend.getGroupDoc(groupDid), []),
  requestJoin: (groupDid: string, invite?: string) => networkCache.run(backend => backend.requestJoin(groupDid, invite), ['groups']),
  leaveGroup: (groupDid: string) => networkCache.run(backend => backend.leaveGroup(groupDid), ['groups']),
  acceptGroupInvitation: (invitation: GroupInvitationItem) => networkCache.run(backend => backend.acceptGroupInvitation(invitation), ['groups', 'mailbox']),
  dismissGroupInvitation: (invitation: GroupInvitationItem) => networkCache.run(backend => backend.dismissGroupInvitation(invitation), ['mailbox']),
  approveJoinRequest: (request: JoinRequestItem) => networkCache.run(backend => backend.approveJoinRequest(request), ['groups']),
  rejectJoinRequest: (request: JoinRequestItem) => networkCache.run(backend => backend.rejectJoinRequest(request), ['groups']),
}
