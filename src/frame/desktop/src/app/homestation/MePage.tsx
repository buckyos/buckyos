import { Bookmark, ChevronRight, Clock, Filter, Inbox, Rss, Send, User } from 'lucide-react'
import { useCallback } from 'react'
import { useI18n } from '../../i18n/provider'
import { formatCount } from './datamodel/format'
import type { HomeStationStore } from './store/types'
import { useHsNav, type HsPage } from './navContext'
import { useStoreSelector } from './store/context'
import { Avatar, PageHeader } from './ui/primitives'

const selectSync = (store: HomeStationStore) => store.peekSyncStatus()

export function MePage() {
  const { t } = useI18n()
  const nav = useHsNav()
  const sync = useStoreSelector(selectSync)
  const selectOwner = useCallback((store: HomeStationStore) => store.peekIdentity(store.owner), [])
  const owner = useStoreSelector(selectOwner)
  const items: { page: HsPage; label: string; hint: string; icon: React.ReactNode; testId: string }[] = [
    { page: { name: 'profile' }, label: t('homestation.nav.profile', 'My homepage'), hint: t('homestation.me.profileHint', 'Your home feed as visitors see it'), icon: <User size={18} />, testId: 'hs-me-profile' },
    { page: { name: 'published' }, label: t('homestation.nav.published', 'My publications'), hint: t('homestation.me.publishedHint', 'Posts, comments, reposts and likes with delivery state'), icon: <Send size={18} />, testId: 'hs-me-published' },
    { page: { name: 'candidates' }, label: t('homestation.nav.catchup', 'Catch up'), hint: t('homestation.me.catchupHint', '{{n}} from people you follow, not in your feed', { n: formatCount(sync.candidates) }), icon: <Inbox size={18} />, testId: 'hs-me-candidates' },
    { page: { name: 'saved', kind: 'bookmark' }, label: t('homestation.nav.bookmarks', 'Bookmarks'), hint: t('homestation.me.bookmarksHint', 'Private by default'), icon: <Bookmark size={18} />, testId: 'hs-me-bookmarks' },
    { page: { name: 'saved', kind: 'read_later' }, label: t('homestation.nav.readLater', 'Read later'), hint: t('homestation.me.readLaterHint', 'Only you can see it'), icon: <Clock size={18} />, testId: 'hs-me-read-later' },
    { page: { name: 'sources' }, label: t('homestation.nav.sources', 'Sources'), hint: t('homestation.me.sourcesHint', '{{n}} sources · {{failing}} with errors', { n: sync.sources, failing: sync.failingSources }), icon: <Rss size={18} />, testId: 'hs-me-sources' },
    { page: { name: 'prefs' }, label: t('homestation.nav.prefsShort', 'Filters & hidden'), hint: t('homestation.me.prefsHint', 'AI-content filters and people you don’t show'), icon: <Filter size={18} />, testId: 'hs-me-prefs' },
  ]
  return (
    <div className="flex h-full min-h-0 flex-col" data-testid="hs-me">
      <PageHeader title={t('homestation.me.title', 'Me')} onBack={nav.back} backLabel={t('common.back', 'Back')} />
      <div className="desktop-scrollbar min-h-0 flex-1 overflow-y-auto">
        <div className="flex items-center gap-3 px-4 py-4">
          <Avatar identity={owner} size={52} />
          <div className="min-w-0">
            <p className="truncate text-base font-semibold">{owner.name}</p>
            <p className="truncate font-mono text-[11px]" style={{ color: 'var(--cp-muted)' }}>{owner.did}</p>
          </div>
        </div>
        <ul className="px-2">
          {items.map(item => (
            <li key={item.testId}>
              <button type="button" className="hs-nav-item min-h-[56px]" onClick={() => nav.navigate(item.page)} data-testid={item.testId}>
                <span style={{ color: 'var(--cp-accent)' }}>{item.icon}</span>
                <span className="min-w-0 flex-1">
                  <span className="block text-sm font-medium">{item.label}</span>
                  <span className="block truncate text-[11px]" style={{ color: 'var(--cp-muted)' }}>{item.hint}</span>
                </span>
                <ChevronRight size={16} style={{ color: 'var(--cp-muted)' }} />
              </button>
            </li>
          ))}
        </ul>
      </div>
    </div>
  )
}
