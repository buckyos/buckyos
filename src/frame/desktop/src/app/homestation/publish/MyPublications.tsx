import { Bug, Lock, PenSquare, Send } from 'lucide-react'
import { useCallback, useMemo, useState } from 'react'
import useSWR from 'swr'
import { useWindowDialog } from '../../../desktop/windows/dialogs'
import { useI18n } from '../../../i18n/provider'
import { useItemActions } from '../actions'
import { FeedCard, PublishStatus } from '../card/FeedCard'
import { audienceLabel, formatDateTime, shortObjId } from '../datamodel/format'
import type { PublishedEntryView, PublishedKindFilter, PublishedPage } from '../datamodel/types'
import { usePagedList } from '../feed/usePagedList'
import type { HomeStationStore } from '../store/types'
import { useHsNav } from '../navContext'
import { useHomeStationStore, useStoreRevalidate, useStoreSelector } from '../store/context'
import { EmptyState, ErrorState, ListSkeleton, PageHeader } from '../ui/primitives'

const selectEntries = (store: HomeStationStore) => store.peekEntries()
const selectGroups = (store: HomeStationStore) => store.peekGroups()

function HeadDebugPanel() {
  const { t, locale } = useI18n()
  const store = useHomeStationStore()
  const entries = useStoreSelector(selectEntries)
  const changes = useSWR(['hs-changes', store.id], () => store.listChanges(store.owner, { reader: { kind: 'owner' }, after: 0 }), { revalidateOnFocus: false })
  const { mutate } = changes
  useStoreRevalidate(['published'], useCallback(() => void mutate(), [mutate]))
  return (
    <div className="hs-root space-y-4 text-xs" data-testid="hs-head-debug">
      <p style={{ color: 'var(--cp-muted)' }}>{t('homestation.debug.intro', 'Every like, repost, edit, withdrawal or public bookmark signs a new Head with a higher seq on its entry. Audience changes don’t create a Head; they appear in the change log only.')}</p>
      <table className="w-full border-collapse">
        <thead>
          <tr style={{ color: 'var(--cp-muted)' }}>
            <th className="py-1 text-left font-medium">{t('homestation.debug.entry', 'Entry')}</th>
            <th className="py-1 text-left font-medium">{t('homestation.debug.kind', 'Kind')}</th>
            <th className="py-1 text-right font-medium">seq</th>
            <th className="py-1 text-left font-medium">{t('homestation.debug.state', 'State')}</th>
          </tr>
        </thead>
        <tbody>
          {entries.map(entry => {
            const head = entry.heads[entry.heads.length - 1]
            return (
              <tr key={entry.entry} style={{ borderTop: '1px solid var(--hs-divider)' }} data-testid="hs-head-row" data-entry={entry.entry} data-seq={head.seq}>
                <td className="max-w-[220px] truncate py-1 font-mono" title={entry.entry}>{entry.entry.replace(/^cyfs:\/\/[^/]+\/home\//, '')}</td>
                <td className="py-1">{entry.kind}</td>
                <td className="py-1 text-right tabular-nums">{head.seq}</td>
                <td className="py-1">{head.state}{head.current ? ` → ${shortObjId(head.current)}` : ''}</td>
              </tr>
            )
          })}
        </tbody>
      </table>
      <div>
        <p className="mb-1 font-semibold">{t('homestation.debug.changes', 'Change log (what a follower reads with a change cursor)')}</p>
        <ol className="max-h-48 space-y-0.5 overflow-y-auto font-mono">
          {[...(changes.data ?? [])].reverse().slice(0, 30).map(change => (
            <li key={change.cursor}>#{change.cursor} {change.kind} seq={change.seq} {change.state} {change.entry.replace(/^cyfs:\/\/[^/]+\/home\//, '')} · {formatDateTime(change.at, locale)}</li>
          ))}
        </ol>
      </div>
    </div>
  )
}

function PublishedRow({ row }: { row: PublishedEntryView }) {
  const { t } = useI18n()
  const store = useHomeStationStore()
  const actions = useItemActions()
  const groups = useStoreSelector(selectGroups)
  const label = audienceLabel(t, row.audience.spec, groups, did => store.peekIdentity(did).name)
  const withdrawn = row.head.state === 'withdrawn'
  const footer = (
    <div className="mt-2 flex flex-wrap items-center gap-1.5 text-[11px]" data-testid="hs-published-meta">
      <span className={row.audience.restricted ? 'hs-badge is-accent' : 'hs-badge'} data-testid="hs-published-audience">{row.audience.restricted ? <Lock size={10} /> : null}{label}</span>
      {row.audience.spec.kind === 'dids' && row.kind === 'comment' ? <span className="hs-badge">{t('homestation.published.notOnHomepage', 'Not on your public homepage')}</span> : null}
      <span className="hs-badge" title={row.entry}>{t('homestation.published.head', 'Head seq {{seq}} · {{state}}', { seq: row.head.seq, state: withdrawn ? t('homestation.version.withdrawn', 'Withdrawn') : t('homestation.published.active', 'active') })}</span>
      {!withdrawn ? <PublishStatus task={row.task} onRetry={row.task?.stage === 'failed' ? () => void store.retryPublish(row.task!.key) : row.task?.delivery?.state === 'partially_failed' ? () => void store.retryDelivery(row.entry) : undefined} /> : null}
      {!withdrawn ? (
        <span className="ml-auto flex gap-1">
          <button type="button" className="hs-badge is-accent" onClick={() => void actions.changeAudience(row.entry, row.audience.spec)} data-testid="hs-published-audience-btn">{t('homestation.menu.audience', 'Change audience')}</button>
          <button type="button" className="hs-badge is-danger" onClick={() => void (row.kind === 'like' || row.kind === 'bookmark' ? store.withdraw(row.entry) : actions.withdraw(row.entry))} data-testid="hs-published-withdraw">{row.kind === 'like' ? t('homestation.like.unlike', 'Unlike') : row.kind === 'bookmark' ? t('homestation.bookmark.makePrivate', 'Make private') : t('homestation.menu.withdraw', 'Withdraw')}</button>
        </span>
      ) : null}
    </div>
  )
  if (!row.objId) return null
  return <FeedCard objId={row.objId} variant="published" footer={footer} />
}

export function MyPublications() {
  const { t } = useI18n()
  const store = useHomeStationStore()
  const nav = useHsNav()
  const dialog = useWindowDialog()
  const [kind, setKind] = useState<PublishedKindFilter>('all')
  const fetchPage = useCallback((scope: string, cursor: string | null): Promise<PublishedPage> => store.listPublished(store.owner, { reader: { kind: 'owner' }, kind: scope as PublishedKindFilter, cursor }), [store])
  const list = usePagedList<PublishedPage>(kind, fetchPage, ['published'])
  const rows = useMemo(() => list.pages.flatMap(page => page.entries), [list.pages])
  const kinds: { id: PublishedKindFilter; label: string }[] = [
    { id: 'all', label: t('homestation.published.kindAll', 'All') },
    { id: 'posts', label: t('homestation.published.kindPosts', 'Posts') },
    { id: 'comments', label: t('homestation.published.kindComments', 'Comments') },
    { id: 'reposts', label: t('homestation.published.kindReposts', 'Reposts & quotes') },
    { id: 'reactions', label: t('homestation.published.kindReactions', 'Likes & public bookmarks') },
    { id: 'work', label: t('homestation.published.kindWork', 'Works') },
    { id: 'product', label: t('homestation.published.kindProduct', 'Products') },
  ]
  return (
    <div className="flex h-full min-h-0 flex-col" data-testid="hs-published">
      <PageHeader
        title={t('homestation.nav.published', 'My publications')}
        subtitle={t('homestation.published.subtitle', 'Your home feed: everything you published, with its audience and delivery state')}
        onBack={nav.isDesktop ? undefined : nav.back}
        backLabel={t('common.back', 'Back')}
        actions={(
          <>
            <button type="button" className="hs-icon-btn" aria-label={t('homestation.debug.open', 'Head debug panel')} title={t('homestation.debug.open', 'Head debug panel')} data-testid="hs-head-debug-open" onClick={() => void dialog.open({ title: t('homestation.debug.title', 'Entry Heads (debug)'), size: 'lg', renderBody: () => <HeadDebugPanel /> })}>
              <Bug size={16} />
            </button>
            {nav.isDesktop ? null : (
              <button type="button" className="hs-icon-btn" aria-label={t('homestation.publish.title', 'New post')} onClick={() => nav.navigate({ name: 'publish' })}>
                <PenSquare size={16} />
              </button>
            )}
          </>
        )}
      />
      <div className="hs-scroll-x flex gap-1.5 overflow-x-auto px-4 py-2" role="tablist" aria-label={t('homestation.published.kindLabel', 'Kind')}>
        {kinds.map(entry => (
          <button key={entry.id} type="button" role="tab" aria-selected={kind === entry.id} className="hs-chip" onClick={() => setKind(entry.id)} data-testid={`hs-published-kind-${entry.id}`}>
            {entry.label}
          </button>
        ))}
      </div>
      <div className="desktop-scrollbar min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto w-full max-w-[760px]">
          {list.isLoading ? <ListSkeleton rows={3} /> : null}
          {list.error ? <ErrorState onRetry={list.reload} /> : null}
          {!list.isLoading && !list.error && rows.length === 0 ? <EmptyState icon={<Send size={32} strokeWidth={1.4} />} title={t('homestation.published.empty', 'Nothing published in this category yet')} /> : null}
          {rows.map(row => <PublishedRow key={row.entry} row={row} />)}
          {list.hasMore ? <div className="flex justify-center py-3"><button type="button" className="hs-btn" onClick={list.loadMore}>{t('homestation.state.loadMore', 'Load more')}</button></div> : null}
          <div className="h-20" />
        </div>
      </div>
    </div>
  )
}
