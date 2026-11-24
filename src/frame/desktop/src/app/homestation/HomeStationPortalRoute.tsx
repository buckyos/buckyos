import { useMediaQuery } from '@mui/material'
import { ExternalLink, Globe2, Loader2, LogIn, Users, Waypoints } from 'lucide-react'
import { useCallback, useMemo, useState } from 'react'
import { useNavigate, useParams } from 'react-router-dom'
import useSWR from 'swr'
import { WindowDialogProvider } from '../../desktop/windows/dialogs'
import { useI18n } from '../../i18n/provider'
import './homestation.css'
import { createPortalStore, type PortalSource } from './api/portalStore'
import { homeStationTransport } from './api/transport'
import { FeedCard } from './card/FeedCard'
import type { PortalHome, PublishedPage, ReaderIdentity } from './datamodel/types'
import { ItemDetail } from './detail/ItemDetail'
import { usePagedList } from './feed/usePagedList'
import { parseEntry, portalHref, portalPath, portalShareUrl, ZONE_FEED, zoneOrigin } from './links'
import { installHomeStationPreviewSource } from './media'
import { createMockPortalSource } from './mock/portal'
import { HsNavContext, type HsNav } from './navContext'
import type { ObjId } from './protocol/feed'
import { PublicProfileView } from './PublicProfileView'
import { useHomeStationStore } from './store/context'
import { HomeStationStoreProvider } from './store/HomeStationStoreProvider'
import { ColumnBody, EmptyState, ErrorState, ListSkeleton, PageHeader } from './ui/primitives'
import { ToastHost } from './ui/ToastHost'
import { useToast } from './ui/toastContext'
import { usableImageUrl } from '../../presentation/model'
import { ObjectImage } from '../../presentation/ObjectIcon'

installHomeStationPreviewSource()

let source: PortalSource | null = null

function portalSource(): PortalSource {
  source ??= homeStationTransport() ?? createMockPortalSource()
  return source
}

/** Full page loads on a short host (its links point at the zone host), router moves otherwise. */
function useGo() {
  const navigate = useNavigate()
  return useCallback((feed: string, key?: string) => {
    const href = portalHref(feed, key)
    if (href !== portalPath(feed, key)) window.location.assign(href)
    else navigate(href)
  }, [navigate])
}

function PortalBar({ home, feed, entryKey }: { home: PortalHome; feed: string; entryKey?: string }) {
  const { t } = useI18n()
  const viewer = home.viewer
  const login = `${zoneOrigin()}/login?redirect_url=${encodeURIComponent(portalShareUrl(feed, entryKey))}`
  return (
    <div className="hs-portal-bar flex flex-shrink-0 items-center gap-3 px-4 py-3 md:px-6" data-testid="hs-portal-bar">
      <a href={portalHref(ZONE_FEED)} className="flex min-w-0 items-center gap-2 text-[15px] font-semibold" data-testid="hs-portal-zone">
        <ObjectImage icons={[usableImageUrl(home.zoneIcon)]} className="h-6 w-6 rounded-md object-cover" fallback={<Globe2 size={20} />} />
        <span className="truncate">{home.zoneName}</span>
      </a>
      <span className="ml-auto flex flex-shrink-0 items-center gap-2">
        {viewer?.user ? (
          <>
            <a href={portalHref(viewer.user)} className="hs-btn is-small" data-testid="hs-portal-my-page">{t('homestation.portal.myPage', 'My page')}</a>
            <a href={`${zoneOrigin()}/homestation`} className="hs-btn is-primary is-small" data-testid="hs-portal-open-app"><Waypoints size={16} />{t('homestation.portal.openApp', 'My HomeStation')}</a>
          </>
        ) : (
          <a href={login} className="hs-btn is-primary is-small" data-testid="hs-portal-sign-in"><LogIn size={16} />{t('homestation.portal.signIn', 'Sign in')}</a>
        )}
      </span>
    </div>
  )
}

function ZoneFeedList() {
  const { t } = useI18n()
  const store = useHomeStationStore()
  const go = useGo()
  const fetchPage = useCallback((_scope: string, cursor: string | null): Promise<PublishedPage> => store.listPublished('', { reader: { kind: 'anonymous' }, cursor }), [store])
  const list = usePagedList<PublishedPage>(ZONE_FEED, fetchPage, [])
  const rows = useMemo(() => list.pages.flatMap(page => page.entries), [list.pages])
  if (list.isLoading) return <ListSkeleton rows={3} />
  if (list.error) return <ErrorState onRetry={list.reload} />
  if (rows.length === 0) return <EmptyState icon={<Globe2 size={28} />} title={t('homestation.portal.zoneEmpty', 'Nothing on the zone page yet')} body={t('homestation.portal.zoneEmptyBody', 'Users of this zone choose, when they post, whether a post also appears here.')} />
  return (
    <div data-testid="hs-zone-list">
      {rows.map(row => (row.objId ? (
        <FeedCard
          key={row.entry}
          objId={row.objId}
          variant="visitor"
          footer={row.user ? (
            <button type="button" className="hs-link mt-3 text-[13px]" onClick={() => go(row.user!)} data-testid="hs-zone-author">
              <Users size={14} />{t('homestation.portal.openAuthor', 'Open {{user}}’s page', { user: row.user })}
            </button>
          ) : null}
        />
      ) : null))}
      {list.hasMore ? <div className="flex justify-center py-4"><button type="button" className="hs-btn" onClick={list.loadMore}>{t('homestation.state.loadMore', 'Load more')}</button></div> : null}
    </div>
  )
}

function ZoneFeedView({ home }: { home: PortalHome }) {
  const { t } = useI18n()
  return (
    <div data-testid="hs-zone-feed">
      <div className="px-4 pb-5 pt-6 md:px-6" style={{ borderBottom: '1px solid var(--hs-line)' }}>
        <h1 className="flex items-center gap-3 text-[24px] font-semibold leading-8"><ObjectImage icons={[usableImageUrl(home.zoneIcon)]} className="h-8 w-8 rounded-lg object-cover" fallback={<Globe2 size={24} />} />{home.zoneName}</h1>
        <p className="mt-2 text-[15px] leading-[22px]" style={{ color: 'var(--cp-muted)' }}>{t('homestation.portal.zoneIntro', 'Posts the users of this zone chose to show on its front page. Each one stays signed by its author and lives in the author’s own home feed.')}</p>
      </div>
      <ZoneFeedList />
    </div>
  )
}

/** `/homestation/<user>/<key>`: the entry's current version. */
function EntryView({ feed, entryKey }: { feed: string; entryKey: string }) {
  const { t } = useI18n()
  const go = useGo()
  const item = useSWR(['hs-portal-entry', feed, entryKey], () => portalSource().call<{ card: { item: { objId: ObjId } } | null }>('portal.item', { feed, key: entryKey }), { revalidateOnFocus: false, shouldRetryOnError: false })
  if (item.isLoading) return <ListSkeleton rows={2} />
  const objId = item.data?.card?.item.objId
  if (!objId) {
    return (
      <div className="flex h-full flex-col">
        <PageHeader title={t('homestation.detail.post', 'Post')} onBack={() => go(feed)} backLabel={t('common.back', 'Back')} />
        <ColumnBody><EmptyState icon={<Globe2 size={28} />} title={t('homestation.detail.notVisible', 'This item isn’t visible to you')} body={t('homestation.portal.entryMissing', 'It may have been withdrawn, or it is shared with a restricted audience.')} /></ColumnBody>
      </div>
    )
  }
  return <ItemDetail key={objId} objId={objId} />
}

function PortalView({ home, feed, entryKey }: { home: PortalHome; feed: string; entryKey?: string }) {
  const { t } = useI18n()
  const store = useHomeStationStore()
  const toast = useToast()
  const go = useGo()
  const isDesktop = useMediaQuery('(min-width: 769px)')
  const [detail, setDetail] = useState<ObjId | null>(null)
  const viewer = home.viewer
  const viewerDid = viewer?.did
  const reader = useMemo<ReaderIdentity>(() => (viewerDid ? { kind: 'did', did: viewerDid } : { kind: 'anonymous' }), [viewerDid])

  const nav = useMemo<HsNav>(() => ({
    page: detail ? { name: 'detail', objId: detail } : { name: 'profile' },
    perspective: 'visitor',
    reader,
    isDesktop,
    inTab: false,
    navigate: () => {},
    back: () => {
      if (detail) setDetail(null)
      else if (entryKey) go(feed)
      else window.history.back()
    },
    openDetail: objId => {
      const entry = parseEntry(store.peekCard(objId, reader)?.item.entry?.entry)
      if (entry) go(entry.user, entry.key)
      else setDetail(objId)
    },
    showFilteredInFeed: () => {},
  }), [detail, entryKey, feed, go, isDesktop, reader, store])

  const follow = viewer?.user && store.owner && viewer.did !== store.owner
    ? async () => {
      const transport = homeStationTransport()
      if (!transport) return
      const resolution = await transport.call<unknown>('sources.resolve', { kind: 'follow', text: store.owner })
      await transport.call('sources.follow', { resolution })
      toast({ text: t('homestation.portal.followed', 'Following. Your HomeStation signed a follow declaration and will keep reading this home feed.') })
    }
    : undefined

  let body
  if (detail) body = <ItemDetail key={detail} objId={detail} />
  else if (entryKey) body = <EntryView feed={feed} entryKey={entryKey} />
  else if (feed === ZONE_FEED) body = <ZoneFeedView home={home} />
  else body = <PublicProfileView owner={store.owner} onFollow={follow} signInHref={viewer ? undefined : `${zoneOrigin()}/login?redirect_url=${encodeURIComponent(portalShareUrl(feed))}`} />

  return (
    <HsNavContext.Provider value={nav}>
      <WindowDialogProvider surface={isDesktop ? 'desktop' : 'mobile'} permissions={{ fullscreen: false }}>
        <div className="flex h-full min-h-0 flex-col">
          <PortalBar home={home} feed={feed} entryKey={entryKey} />
          <div className="flex min-h-0 flex-1 justify-center md:px-6">
            <div className="flex h-full min-w-0 flex-col" style={{ flex: '0 1 640px' }}>
              {detail || entryKey ? body : (
                <ColumnBody>
                  {body}
                  <p className="flex items-center justify-center gap-2 px-4 py-6 text-[13px]" style={{ color: 'var(--cp-muted)' }}>
                    <ExternalLink size={14} />
                    <span className="truncate font-mono">{portalShareUrl(feed, entryKey)}</span>
                  </p>
                </ColumnBody>
              )}
            </div>
          </div>
        </div>
      </WindowDialogProvider>
    </HsNavContext.Provider>
  )
}

function PortalPage({ home, feed, entryKey }: { home: PortalHome; feed: string; entryKey?: string }) {
  const create = useCallback(() => createPortalStore(portalSource(), feed), [feed])
  return (
    <HomeStationStoreProvider create={create}>
      <PortalView home={home} feed={feed} entryKey={entryKey} />
    </HomeStationStoreProvider>
  )
}

/**
 * `/homestation/:feed[/:key]`, and `/` on `www.<zone>` / `homestation.<zone>` (the zone's
 * default feed): public, login optional. A signed-in viewer reads with their own identity.
 */
export function HomeStationPortalRoute() {
  const { t } = useI18n()
  const params = useParams()
  const home = useSWR('hs-portal-home', () => portalSource().call<PortalHome>('portal.home'), { revalidateOnFocus: false, shouldRetryOnError: false })
  const feed = params.feed ?? home.data?.defaultFeed
  const isDesktop = useMediaQuery('(min-width: 769px)')
  return (
    <main className={isDesktop ? 'hs-root hs-shell-desktop relative flex h-dvh w-full flex-col overflow-hidden' : 'hs-root hs-shell-mobile relative flex h-dvh w-full flex-col overflow-hidden'} style={{ background: isDesktop ? 'var(--hs-page)' : 'var(--hs-column)' }} data-testid="hs-portal" data-feed={feed ?? ''}>
      <ToastHost>
        {home.data && feed ? (
          <PortalPage key={feed} home={home.data} feed={feed} entryKey={params.key} />
        ) : home.error ? (
          <ErrorState onRetry={() => void home.mutate()} />
        ) : (
          <div className="flex flex-1 items-center justify-center"><Loader2 size={24} className="animate-spin" style={{ color: 'var(--cp-muted)' }} aria-label={t('homestation.state.loading', 'Loading…')} /></div>
        )}
      </ToastHost>
    </main>
  )
}
