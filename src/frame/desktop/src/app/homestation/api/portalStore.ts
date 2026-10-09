import type { CardView, IdentityView, ProfileView, PublishedPage, WrappedBody } from '../datamodel/types'
import type { Did, ObjId } from '../protocol/feed'
import type { CommentList, HomeStationStore, StoreStatus } from '../store/types'
import { setContentHome, TransportError } from './transport'

// A read-only store for portal pages (`/homestation/<user>`, `/homestation/~zone`, the short
// hosts' `/`): everything goes through `portal.*`, which the service answers with or without a
// session, as the signed-in viewer or as an anonymous visitor (§4.4, §4.6). Writing is the
// viewer's own HomeStation's business, not the portal's.

export interface PortalSource {
  call<T>(method: string, params?: Record<string, unknown>): Promise<T>
}

const PAGE = 10

function hueOf(did: string) {
  let hash = 2166136261
  for (const byte of new TextEncoder().encode(did)) hash = Math.imul(hash ^ byte, 16777619) >>> 0
  return hash % 360
}

function readOnly(): Promise<never> {
  return Promise.reject(new TransportError('protocol', 'homestation.portal.readOnly'))
}

let storeCounter = 0

export function createPortalStore(source: PortalSource, feed: string): HomeStationStore {
  const id = `hs-portal-${++storeCounter}`
  const listeners = new Set<() => void>()
  const cards = new Map<ObjId, CardView | null>()
  const pending = new Set<ObjId>()
  const identities = new Map<Did, IdentityView>()
  let version = 0
  let status: StoreStatus = 'loading'
  let profile: ProfileView | null = null
  let loading: Promise<void> | null = null

  function notify() {
    version += 1
    for (const listener of listeners) listener()
  }

  function putCards(list: (CardView | null | undefined)[] | undefined) {
    for (const card of list ?? []) {
      if (!card) continue
      cards.set(card.item.objId, card)
      const publisher = card.item.publisher
      if (publisher.did) identities.set(publisher.did, publisher)
    }
    notify()
  }

  async function loadProfile() {
    const data = await source.call<ProfileView>('portal.profile', { feed })
    profile = data
    if (data.did) identities.set(data.did, { did: data.did, name: data.name, kind: data.kind === 'zone' ? 'site' : 'person', hue: data.hue })
    return data
  }

  function load() {
    loading ??= loadProfile()
      .then(() => {
        status = 'ready'
      }, () => {
        status = 'error'
      })
      .finally(() => {
        loading = null
        notify()
      })
    return loading
  }

  function requestCard(objId: ObjId) {
    if (pending.has(objId)) return
    pending.add(objId)
    source
      .call<{ card: CardView | null }>('portal.item', { feed, objId })
      .then(result => {
        cards.set(objId, result.card)
        putCards([result.card])
      }, () => undefined)
      .finally(() => pending.delete(objId))
  }

  const store: HomeStationStore = {
    id,
    get owner() {
      return profile?.did ?? ''
    },
    subscribe(listener) {
      listeners.add(listener)
      return () => {
        listeners.delete(listener)
      }
    },
    getVersion: () => version,
    onDomains: () => () => {},
    connect() {
      setContentHome(feed)
      if (status !== 'ready') {
        if (status === 'error') {
          status = 'loading'
          notify()
        }
        void load()
      }
      return () => {}
    },
    watchCard(objId) {
      if (!cards.has(objId)) requestCard(objId)
      return () => {}
    },

    peekStatus: () => status,
    peekCard: objId => cards.get(objId) ?? null,
    peekIdentity: did => identities.get(did) ?? { did, name: did.replace(/^did:[a-z0-9]+:/i, ''), kind: 'person', hue: hueOf(did) },
    peekSettings: () => ({ defaultAudience: { kind: 'public' }, likeNoticeShown: false }),
    peekGroups: () => [],
    peekMuteRules: () => [],
    peekFilterRules: () => [],
    peekTopics: () => [],
    peekSyncStatus: () => ({ candidates: 0, preparing: 0, lastFetchAt: 0, sources: 0, failingSources: 0 }),
    peekHiddenSummary: () => ({ hiddenByRules: 0, hiddenByMute: 0 }),
    peekReadingSummary: () => ({ visible: 0, hiddenByRules: 0, hiddenByMute: 0 }),
    peekEntries: () => [],
    peekCollector: () => null,
    peekPreviewReaders: () => ({}),
    peekMuteCandidates: () => [],
    peekHome: () => null,

    listReading: () => readOnly(),
    listFollowedCandidates: () => readOnly(),
    openCandidate: () => readOnly(),
    async listPublished(_owner, { kind, cursor }) {
      const page = await source.call<PublishedPage & { cards: CardView[] }>('portal.list', { feed, ...(kind ? { kind } : {}), cursor: cursor ?? null, limit: PAGE })
      putCards(page.cards)
      return { entries: page.entries, nextCursor: page.nextCursor, changeCursor: page.changeCursor }
    },
    listChanges: () => Promise.resolve([]),
    async getItem(objId) {
      const result = await source.call<{ card: CardView | null }>('portal.item', { feed, objId })
      cards.set(objId, result.card)
      putCards([result.card])
      return result.card
    },
    getWrappedBody: objId => source.call<WrappedBody>('portal.wrapped_body', { feed, objId }),
    retryResources: () => readOnly(),
    listComments: (objId, { type }) => source.call<CommentList>('portal.comments', { feed, objId, type }),
    listSaved: () => Promise.resolve([]),
    async getProfile() {
      const data = await loadProfile()
      if (data.featured.length) {
        const found = await Promise.all(data.featured.map(objId => source.call<{ card: CardView | null }>('portal.item', { feed, objId }).catch(() => ({ card: null }))))
        putCards(found.map(result => result.card))
      }
      return data
    },

    setLike: () => readOnly(),
    setBookmark: () => readOnly(),
    setReadLater: () => readOnly(),
    setDislike: () => readOnly(),
    repost: () => readOnly(),
    quote: () => readOnly(),
    comment: () => readOnly(),
    withdraw: () => readOnly(),
    editPost: () => readOnly(),
    setAudience: () => readOnly(),
    setZoneListing: () => readOnly(),
    publish: () => readOnly(),
    retryPublish: () => readOnly(),
    retryDelivery: () => readOnly(),
    shareCapture: () => readOnly(),
    uploadAttachment: () => readOnly(),
    fetchLinkPreview: () => readOnly(),

    resolveSourceInput: () => readOnly(),
    follow: () => readOnly(),
    unfollow: () => readOnly(),
    pauseSource: () => readOnly(),
    listSources: () => readOnly(),

    setMuteRule: () => readOnly(),
    setFilterRule: () => readOnly(),
    setTagOverride: () => readOnly(),
    markLessLike: () => readOnly(),
    setFeatured: () => readOnly(),
    setDefaultAudience: () => readOnly(),
  }
  return store
}
