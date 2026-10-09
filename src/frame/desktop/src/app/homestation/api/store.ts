import type { AttachmentInput } from '../datamodel/inputs'
import type {
  AudienceSpec,
  CandidatePage,
  CardView,
  ContactGroup,
  FilterRule,
  HomeInfo,
  IdentityView,
  MuteRule,
  PersonalState,
  ProfileView,
  PublishedPage,
  PublishTask,
  ReaderIdentity,
  ReadingPage,
  ResourceState,
  SavedItem,
  SourceResolution,
  SourceView,
  StreamChange,
  SubscriptionIntent,
  SyncStatus,
  TopicView,
  WrappedBody,
} from '../datamodel/types'
import type { Did, ObjId } from '../protocol/feed'
import type { CommentList, EntryDebugView, HomeStationStore, ReadingSummary, StoreDomain, StoreSettings, StoreStatus } from '../store/types'
import { setContentHome, type Transport, type UploadMeta } from './transport'

const READING_PAGE = 8
const PUBLISHED_PAGE = 10
const POLL_MS = 3000
const OWNER: ReaderIdentity = { kind: 'owner' }
const STORE_DOMAINS: StoreDomain[] = ['reading', 'candidates', 'published', 'comments', 'saved', 'sources', 'prefs', 'profile']
const BOOTSTRAP_DOMAINS: StoreDomain[] = ['reading', 'candidates', 'sources', 'prefs']
const CARD_DOMAINS: StoreDomain[] = ['published', 'comments', 'saved']
const EMPTY_SUMMARY: ReadingSummary = { visible: 0, hiddenByRules: 0, hiddenByMute: 0 }
const EMPTY_SYNC: SyncStatus = { candidates: 0, preparing: 0, lastFetchAt: 0, sources: 0, failingSources: 0 }

interface Bootstrap {
  owner: Did
  user: string
  home?: HomeInfo
  settings: StoreSettings & { muteRules: MuteRule[]; filterRules: FilterRule[]; collectors: { did: Did; name: string }[] }
  groups: ContactGroup[]
  friends: Did[]
  followers: Did[]
  following: { did: Did; name: string }[]
  identities: IdentityView[]
  topics: TopicView[]
  syncStatus: SyncStatus
  hiddenSummary: { hiddenByRules: number; hiddenByMute: number }
  versions: Record<string, number>
}

interface Lazy<T> {
  value?: T
  stale: boolean
  pending: boolean
}

function readerKey(reader: ReaderIdentity = OWNER) {
  return reader.kind === 'did' ? `did:${reader.did}` : reader.kind
}

function hueOf(did: string) {
  let hash = 2166136261
  for (const byte of new TextEncoder().encode(did)) hash = Math.imul(hash ^ byte, 16777619) >>> 0
  return hash % 360
}

function loadMedia<T>(file: File, tag: 'img' | 'video' | 'audio', read: (element: HTMLImageElement | HTMLMediaElement) => T): Promise<T | undefined> {
  const url = URL.createObjectURL(file)
  return new Promise<T | undefined>(resolve => {
    const element = document.createElement(tag)
    const done = (value: T | undefined) => {
      URL.revokeObjectURL(url)
      resolve(value)
    }
    element.addEventListener(tag === 'img' ? 'load' : 'loadedmetadata', () => done(read(element)))
    element.addEventListener('error', () => done(undefined))
    if (element instanceof HTMLMediaElement) element.preload = 'metadata'
    element.src = url
  })
}

async function uploadMeta(file: File, kind: AttachmentInput['kind']): Promise<UploadMeta> {
  const meta: UploadMeta = { name: file.name, mime: file.type || 'application/octet-stream' }
  if (kind === 'image') {
    const size = await loadMedia(file, 'img', element => ({ width: (element as HTMLImageElement).naturalWidth, height: (element as HTMLImageElement).naturalHeight }))
    return size && size.width > 0 ? { ...meta, ...size } : meta
  }
  const duration = await loadMedia(file, kind, element => (element as HTMLMediaElement).duration)
  return duration && Number.isFinite(duration) ? { ...meta, duration_ms: duration * 1000 } : meta
}

let storeCounter = 0

export function createApiHomeStationStore(transport: Transport): HomeStationStore {
  const id = `hs-api-${++storeCounter}`
  const listeners = new Set<() => void>()
  const domainListeners = new Set<{ domains: StoreDomain[]; listener: () => void }>()
  const cards = new Map<string, CardView | null>()
  const cardGen = new Map<string, number>()
  const queued = new Map<string, { objId: ObjId; reader: ReaderIdentity }>()
  const watched = new Map<string, { objId: ObjId; reader: ReaderIdentity; count: number }>()
  const summaries = new Map<string, Lazy<ReadingSummary>>()
  const entries: Lazy<EntryDebugView[]> = { stale: true, pending: false }
  let version = 0
  let status: StoreStatus = 'loading'
  let boot: Bootstrap | null = null
  let identities = new Map<Did, IdentityView>()
  let versions: Record<string, number> | null = null
  let booting: Promise<void> | null = null
  let bootAgain = false
  let connections = 0
  let pollTimer = 0
  let polling = false
  let flushTimer = 0

  const call = <T>(method: string, params: Record<string, unknown> = {}) => transport.call<T>(method, params)

  function notify() {
    version += 1
    for (const listener of listeners) listener()
  }

  function emit(domains: StoreDomain[]) {
    notify()
    for (const entry of domainListeners) {
      if (entry.domains.some(domain => domains.includes(domain))) entry.listener()
    }
  }

  function cardKey(objId: ObjId, reader?: ReaderIdentity) {
    return `${readerKey(reader)}|${objId}`
  }

  function putCards(list: CardView[] | undefined, reader?: ReaderIdentity) {
    if (!list?.length) return
    for (const card of list) cards.set(cardKey(card.item.objId, reader), card)
    notify()
  }

  function flushCards() {
    flushTimer = 0
    const groups = new Map<string, { reader: ReaderIdentity; items: { key: string; objId: ObjId; gen: number }[] }>()
    for (const [key, { objId, reader }] of queued) {
      const gen = (cardGen.get(key) ?? 0) + 1
      cardGen.set(key, gen)
      const group = groups.get(readerKey(reader)) ?? { reader, items: [] }
      group.items.push({ key, objId, gen })
      groups.set(readerKey(reader), group)
    }
    queued.clear()
    for (const { reader, items } of groups.values()) {
      call<CardView[]>('item.cards', { objIds: items.map(item => item.objId), reader }).then(list => {
        const found = new Map(list.map(card => [card.item.objId, card]))
        for (const item of items) {
          if (cardGen.get(item.key) === item.gen) cards.set(item.key, found.get(item.objId) ?? null)
        }
        notify()
      }, () => undefined)
    }
  }

  function requestCard(objId: ObjId, reader: ReaderIdentity = OWNER) {
    queued.set(cardKey(objId, reader), { objId, reader })
    if (!flushTimer) flushTimer = window.setTimeout(flushCards, 0)
  }

  function patchPersonal(objId: ObjId, personal: PersonalState) {
    const key = cardKey(objId)
    const card = cards.get(key)
    if (card) cards.set(key, { ...card, personal })
  }

  function loadBootstrap(): Promise<void> {
    if (booting) {
      bootAgain = true
      return booting
    }
    booting = (async () => {
      try {
        do {
          bootAgain = false
          const data = await call<Bootstrap>('ui.bootstrap')
          boot = data
          setContentHome(data.user)
          identities = new Map(data.identities.map(identity => [identity.did ?? '', identity]))
          versions ??= data.versions
          status = 'ready'
          notify()
        } while (bootAgain)
      } catch {
        if (!boot) {
          status = 'error'
          notify()
        }
      } finally {
        booting = null
      }
    })()
    return booting
  }

  function lazy<T>(entry: Lazy<T>, load: () => Promise<T>) {
    if (!entry.stale || entry.pending) return
    entry.stale = false
    entry.pending = true
    load()
      .then(value => {
        entry.value = value
      }, () => undefined)
      .finally(() => {
        entry.pending = false
        notify()
      })
  }

  function invalidate(domains: StoreDomain[], bootstrapped = false) {
    if (!bootstrapped && domains.some(domain => BOOTSTRAP_DOMAINS.includes(domain))) void loadBootstrap()
    if (domains.includes('reading') || domains.includes('prefs')) {
      for (const summary of summaries.values()) summary.stale = true
    }
    if (domains.includes('published')) entries.stale = true
    if (domains.some(domain => CARD_DOMAINS.includes(domain))) {
      for (const { objId, reader } of watched.values()) if (reader.kind === 'owner') requestCard(objId)
    }
    emit(domains)
  }

  async function written(domains: StoreDomain[], objIds: ObjId[] = []) {
    for (const objId of objIds) requestCard(objId)
    const bootstrapped = domains.includes('prefs') || domains.includes('sources')
    if (bootstrapped) await loadBootstrap()
    invalidate(domains, bootstrapped)
  }

  async function poll() {
    if (polling || !versions) return
    polling = true
    const next = await call<Record<string, number>>('ui.versions').catch(() => null)
    polling = false
    if (!next || !versions) return
    const previous = versions
    versions = next
    const changed = new Set<StoreDomain>()
    for (const [domain, value] of Object.entries(next)) {
      if (previous[domain] === value) continue
      if (domain === 'evaluation') changed.add('reading')
      else if ((STORE_DOMAINS as string[]).includes(domain)) changed.add(domain as StoreDomain)
    }
    if (changed.size > 0) invalidate([...changed])
  }

  async function settingsWrite(patch: (settings: Bootstrap['settings']) => Bootstrap['settings'], method: string, params: Record<string, unknown>, domains: StoreDomain[]) {
    if (boot) {
      boot = { ...boot, settings: patch(boot.settings) }
      notify()
    }
    try {
      await call(method, params)
    } finally {
      await written(domains)
    }
  }

  function startPolling() {
    if (pollTimer || document.hidden) return
    pollTimer = window.setInterval(() => void poll(), POLL_MS)
  }

  function stopPolling() {
    window.clearInterval(pollTimer)
    pollTimer = 0
  }

  function onVisibility() {
    if (document.hidden) {
      stopPolling()
      return
    }
    void poll()
    startPolling()
  }

  async function personalWrite(method: string, objId: ObjId, params: Record<string, unknown>, domains: StoreDomain[]) {
    const personal = await call<PersonalState>(method, { objId, ...params })
    patchPersonal(objId, personal)
    await written(domains, [objId])
    return personal
  }

  const store: HomeStationStore = {
    id,
    get owner() {
      return boot?.owner ?? ''
    },
    subscribe(listener) {
      listeners.add(listener)
      return () => {
        listeners.delete(listener)
      }
    },
    getVersion: () => version,
    onDomains(domains, listener) {
      const entry = { domains, listener }
      domainListeners.add(entry)
      return () => {
        domainListeners.delete(entry)
      }
    },
    connect() {
      connections += 1
      if (connections === 1) {
        if (status === 'error') {
          status = 'loading'
          notify()
        }
        if (status !== 'ready' && !booting) void loadBootstrap()
        startPolling()
        document.addEventListener('visibilitychange', onVisibility)
      }
      let open = true
      return () => {
        if (!open) return
        open = false
        connections -= 1
        if (connections === 0) {
          stopPolling()
          document.removeEventListener('visibilitychange', onVisibility)
        }
      }
    },
    watchCard(objId, reader) {
      const key = cardKey(objId, reader)
      const entry = watched.get(key)
      if (entry) entry.count += 1
      else watched.set(key, { objId, reader, count: 1 })
      if (!cards.has(key)) requestCard(objId, reader)
      return () => {
        const current = watched.get(key)
        if (!current) return
        current.count -= 1
        if (current.count <= 0) watched.delete(key)
      }
    },

    peekStatus: () => status,
    peekCard: (objId, reader) => cards.get(cardKey(objId, reader)) ?? null,
    peekIdentity: did => identities.get(did) ?? { did, name: did.replace(/^did:[a-z0-9]+:/i, ''), kind: 'person', hue: hueOf(did) },
    peekSettings: () => ({ defaultAudience: boot?.settings.defaultAudience ?? { kind: 'public' }, likeNoticeShown: boot?.settings.likeNoticeShown ?? false }),
    peekGroups: () => boot?.groups ?? [],
    peekMuteRules: () => boot?.settings.muteRules ?? [],
    peekFilterRules: () => boot?.settings.filterRules ?? [],
    peekTopics: () => boot?.topics ?? [],
    peekSyncStatus: () => boot?.syncStatus ?? EMPTY_SYNC,
    peekHiddenSummary: () => boot?.hiddenSummary ?? { hiddenByRules: 0, hiddenByMute: 0 },
    peekReadingSummary(query) {
      const key = JSON.stringify([query.filter, query.topicId, query.search.trim().toLowerCase()])
      let entry = summaries.get(key)
      if (!entry) {
        entry = { stale: true, pending: false }
        summaries.set(key, entry)
        if (summaries.size > 20) summaries.delete(summaries.keys().next().value!)
      }
      const target = entry
      lazy(target, () => call<ReadingSummary>('reading.summary', { query: { ...query, showFiltered: false } }))
      return target.value ?? EMPTY_SUMMARY
    },
    peekEntries() {
      lazy(entries, () => call<EntryDebugView[]>('published.entries'))
      return entries.value ?? []
    },
    peekCollector() {
      const collector = boot?.settings.collectors[0]
      return collector ? { id: collector.did, name: collector.name } : null
    },
    peekPreviewReaders() {
      const friends = boot?.friends ?? []
      const followers = boot?.followers ?? []
      return { follower: followers.find(did => !friends.includes(did)) ?? followers[0], friend: friends[0] }
    },
    peekMuteCandidates() {
      if (!boot) return []
      const people = boot.identities.filter(identity => identity.kind === 'person' && identity.did).map(identity => identity.did!)
      return [...new Set([...boot.friends, ...boot.following.map(entry => entry.did), ...people])].filter(did => did !== boot?.owner)
    },
    peekHome: () => boot?.home ?? null,

    async listReading(query, cursor) {
      const page = await call<ReadingPage & { cards: CardView[] }>('reading.list', { query, cursor: cursor ?? null, limit: READING_PAGE })
      putCards(page.cards)
      return { objIds: page.objIds, nextCursor: page.nextCursor, hiddenByRules: page.hiddenByRules, hiddenByMute: page.hiddenByMute, total: page.total }
    },
    async listFollowedCandidates(cursor, options = {}) {
      const page = await call<CandidatePage & { cards: CardView[] }>('candidates.list', { cursor: cursor ?? null, includeRead: !!options.includeRead, limit: READING_PAGE })
      putCards(page.cards)
      return { entries: page.entries, nextCursor: page.nextCursor, readCount: page.readCount, retentionDays: page.retentionDays, lastFetchAt: page.lastFetchAt }
    },
    async openCandidate(objId) {
      const result = await call<{ ok: boolean; resources: ResourceState }>('candidates.open', { objId })
      await written(['candidates'], [objId])
      return { ok: result.ok }
    },
    async listPublished(owner, { reader, kind, cursor }) {
      const page = await call<PublishedPage & { cards: CardView[] }>('published.list', { owner, reader, ...(kind ? { kind } : {}), cursor: cursor ?? null, limit: PUBLISHED_PAGE })
      putCards(page.cards, reader)
      return { entries: page.entries, nextCursor: page.nextCursor, changeCursor: page.changeCursor, readerApproximated: page.readerApproximated }
    },
    listChanges: (_owner, { reader, after }) => call<StreamChange[]>('published.changes', { reader, after }),
    async getItem(objId, reader = OWNER) {
      const card = await call<CardView | null>('item.get', { objId, reader })
      cards.set(cardKey(objId, reader), card)
      notify()
      return card
    },
    getWrappedBody: objId => call<WrappedBody>('item.wrapped_body', { objId }),
    async retryResources(objId) {
      const state = await call<ResourceState>('item.retry_resources', { objId })
      await written(['reading', 'candidates'], [objId])
      return state
    },
    listComments: (objId, { view, type }) => call<CommentList>('comments.list', { objId, view, type }),
    async listSaved(kind) {
      const rows = await call<SavedItem[]>('saved.list', { kind })
      if (rows.length > 0) putCards(await call<CardView[]>('item.cards', { objIds: rows.map(row => row.objId) }))
      return rows
    },
    async getProfile(did, reader) {
      const profile = await call<ProfileView | null>('profile.get', { did, reader })
      if (profile?.featured.length) putCards(await call<CardView[]>('item.cards', { objIds: profile.featured, reader }), reader)
      return profile
    },

    async setLike(objId, on) {
      const personal = await personalWrite('interact.like', objId, { on }, ['published', 'comments'])
      if (on && boot && personal.like.visibility === 'public') boot.settings.likeNoticeShown = true
      return personal
    },
    setBookmark: (objId, options) => personalWrite('interact.bookmark', objId, { on: options.on, public: options.public }, ['saved', 'published', 'comments']),
    setReadLater: (objId, on) => personalWrite('interact.read_later', objId, { on }, ['saved']),
    setDislike: (objId, on) => personalWrite('interact.dislike', objId, { on }, ['saved']),
    repost: (objId, on) => personalWrite('interact.repost', objId, { on }, ['published', 'comments', 'profile']),
    async quote(objId, text, audience) {
      const task = await call<PublishTask>('interact.quote', { objId, text, audience })
      await written(['published', 'comments', 'profile'], [objId])
      return task
    },
    async comment(objId, text) {
      const result = await call<{ task: PublishTask; audience: AudienceSpec }>('interact.comment', { objId, text })
      await written(['published', 'comments', 'profile'], [objId])
      return result
    },
    async withdraw(entry) {
      await call('entry.withdraw', { entry })
      await written(['published', 'profile', 'comments', 'reading', 'saved'])
    },
    async editPost(entry, text) {
      const objId = await call<ObjId | null>('entry.edit', { entry, text })
      await written(['published', 'profile', 'comments', 'reading'])
      return objId
    },
    async setAudience(entry, audience) {
      await call('entry.set_audience', { entry, audience })
      await written(['published', 'profile'])
    },
    async setZoneListing(entry, listed) {
      await call('zone.set_listing', { entry, listed })
      await written(['published'])
    },
    async publish(input, key) {
      const task = await call<PublishTask>('publish.create', { key, input })
      await written(['published', 'profile', 'reading'])
      return task
    },
    async retryPublish(key) {
      const task = await call<PublishTask>('publish.retry', { key })
      await written(['published', 'profile', 'reading'])
      return task
    },
    async retryDelivery(entry) {
      await call('entry.retry_delivery', { entry })
      await written(['published'])
    },
    async shareCapture(objId) {
      const task = await call<PublishTask>('publish.share_capture', { objId })
      await written(['published', 'profile', 'reading'], [objId])
      return task
    },
    async uploadAttachment(file, kind) {
      const uploaded = await transport.upload(file, await uploadMeta(file, kind))
      return uploaded.objId
    },
    fetchLinkPreview: url => call<{ title: string; summary: string }>('publish.link_preview', { url }),

    resolveSourceInput: (kind, text) => call<SourceResolution>('sources.resolve', { kind, text }),
    async follow(resolution) {
      const added = await call<SourceView[]>('sources.follow', { resolution })
      await written(['sources'])
      return added
    },
    async unfollow(sourceId) {
      const result = await call<'removed' | 'friend_basis'>('sources.unfollow', { sourceId })
      await written(['sources', 'candidates'])
      return result
    },
    async pauseSource(sourceId, paused) {
      await call('sources.pause', { sourceId, paused })
      await written(['sources'])
    },
    listSources: () => call<{ sources: SourceView[]; intents: SubscriptionIntent[] }>('sources.list'),

    setMuteRule(rule, on) {
      const same = (candidate: MuteRule) => (candidate.kind === 'person' && rule.kind === 'person' && candidate.did === rule.did) || (candidate.kind === 'group' && rule.kind === 'group' && candidate.groupId === rule.groupId)
      return settingsWrite(settings => ({ ...settings, muteRules: [...settings.muteRules.filter(candidate => !same(candidate)), ...(on ? [rule] : [])] }), 'prefs.set_mute_rule', { rule, on }, ['prefs', 'reading', 'candidates'])
    },
    setFilterRule(rule) {
      return settingsWrite(settings => ({ ...settings, filterRules: settings.filterRules.some(candidate => candidate.id === rule.id) ? settings.filterRules.map(candidate => (candidate.id === rule.id ? rule : candidate)) : [...settings.filterRules, rule] }), 'prefs.set_filter_rule', { rule }, ['prefs', 'reading', 'candidates'])
    },
    async setTagOverride(objId, tag, override) {
      await call('prefs.set_tag_override', { objId, tag, override })
      await written(['prefs', 'reading', 'candidates'], [objId])
    },
    async markLessLike(objId) {
      await call('prefs.mark_less_like', { objId })
    },
    async setFeatured(order) {
      await call('profile.set_featured', { order })
      await written(['profile'])
    },
    setDefaultAudience(audience) {
      return settingsWrite(settings => ({ ...settings, defaultAudience: audience }), 'prefs.set_default_audience', { audience }, ['prefs'])
    },
  }
  return store
}
