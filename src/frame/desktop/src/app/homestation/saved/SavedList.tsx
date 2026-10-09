import { Bookmark, Clock, Globe, History, Lock, Trash2 } from 'lucide-react'
import { useCallback } from 'react'
import useSWR from 'swr'
import { useI18n } from '../../../i18n/provider'
import { FeedCard } from '../card/FeedCard'
import { formatTimeAgo } from '../datamodel/format'
import type { SavedItem } from '../datamodel/types'
import { useHsNav } from '../navContext'
import { useHomeStationStore, useNow, useStoreRevalidate } from '../store/context'
import { EmptyState, ErrorState, ListSkeleton, PageHeader } from '../ui/primitives'

function SavedFooter({ item, kind }: { item: SavedItem; kind: 'bookmark' | 'read_later' }) {
  const { t } = useI18n()
  const now = useNow()
  return (
    <div className="mt-2 flex flex-wrap items-center gap-1.5 text-[11px]" data-testid="hs-saved-meta" data-target={item.targetState}>
      {kind === 'bookmark' ? (
        item.visibility === 'private'
          ? <span className="hs-badge"><Lock size={10} />{t('homestation.saved.private', 'Private bookmark')}</span>
          : <span className="hs-badge is-accent"><Globe size={10} />{t('homestation.saved.public', 'Public bookmark')}</span>
      ) : null}
      {item.targetState === 'withdrawn' ? <span className="hs-badge is-danger" data-testid="hs-saved-withdrawn"><Trash2 size={10} />{t('homestation.saved.targetWithdrawn', 'Target withdrawn by its author')}</span> : null}
      {item.targetState === 'updated' ? <span className="hs-badge is-warning"><History size={10} />{t('homestation.saved.targetUpdated', 'Saved version was updated')}</span> : null}
      <span style={{ color: 'var(--cp-muted)' }}>{t('homestation.saved.savedAgo', 'saved {{time}} ago', { time: formatTimeAgo(t, item.savedAt, now) })}</span>
    </div>
  )
}

export function SavedList({ kind }: { kind: 'bookmark' | 'read_later' }) {
  const { t } = useI18n()
  const store = useHomeStationStore()
  const nav = useHsNav()
  const swr = useSWR(['hs-saved', store.id, kind], () => store.listSaved(kind), { revalidateOnFocus: false, shouldRetryOnError: false })
  const { mutate } = swr
  useStoreRevalidate(['saved', 'published'], useCallback(() => void mutate(), [mutate]))
  const title = kind === 'bookmark' ? t('homestation.nav.bookmarks', 'Bookmarks') : t('homestation.nav.readLater', 'Read later')
  return (
    <div className="flex h-full min-h-0 flex-col" data-testid={`hs-saved-${kind}`}>
      <PageHeader
        title={title}
        subtitle={kind === 'bookmark' ? t('homestation.saved.bookmarkSubtitle', 'Private unless you make one public. Kept even if the author withdraws the post.') : t('homestation.saved.laterSubtitle', 'Only you can see this list.')}
        onBack={nav.isDesktop ? undefined : nav.back}
        backLabel={t('common.back', 'Back')}
      />
      <div className="desktop-scrollbar min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto w-full max-w-[760px]">
          {swr.isLoading ? <ListSkeleton rows={2} /> : null}
          {swr.error ? <ErrorState onRetry={() => void mutate()} /> : null}
          {swr.data && swr.data.length === 0 ? <EmptyState icon={kind === 'bookmark' ? <Bookmark size={30} strokeWidth={1.4} /> : <Clock size={30} strokeWidth={1.4} />} title={kind === 'bookmark' ? t('homestation.saved.emptyBookmarks', 'No bookmarks yet') : t('homestation.saved.emptyLater', 'Nothing saved for later')} /> : null}
          {swr.data?.map(item => <FeedCard key={item.objId} objId={item.objId} variant="saved" footer={<SavedFooter item={item} kind={kind} />} />)}
          <div className="h-20" />
        </div>
      </div>
    </div>
  )
}
