import { Bookmark, Clock, Filter, Hash, Home, Inbox, Rss, Send, User } from 'lucide-react'
import { useCallback } from 'react'
import { useI18n } from '../../i18n/provider'
import type { HomeStationStore } from './mock/store'
import { useHsNav, type HsPage } from './navContext'
import { useStoreSelector } from './store/context'
import { Avatar } from './ui/primitives'

const selectSync = (store: HomeStationStore) => store.peekSyncStatus()
const selectTopics = (store: HomeStationStore) => store.peekTopics()

export function SidebarPanel({ collapsed, activeTopicId, onSelectTopic }: { collapsed: boolean; activeTopicId: string | null; onSelectTopic: (topicId: string | null) => void }) {
  const { t } = useI18n()
  const nav = useHsNav()
  const sync = useStoreSelector(selectSync)
  const topics = useStoreSelector(selectTopics)
  const selectOwner = useCallback((store: HomeStationStore) => store.peekIdentity(store.owner), [])
  const owner = useStoreSelector(selectOwner)
  const current = nav.page.name === 'detail' ? 'feed' : nav.page.name
  const savedKind = nav.page.name === 'saved' ? nav.page.kind : null
  const items: { page: HsPage; label: string; icon: React.ReactNode; badge?: number; testId: string }[] = [
    { page: { name: 'feed' }, label: t('homestation.nav.feed', 'Reading feed'), icon: <Home size={16} />, testId: 'hs-nav-feed' },
    { page: { name: 'profile' }, label: t('homestation.nav.profile', 'My homepage'), icon: <User size={16} />, testId: 'hs-nav-profile' },
    { page: { name: 'published' }, label: t('homestation.nav.published', 'My publications'), icon: <Send size={16} />, testId: 'hs-nav-published' },
    { page: { name: 'candidates' }, label: t('homestation.nav.catchup', 'Catch up'), icon: <Inbox size={16} />, badge: sync.candidates, testId: 'hs-nav-candidates' },
    { page: { name: 'saved', kind: 'bookmark' }, label: t('homestation.nav.bookmarks', 'Bookmarks'), icon: <Bookmark size={16} />, testId: 'hs-nav-bookmarks' },
    { page: { name: 'saved', kind: 'read_later' }, label: t('homestation.nav.readLater', 'Read later'), icon: <Clock size={16} />, testId: 'hs-nav-read-later' },
    { page: { name: 'sources' }, label: t('homestation.nav.sources', 'Sources'), icon: <Rss size={16} />, testId: 'hs-nav-sources' },
    { page: { name: 'prefs' }, label: t('homestation.nav.prefsShort', 'Filters & hidden'), icon: <Filter size={16} />, testId: 'hs-nav-prefs' },
  ]
  const isCurrent = (page: HsPage) => page.name === current && (page.name !== 'saved' || page.kind === savedKind)
  return (
    <nav className="desktop-scrollbar flex h-full flex-col overflow-y-auto" style={{ background: 'color-mix(in srgb, var(--cp-surface) 70%, var(--cp-bg))', borderRight: '1px solid var(--hs-divider)' }} aria-label={t('homestation.nav.label', 'HomeStation')} data-testid="hs-sidebar">
      <div className="flex items-center gap-2 px-3 py-3" style={{ borderBottom: '1px solid var(--hs-divider)' }}>
        <Avatar identity={owner} size={34} />
        {!collapsed ? (
          <div className="min-w-0">
            <p className="truncate text-sm font-semibold">{owner.name}</p>
            <p className="truncate text-[11px]" style={{ color: 'var(--cp-muted)' }}>{t('homestation.title', 'HomeStation')}</p>
          </div>
        ) : null}
      </div>
      <div className="flex flex-col gap-0.5 px-2 py-2">
        {items.map(item => (
          <button
            key={item.testId}
            type="button"
            className="hs-nav-item"
            aria-current={isCurrent(item.page) ? 'page' : undefined}
            title={collapsed ? item.label : undefined}
            aria-label={collapsed ? item.label : undefined}
            data-testid={item.testId}
            onClick={() => nav.navigate(item.page, { reset: true })}
          >
            <span style={{ color: 'var(--cp-muted)' }}>{item.icon}</span>
            {!collapsed ? <span className="flex-1 truncate">{item.label}</span> : null}
            {!collapsed && item.badge ? <span className="hs-badge">{item.badge}</span> : null}
          </button>
        ))}
      </div>
      {!collapsed ? (
        <div className="px-2 pb-4 pt-2">
          <p className="hs-section-title px-3 pb-1">{t('homestation.topics.title', 'Topics')}</p>
          <p className="px-3 pb-1 text-[10px]" style={{ color: 'var(--cp-muted)' }}>{t('homestation.topics.scope', 'Counts: your feed · last 7 days')}</p>
          {topics.map(topic => (
            <button
              key={topic.id}
              type="button"
              className="hs-nav-item py-1.5"
              aria-current={activeTopicId === topic.id && current === 'feed' ? 'page' : undefined}
              onClick={() => onSelectTopic(activeTopicId === topic.id ? null : topic.id)}
            >
              <Hash size={13} style={{ color: 'var(--cp-muted)' }} />
              <span className="flex-1 truncate">{topic.name}</span>
              <span className="text-[11px]" style={{ color: 'var(--cp-muted)' }}>{topic.recentCount}</span>
            </button>
          ))}
        </div>
      ) : null}
    </nav>
  )
}
