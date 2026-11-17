import { CircleAlert, Inbox, Loader2, RefreshCw } from 'lucide-react'
import { useCallback, useMemo, useState } from 'react'
import { useI18n } from '../../../i18n/provider'
import { FeedCard } from '../card/FeedCard'
import { formatTimeAgo } from '../datamodel/format'
import type { CandidateEntry, CandidatePage } from '../datamodel/types'
import { usePagedList } from '../feed/usePagedList'
import { useHsNav } from '../navContext'
import { useHomeStationStore, useNow } from '../store/context'
import { ColumnBody, EmptyState, ErrorState, ListSkeleton, PageHeader } from '../ui/primitives'

function CandidateFooter({ entry }: { entry: CandidateEntry }) {
  const { t } = useI18n()
  const now = useNow()
  const store = useHomeStationStore()
  const nav = useHsNav()
  const [state, setState] = useState<'idle' | 'opening' | 'failed'>('idle')
  const open = async () => {
    setState('opening')
    const result = await store.openCandidate(entry.objId)
    if (result.ok) {
      setState('idle')
      nav.openDetail(entry.objId)
    } else {
      setState('failed')
    }
  }
  const selection = {
    unscreened: t('homestation.catchup.unscreened', 'Not screened yet'),
    not_selected: t('homestation.catchup.notSelected', 'Not selected for your feed'),
    preparing: t('homestation.catchup.preparing', 'Preparing resources'),
  }[entry.selection]
  return (
    <div className="hs-meta mt-3" data-testid="hs-candidate-footer" data-selection={entry.selection}>
      <span className="hs-badge" data-testid="hs-candidate-state">
        {entry.selection === 'preparing' ? <Loader2 size={12} className="animate-spin" /> : null}
        {selection}
      </span>
      <span>
        {entry.sourcePaths.map(path => (path.transport === 'pull' ? t('homestation.catchup.viaPull', 'read from {{name}}’s feed', { name: path.label }) : t('homestation.catchup.viaPush', 'delivered by {{name}}', { name: path.label }))).join(' · ')}
        {' · '}
        {t('homestation.catchup.arrived', 'arrived {{time}} ago', { time: formatTimeAgo(t, entry.arrivedAt, now) })}
      </span>
      {entry.openedAt ? <span>{t('homestation.catchup.read', 'Read here before')}</span> : null}
      <span className="ml-auto flex items-center gap-3">
        {state === 'failed' ? <span className="hs-badge is-warning" data-testid="hs-candidate-failed"><CircleAlert size={12} />{t('homestation.catchup.failed', 'Resources unavailable right now')}</span> : null}
        <button type="button" className="hs-btn is-small" disabled={state === 'opening'} onClick={() => void open()} data-testid="hs-candidate-open">
          {state === 'opening' ? <Loader2 size={14} className="animate-spin" /> : state === 'failed' ? <RefreshCw size={14} /> : null}
          {state === 'opening' ? t('homestation.catchup.opening', 'Preparing…') : state === 'failed' ? t('common.retry', 'Retry') : t('homestation.catchup.open', 'Open')}
        </button>
      </span>
    </div>
  )
}

export function FollowedCandidates() {
  const { t } = useI18n()
  const store = useHomeStationStore()
  const nav = useHsNav()
  const now = useNow()
  const [includeRead, setIncludeRead] = useState(false)
  const fetchPage = useCallback((scope: string, cursor: string | null) => store.listFollowedCandidates(cursor, { includeRead: scope === 'all' }), [store])
  const list = usePagedList<CandidatePage>(includeRead ? 'all' : 'unread', fetchPage, ['candidates'])
  const entries = useMemo(() => list.pages.flatMap(page => page.entries), [list.pages])
  const meta = list.pages[0]
  return (
    <div className="flex h-full min-h-0 flex-col" data-testid="hs-candidates">
      <PageHeader title={t('homestation.nav.catchup', 'Catch up')} onBack={nav.isDesktop ? undefined : nav.back} backLabel={t('common.back', 'Back')} />
      <ColumnBody>
        <div className={nav.isDesktop ? 'px-6 py-4' : 'px-4 py-4'} style={{ borderBottom: '1px solid var(--hs-line)' }}>
          <p className="text-[13px] leading-5" style={{ color: 'var(--cp-muted)' }}>{t('homestation.catchup.subtitle', 'Fetched but not in your feed. Following makes collection reliable; it doesn’t promise everything enters your feed.')}</p>
          <div className="hs-meta mt-3">
            {meta ? <span>{t('homestation.catchup.sync', 'Kept for {{days}} days · last fetch {{time}} ago · not the sources’ full history', { days: meta.retentionDays, time: formatTimeAgo(t, meta.lastFetchAt, now) })}</span> : null}
            <label className="flex items-center gap-2" style={{ color: 'var(--cp-text)' }}>
              <input type="checkbox" className="h-4 w-4" checked={includeRead} onChange={event => setIncludeRead(event.target.checked)} data-testid="hs-candidates-include-read" />
              {t('homestation.catchup.includeRead', 'Include ones I already read here ({{n}})', { n: meta?.readCount ?? 0 })}
            </label>
          </div>
        </div>
        {list.isLoading ? <ListSkeleton rows={3} /> : null}
        {list.error ? <ErrorState onRetry={list.reload} /> : null}
        {!list.isLoading && !list.error && entries.length === 0 ? <EmptyState icon={<Inbox size={28} />} title={t('homestation.catchup.empty', 'Nothing to catch up on')} body={t('homestation.catchup.emptyBody', 'Hidden people, filtered items and withdrawn posts never show up here.')} /> : null}
        {entries.map(entry => <FeedCard key={entry.objId} objId={entry.objId} variant="candidate" footer={<CandidateFooter entry={entry} />} />)}
        {list.hasMore ? <div className="flex justify-center py-4"><button type="button" className="hs-btn" onClick={list.loadMore}>{t('homestation.state.loadMore', 'Load more')}</button></div> : null}
        <div className="h-6" />
      </ColumnBody>
    </div>
  )
}
