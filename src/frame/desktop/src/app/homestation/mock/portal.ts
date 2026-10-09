import type { PortalSource } from '../api/portalStore'
import type { CardView, PortalHome, PublishedPage, ReaderIdentity } from '../datamodel/types'
import type { EntryUrl } from '../protocol/feed'
import type { HomeStationStore } from '../store/types'
import { OWNER_DID, userOf, zoneOf } from './data'
import { createHomeStationStore, parseScenario } from './store'

// Mock runtime answers for `portal.*`: the seeded mock network read as an anonymous visitor.
// Usernames map to `did:bns:<user>`; the zone is the mock owner's.

const ANONYMOUS: ReaderIdentity = { kind: 'anonymous' }
const ZONE_FEED = '~zone'

export function createMockPortalSource(): PortalSource {
  const store = createHomeStationStore(parseScenario(window.location.search)) as HomeStationStore & { zoneEntries(): EntryUrl[] }
  const zone = zoneOf(OWNER_DID)
  const didOf = (feed: string) => `did:bns:${feed}`
  const cardsOf = async (ids: (string | undefined)[]) => (await Promise.all(ids.filter((id): id is string => !!id).map(id => store.getItem(id, ANONYMOUS)))).filter((card): card is CardView => !!card)

  async function zoneList(): Promise<PublishedPage & { cards: CardView[] }> {
    const listed = new Set(store.zoneEntries())
    const page = await store.listPublished(OWNER_DID, { reader: ANONYMOUS, kind: 'all' })
    const entries = page.entries.filter(entry => listed.has(entry.entry) && entry.head.state !== 'withdrawn').map(entry => ({ ...entry, user: userOf(OWNER_DID), zoneFeed: true }))
    return { entries, nextCursor: null, changeCursor: 0, cards: await cardsOf(entries.map(entry => entry.objId)) }
  }

  return {
    async call<T>(method: string, params: Record<string, unknown> = {}): Promise<T> {
      const feed = String(params.feed ?? '')
      switch (method) {
        case 'portal.home':
          return { zone, zoneName: zone, defaultFeed: userOf(OWNER_DID), zoneFeed: ZONE_FEED, viewer: null } satisfies PortalHome as T
        case 'portal.profile': {
          if (feed === ZONE_FEED) return { did: '', user: ZONE_FEED, kind: 'zone', name: zone, bio: '', hue: 200, followers: 0, following: 0, posts: store.zoneEntries().length, featured: [] } as T
          const profile = await store.getProfile(didOf(feed), ANONYMOUS)
          if (!profile) throw new Error('homestation.portal.noSuchFeed')
          return { ...profile, user: feed, kind: 'user' } as T
        }
        case 'portal.list': {
          if (feed === ZONE_FEED) return (await zoneList()) as T
          const page = await store.listPublished(didOf(feed), { reader: ANONYMOUS, kind: params.kind as never, cursor: (params.cursor as string | null) ?? null })
          return { ...page, cards: await cardsOf(page.entries.map(entry => entry.objId)) } as T
        }
        case 'portal.item': {
          if (typeof params.key === 'string') {
            const page = await store.listPublished(didOf(feed), { reader: ANONYMOUS, kind: 'all' })
            const entry = page.entries.find(row => row.entry.endsWith(`/@/${params.key}`))
            const card = entry?.objId ? await store.getItem(entry.objId, ANONYMOUS) : null
            return { entry: entry ?? null, card } as T
          }
          return { card: await store.getItem(String(params.objId), ANONYMOUS) } as T
        }
        case 'portal.comments':
          return store.listComments(String(params.objId), { view: 'author', type: (params.type as 'text') ?? 'text' }) as Promise<T>
        case 'portal.wrapped_body':
          return store.getWrappedBody(String(params.objId)) as Promise<T>
        default:
          throw new Error(`unknown method ${method}`)
      }
    },
  }
}
