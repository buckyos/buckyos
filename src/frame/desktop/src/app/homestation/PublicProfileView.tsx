import { ArrowDown, ArrowUp, Eye, ShieldCheck, Star, UserPlus } from 'lucide-react'
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
    <p className="mx-4 mt-3 flex items-start gap-1.5 rounded-xl px-3 py-2 text-xs leading-5" style={{ background: 'var(--hs-subtle-bg)', color: 'var(--cp-muted)' }} data-testid="hs-reader-approximated">
      <Eye size={13} className="mt-0.5 flex-shrink-0" />
      {t('homestation.profile.readerApproximated', 'Your HomeStation can’t read a third party’s home feed as another person, so this is approximated: it shows what an anonymous reader gets.')}
    </p>
  ) : null
  if (list.isLoading) return <ListSkeleton rows={3} />
  if (list.error) return <ErrorState onRetry={list.reload} />
  if (rows.length === 0) return <>{approximated}<EmptyState icon={<Eye size={30} strokeWidth={1.4} />} title={t('homestation.profile.emptyTab', 'Nothing here for this reader')} /></>
  return (
    <div data-testid="hs-profile-list">
      {approximated}
      {rows.map(row => (row.objId ? <FeedCard key={row.entry} objId={row.objId} variant={variant} /> : null))}
      {list.hasMore ? <div className="flex justify-center py-3"><button type="button" className="hs-btn" onClick={list.loadMore}>{t('homestation.state.loadMore', 'Load more')}</button></div> : null}
    </div>
  )
}

function FeaturedList({ featured, editable, variant }: { featured: string[]; editable: boolean; variant: 'profile' | 'visitor' }) {
  const { t } = useI18n()
  const store = useHomeStationStore()
  if (featured.length === 0) return <EmptyState icon={<Star size={30} strokeWidth={1.4} />} title={t('homestation.profile.noFeatured', 'No featured items')} />
  const move = (index: number, delta: number) => {
    const next = [...featured]
    const [item] = next.splice(index, 1)
    next.splice(index + delta, 0, item)
    void store.setFeatured(next)
  }
  return (
    <div>
      <p className="px-4 pt-3 text-[11px]" style={{ color: 'var(--cp-muted)' }}>{t('homestation.profile.featuredNote', 'Featured is an ordered list of references you maintain. Reordering never changes the original posts.')}</p>
      {featured.map((objId, index) => (
        <FeedCard
          key={objId}
          objId={objId}
          variant={variant}
          footer={editable ? (
            <div className="mt-2 flex gap-1">
              <button type="button" className="hs-badge" disabled={index === 0} onClick={() => move(index, -1)} aria-label={t('homestation.profile.moveUp', 'Move up')}><ArrowUp size={11} /></button>
              <button type="button" className="hs-badge" disabled={index === featured.length - 1} onClick={() => move(index, 1)} aria-label={t('homestation.profile.moveDown', 'Move down')}><ArrowDown size={11} /></button>
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
  const readerLabel = (option: PreviewReader) => (option === 'owner' ? t('homestation.profile.asOwner', 'You') : previewReaderLabel(t, option, previewReaders[option], nameOf))

  return (
    <HsNavContext.Provider value={scopedNav}>
      <div data-testid="hs-profile" data-reader={previewReader ?? reader.kind}>
        {ownerView && onPreviewReaderChange ? (
          <div className="flex flex-wrap items-center gap-1.5 px-4 py-2 text-[11px]" style={{ borderBottom: '1px solid var(--hs-divider)', background: 'var(--hs-subtle-bg)' }}>
            <Eye size={12} />
            <span style={{ color: 'var(--cp-muted)' }}>{t('homestation.profile.previewAs', 'Preview as')}</span>
            {readerOptions.map(option => (
              <button key={option} type="button" className="hs-chip" style={{ padding: '3px 10px' }} aria-pressed={(previewReader ?? 'owner') === option} data-testid={`hs-preview-${option}`} onClick={() => onPreviewReaderChange(option)}>
                {readerLabel(option)}
              </button>
            ))}
            <span className="hs-badge">{t('homestation.profile.devOnly', 'Dev tool')}</span>
          </div>
        ) : null}
        <div className="h-28 w-full md:h-40" style={{ background: `linear-gradient(135deg, color-mix(in srgb, hsl(${data?.hue ?? 260} 60% 55%) 34%, var(--cp-surface)), color-mix(in srgb, var(--cp-accent-soft) 40%, var(--cp-surface-2)))` }} />
        <div className="relative px-4 pb-3">
          <div className="-mt-9 rounded-full border-4" style={{ borderColor: 'var(--cp-bg)', width: 'fit-content' }}>
            <Avatar identity={{ name: data?.name ?? '?', hue: data?.hue ?? 260 }} size={72} />
          </div>
          <div className="mt-2 flex flex-wrap items-start gap-2">
            <div className="min-w-0 flex-1">
              <h2 className="flex items-center gap-1.5 text-xl font-bold">
                {data?.name ?? '…'}
                <ShieldCheck size={17} style={{ color: 'var(--cp-success)' }} aria-label={t('homestation.verify.verified', 'Signature verified')} />
              </h2>
              <p className="truncate font-mono text-[11px]" style={{ color: 'var(--cp-muted)' }}>{owner}</p>
            </div>
            {!ownerView && onFollow ? (
              <button
                type="button"
                className={followState === 'done' ? 'hs-btn' : 'hs-btn is-primary'}
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
                <UserPlus size={14} />
                {followState === 'done' ? t('homestation.profile.following', 'Following') : t('homestation.profile.follow', 'Follow')}
              </button>
            ) : null}
            {!ownerView && !onFollow && signInHref ? (
              <a className="hs-btn" href={signInHref} data-testid="hs-visitor-sign-in">
                <UserPlus size={14} />
                {t('homestation.portal.signInToFollow', 'Sign in to follow')}
              </a>
            ) : null}
          </div>
          {data?.bio ? <p className="mt-1.5 text-sm leading-relaxed" style={{ color: 'color-mix(in srgb, var(--cp-text) 78%, transparent)' }}>{data.bio}</p> : null}
          {data ? (
            <div className="mt-2 flex flex-wrap items-center gap-x-4 gap-y-1 text-sm" style={{ color: 'var(--cp-muted)' }}>
              <span><strong style={{ color: 'var(--cp-text)' }}>{data.posts}</strong> {t('homestation.profile.postsCount', 'posts visible to this reader')}</span>
              <span title={t('homestation.profile.followersHint', 'Counted from valid follow declarations received, de-duplicated by DID')} data-testid="hs-followers">
                <strong style={{ color: 'var(--cp-text)' }}>{formatCount(data.followers)}</strong> {t('homestation.profile.followers', 'followers (valid follow declarations received)')}
              </span>
              <span><strong style={{ color: 'var(--cp-text)' }}>{data.following}</strong> {t('homestation.profile.followingCount', 'following')}</span>
            </div>
          ) : null}
          {!ownerView || previewing ? <p className="mt-2 text-[11px]" style={{ color: 'var(--cp-muted)' }}>{t('homestation.profile.visitorNote', 'This is the home feed as this reader gets it: only entries their audience allows. Reading lists, bookmarks and recommendation reasons are never included.')}</p> : null}
        </div>
        <div className="flex gap-0.5 overflow-x-auto px-3" style={{ borderBottom: '1px solid var(--hs-divider)' }} role="tablist">
          {tabs.map(entry => (
            <button key={entry.id} type="button" role="tab" aria-selected={tab === entry.id} className="relative px-3 py-2.5 text-sm font-medium" style={{ color: tab === entry.id ? 'var(--cp-text)' : 'var(--cp-muted)' }} onClick={() => setTab(entry.id)} data-testid={`hs-profile-tab-${entry.id}`}>
              {entry.label}
              {tab === entry.id ? <span className="absolute bottom-0 left-2 right-2 h-0.5 rounded-full" style={{ background: 'var(--cp-accent)' }} /> : null}
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
