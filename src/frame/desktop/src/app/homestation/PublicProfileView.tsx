import { ArrowDown, ArrowUp, Eye, ShieldCheck, Star } from 'lucide-react'
import { useCallback, useMemo, useState } from 'react'
import useSWR from 'swr'
import { useI18n } from '../../i18n/provider'
import { FeedCard } from './card/FeedCard'
import { previewReaderLabel } from './card/labels'
import { formatCount } from './datamodel/format'
import type { PublishedKindFilter, PublishedPage, ReaderIdentity } from './datamodel/types'
import { usePagedList } from './feed/usePagedList'
import { HsNavContext, useHsNav } from './navContext'
import type { Did } from './protocol/feed'
import { useHomeStationStore, usePreviewReaders, useStoreRevalidate, type PreviewReaderKey } from './store/context'
import type { ProfileTab } from './types'
import { Avatar, EmptyState, ErrorState, ListSkeleton } from './ui/primitives'
import { useToast } from './ui/toastContext'

export type PreviewReader = 'owner' | 'anonymous' | 'follower' | 'friend'

function PublishedList({ owner, kind, variant }: { owner: Did; kind: PublishedKindFilter | undefined; variant: 'profile' | 'visitor' }) {
  const { t } = useI18n()
  const store = useHomeStationStore()
  const nav = useHsNav()
  const reader = nav.reader
  const scope = JSON.stringify([owner, kind ?? 'feed', reader])
  const fetchPage = useCallback((key: string, cursor: string | null): Promise<PublishedPage> => {
    const [did, kindKey, readerKey] = JSON.parse(key) as [Did, string, ReaderIdentity]
    return store.listPublished(did, { reader: readerKey, kind: kindKey === 'feed' ? undefined : (kindKey as PublishedKindFilter), cursor })
  }, [store])
  const list = usePagedList<PublishedPage>(scope, fetchPage, ['published', 'profile'])
  const rows = useMemo(() => list.pages.flatMap(page => page.entries), [list.pages])
  const approximated = list.pages[0]?.readerApproximated ? (
    <p className={nav.isDesktop ? 'hs-note mx-6 my-4' : 'hs-note mx-4 my-4'} data-testid="hs-reader-approximated">
      <Eye size={14} />
      {t('homestation.profile.readerApproximated', 'Your HomeStation can’t read a third party’s home feed as another person, so this is approximated: it shows what an anonymous reader gets.')}
    </p>
  ) : null
  if (list.isLoading) return <ListSkeleton rows={3} />
  if (list.error) return <ErrorState onRetry={list.reload} />
  if (rows.length === 0) return <>{approximated}<EmptyState icon={<Eye size={28} />} title={t('homestation.profile.emptyTab', 'Nothing here for this reader')} /></>
  return (
    <div data-testid="hs-profile-list">
      {approximated}
      {rows.map(row => (row.objId ? <FeedCard key={row.entry} objId={row.objId} variant={variant} /> : null))}
      {list.hasMore ? <div className="flex justify-center py-4"><button type="button" className="hs-btn" onClick={list.loadMore}>{t('homestation.state.loadMore', 'Load more')}</button></div> : null}
    </div>
  )
}

function FeaturedList({ featured, editable, variant }: { featured: string[]; editable: boolean; variant: 'profile' | 'visitor' }) {
  const { t } = useI18n()
  const store = useHomeStationStore()
  const nav = useHsNav()
  if (featured.length === 0) return <EmptyState icon={<Star size={28} />} title={t('homestation.profile.noFeatured', 'No featured items')} />
  const move = (index: number, delta: number) => {
    const next = [...featured]
    const [item] = next.splice(index, 1)
    next.splice(index + delta, 0, item)
    void store.setFeatured(next)
  }
  return (
    <div>
      <p className={nav.isDesktop ? 'px-6 pt-4 text-[13px] leading-5' : 'px-4 pt-4 text-[13px] leading-5'} style={{ color: 'var(--cp-muted)' }}>{t('homestation.profile.featuredNote', 'Featured is an ordered list of references you maintain. Reordering never changes the original posts.')}</p>
      {featured.map((objId, index) => (
        <FeedCard
          key={objId}
          objId={objId}
          variant={variant}
          footer={editable ? (
            <div className="-ml-2 mt-2 flex gap-1">
              <button type="button" className="hs-icon-btn" disabled={index === 0} onClick={() => move(index, -1)} aria-label={t('homestation.profile.moveUp', 'Move up')} title={t('homestation.profile.moveUp', 'Move up')}><ArrowUp size={16} /></button>
              <button type="button" className="hs-icon-btn" disabled={index === featured.length - 1} onClick={() => move(index, 1)} aria-label={t('homestation.profile.moveDown', 'Move down')} title={t('homestation.profile.moveDown', 'Move down')}><ArrowDown size={16} /></button>
            </div>
          ) : null}
        />
      ))}
    </div>
  )
}

/**
 * A home feed as a reader gets it. In the owner's app with reader previews; on a portal page
 * with `onFollow` (a signed-in viewer follows from here) or `signInHref` (anonymous visitors).
 */
export function PublicProfileView({ owner, previewReader, onPreviewReaderChange, onFollow, signInHref }: { owner: Did; previewReader?: PreviewReader; onPreviewReaderChange?: (reader: PreviewReader) => void; onFollow?: () => Promise<void>; signInHref?: string }) {
  const { t } = useI18n()
  const store = useHomeStationStore()
  const nav = useHsNav()
  const toast = useToast()
  const [tab, setTab] = useState<ProfileTab>('posts')
  const [followState, setFollowState] = useState<'idle' | 'pending' | 'done'>('idle')
  const ownerView = nav.perspective === 'owner'
  const previewing = ownerView && previewReader && previewReader !== 'owner'
  const previewReaders = usePreviewReaders()
  const reader: ReaderIdentity = previewing ? previewReaders[previewReader as PreviewReaderKey] ?? previewReaders.anonymous : nav.reader
  const variant: 'profile' | 'visitor' = ownerView && !previewing ? 'profile' : 'visitor'
  const scopedNav = useMemo(() => ({ ...nav, reader, perspective: previewing ? ('visitor' as const) : nav.perspective }), [nav, previewing, reader])
  const profile = useSWR(['hs-profile', store.id, owner, JSON.stringify(reader)], () => store.getProfile(owner, reader), { revalidateOnFocus: false })
  const { mutate } = profile
  useStoreRevalidate(['profile', 'published'], useCallback(() => void mutate(), [mutate]))
  const data = profile.data
  const tabs: { id: ProfileTab; label: string }[] = [
    { id: 'posts', label: t('homestation.profile.tabPosts', 'Posts') },
    { id: 'works', label: t('homestation.profile.tabWorks', 'Works') },
    { id: 'products', label: t('homestation.profile.tabProducts', 'Products') },
    { id: 'featured', label: t('homestation.profile.tabFeatured', 'Featured') },
  ]
  const nameOf = (did: string) => store.peekIdentity(did).name
  const readerOptions = (['owner', 'anonymous', 'follower', 'friend'] as PreviewReader[]).filter(option => option === 'owner' || previewReaders[option])
  const edge = nav.isDesktop ? 'px-6' : 'px-4'
  const readerLabel = (option: PreviewReader) => (option === 'owner' ? t('homestation.profile.asOwner', 'You') : previewReaderLabel(t, option, previewReaders[option], nameOf))

  return (
    <HsNavContext.Provider value={scopedNav}>
      <div data-testid="hs-profile" data-reader={previewReader ?? reader.kind}>
        {ownerView && onPreviewReaderChange ? (
          <div className={`hs-scroll-x flex items-center gap-2 overflow-x-auto ${edge} py-3`} style={{ borderBottom: '1px solid var(--hs-line)' }}>
            <span className="flex-shrink-0 text-[13px]" style={{ color: 'var(--cp-muted)' }}>{t('homestation.profile.previewAs', 'Preview as')}</span>
            {readerOptions.map(option => (
              <button key={option} type="button" className="hs-chip" aria-pressed={(previewReader ?? 'owner') === option} data-testid={`hs-preview-${option}`} onClick={() => onPreviewReaderChange(option)}>
                {readerLabel(option)}
              </button>
            ))}
            <span className="ml-auto flex-shrink-0 pl-2 text-[13px]" style={{ color: 'var(--cp-muted)' }}>{t('homestation.profile.devOnly', 'Dev tool')}</span>
          </div>
        ) : null}
        <div className={`${edge} pb-4 pt-6`}>
          <div className="flex items-start gap-4">
            <div className="min-w-0 flex-1">
              <h2 className="flex min-w-0 items-center gap-2 text-[24px] font-semibold leading-8">
                <span className="truncate">{data?.name ?? '…'}</span>
                <ShieldCheck size={18} className="flex-shrink-0" style={{ color: 'var(--hs-text-2)' }} aria-label={t('homestation.verify.verified', 'Signature verified')} />
              </h2>
              <p className="mt-1 truncate font-mono text-[13px] leading-5" style={{ color: 'var(--cp-muted)' }}>{owner}</p>
            </div>
            <Avatar identity={{ name: data?.name ?? '?', hue: data?.hue ?? 260, avatar: data?.avatar }} size={nav.isDesktop ? 84 : 72} />
          </div>
          {data?.bio ? <p className="mt-4 text-[15px] leading-[22px]">{data.bio}</p> : null}
          {data ? (
            <p className="mt-4 flex flex-wrap items-center gap-x-4 gap-y-1 text-[15px] leading-[22px] tabular-nums" style={{ color: 'var(--cp-muted)' }}>
              <span title={t('homestation.profile.followersHint', 'Counted from valid follow declarations received, de-duplicated by DID')} data-testid="hs-followers">
                <span style={{ color: 'var(--cp-text)' }}>{formatCount(data.followers)}</span> {t('homestation.profile.followers', 'followers')}
              </span>
              <span><span style={{ color: 'var(--cp-text)' }}>{data.following}</span> {t('homestation.profile.followingCount', 'following')}</span>
              <span><span style={{ color: 'var(--cp-text)' }}>{data.posts}</span> {t('homestation.profile.postsCount', 'posts')}</span>
            </p>
          ) : null}
          {!ownerView && onFollow ? (
            <button
              type="button"
              className={followState === 'done' ? 'hs-btn is-block mt-4' : 'hs-btn is-primary is-block mt-4'}
              data-testid="hs-visitor-follow"
              disabled={followState !== 'idle'}
              onClick={() => {
                setFollowState('pending')
                onFollow().then(() => setFollowState('done'), error => {
                  setFollowState('idle')
                  toast({ text: t('homestation.portal.followFailed', 'Couldn’t follow: {{reason}}', { reason: error instanceof Error ? error.message : String(error) }) })
                })
              }}
            >
              {followState === 'done' ? t('homestation.profile.following', 'Following') : t('homestation.profile.follow', 'Follow')}
            </button>
          ) : null}
          {!ownerView && !onFollow && signInHref ? (
            <a className="hs-btn is-block mt-4" href={signInHref} data-testid="hs-visitor-sign-in">
              {t('homestation.portal.signInToFollow', 'Sign in to follow')}
            </a>
          ) : null}
          {!ownerView || previewing ? <p className="mt-3 text-[13px] leading-5" style={{ color: 'var(--cp-muted)' }}>{t('homestation.profile.visitorNote', 'This is the home feed as this reader gets it: only entries their audience allows. Reading lists, bookmarks and recommendation reasons are never included.')}</p> : null}
        </div>
        <div className="hs-tabs is-even" role="tablist">
          {tabs.map(entry => (
            <button key={entry.id} type="button" role="tab" aria-selected={tab === entry.id} className="hs-tab" onClick={() => setTab(entry.id)} data-testid={`hs-profile-tab-${entry.id}`}>
              {entry.label}
            </button>
          ))}
        </div>
        {tab === 'posts' ? <PublishedList owner={owner} kind={undefined} variant={variant} /> : null}
        {tab === 'works' ? <PublishedList owner={owner} kind="work" variant={variant} /> : null}
        {tab === 'products' ? <PublishedList owner={owner} kind="product" variant={variant} /> : null}
        {tab === 'featured' ? (profile.data ? <FeaturedList featured={profile.data.featured} editable={variant === 'profile' && owner === store.owner} variant={variant} /> : <ListSkeleton rows={2} />) : null}
      </div>
    </HsNavContext.Provider>
  )
}
