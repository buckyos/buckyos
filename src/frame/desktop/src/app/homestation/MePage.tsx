import { Bookmark, ChevronRight, Clock, Filter, Inbox, Rss, Send, User } from 'lucide-react'
import { useCallback } from 'react'
import { useI18n } from '../../i18n/provider'
import { formatCount } from './datamodel/format'
import type { HomeStationStore } from './store/types'
import { useHsNav, type HsPage } from './navContext'
import { useStoreSelector } from './store/context'
import { Avatar, ColumnBody, PageHeader } from './ui/primitives'

const selectSync = (store: HomeStationStore) => store.peekSyncStatus()

export function MePage() {
  const { t } = useI18n()
  const nav = useHsNav()
  const sync = useStoreSelector(selectSync)
  const selectOwner = useCallback((store: HomeStationStore) => store.peekIdentity(store.owner), [])
  const owner = useStoreSelector(selectOwner)
  const items: { page: HsPage; label: string; hint: string; icon: React.ReactNode; testId: string }[] = [
    { page: { name: 'profile' }, label: t('homestation.nav.profile', 'My homepage'), hint: t('homestation.me.profileHint', 'Your home feed as visitors see it'), icon: <User size={20} />, testId: 'hs-me-profile' },
    { page: { name: 'published' }, label: t('homestation.nav.published', 'My publications'), hint: t('homestation.me.publishedHint', 'Posts, comments, reposts and likes with delivery state'), icon: <Send size={20} />, testId: 'hs-me-published' },
    { page: { name: 'candidates' }, label: t('homestation.nav.catchup', 'Catch up'), hint: t('homestation.me.catchupHint', '{{n}} from people you follow, not in your feed', { n: formatCount(sync.candidates) }), icon: <Inbox size={20} />, testId: 'hs-me-candidates' },
    { page: { name: 'saved', kind: 'bookmark' }, label: t('homestation.nav.bookmarks', 'Bookmarks'), hint: t('homestation.me.bookmarksHint', 'Private by default'), icon: <Bookmark size={20} />, testId: 'hs-me-bookmarks' },
    { page: { name: 'saved', kind: 'read_later' }, label: t('homestation.nav.readLater', 'Read later'), hint: t('homestation.me.readLaterHint', 'Only you can see it'), icon: <Clock size={20} />, testId: 'hs-me-read-later' },
    { page: { name: 'sources' }, label: t('homestation.nav.sources', 'Sources'), hint: t('homestation.me.sourcesHint', '{{n}} sources · {{failing}} with errors', { n: sync.sources, failing: sync.failingSources }), icon: <Rss size={20} />, testId: 'hs-me-sources' },
    { page: { name: 'prefs' }, label: t('homestation.nav.prefsShort', 'Filters & hidden'), hint: t('homestation.me.prefsHint', 'AI-content filters and people you don’t show'), icon: <Filter size={20} />, testId: 'hs-me-prefs' },
  ]
  return (
    <div className="flex h-full min-h-0 flex-col" data-testid="hs-me">
      <PageHeader title={t('homestation.me.title', 'Me')} onBack={nav.inTab ? nav.back : undefined} backLabel={t('common.back', 'Back')} />
      <ColumnBody>
        <div className="flex items-start gap-4 px-4 pb-6 pt-6">
          <div className="min-w-0 flex-1">
            <p className="truncate text-[24px] font-semibold leading-8">{owner.name}</p>
            <p className="mt-1 truncate font-mono text-[13px] leading-5" style={{ color: 'var(--cp-muted)' }}>{owner.did}</p>
          </div>
          <Avatar identity={owner} size={72} />
        </div>
        <ul style={{ borderTop: '1px solid var(--hs-line)' }}>
          {items.map(item => (
            <li key={item.testId} style={{ borderBottom: '1px solid var(--hs-line)' }}>
              <button type="button" className="flex min-h-[64px] w-full items-center gap-4 px-4 py-3 text-left transition-colors active:bg-[var(--hs-press-bg)]" onClick={() => nav.navigate(item.page)} data-testid={item.testId}>
                <span style={{ color: 'var(--cp-muted)' }}>{item.icon}</span>
                <span className="min-w-0 flex-1">
                  <span className="block text-[15px] font-medium leading-[22px]">{item.label}</span>
                  <span className="block truncate text-[13px] leading-5" style={{ color: 'var(--cp-muted)' }}>{item.hint}</span>
                </span>
                <ChevronRight size={18} style={{ color: 'var(--cp-muted)' }} />
              </button>
            </li>
          ))}
        </ul>
        <div className="h-6" />
      </ColumnBody>
    </div>
  )
}
