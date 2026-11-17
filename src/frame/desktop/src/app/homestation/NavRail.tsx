import clsx from 'clsx'
import { Bookmark, Clock, Filter, Home, Inbox, Menu as MenuIcon, Plus, Rss, Search, Send, User, Waypoints } from 'lucide-react'
import { useState } from 'react'
import { useI18n } from '../../i18n/provider'
import { useHsNav, type HsPage } from './navContext'
import { useStoreSelector } from './store/context'
import type { HomeStationStore } from './store/types'
import { Menu, type MenuItemSpec } from './ui/Popover'
import { Avatar } from './ui/primitives'

const selectSync = (store: HomeStationStore) => store.peekSyncStatus()
const selectOwner = (store: HomeStationStore) => store.peekIdentity(store.owner)

interface NavProps {
  searchOpen: boolean
  onSearch: () => void
}

function currentName(page: HsPage) {
  if (page.name === 'detail') return 'feed'
  if (page.name === 'saved') return `saved:${page.kind}`
  return page.name
}

function useMoreItems() {
  const { t } = useI18n()
  const nav = useHsNav()
  const current = currentName(nav.page)
  const entries: { page: HsPage; key: string; label: string; icon: React.ReactNode; testId: string }[] = [
    { page: { name: 'published' }, key: 'published', label: t('homestation.nav.published', 'My publications'), icon: <Send size={16} />, testId: 'hs-nav-published' },
    { page: { name: 'saved', kind: 'bookmark' }, key: 'saved:bookmark', label: t('homestation.nav.bookmarks', 'Bookmarks'), icon: <Bookmark size={16} />, testId: 'hs-nav-bookmarks' },
    { page: { name: 'saved', kind: 'read_later' }, key: 'saved:read_later', label: t('homestation.nav.readLater', 'Read later'), icon: <Clock size={16} />, testId: 'hs-nav-read-later' },
    { page: { name: 'sources' }, key: 'sources', label: t('homestation.nav.sources', 'Sources'), icon: <Rss size={16} />, testId: 'hs-nav-sources' },
    { page: { name: 'prefs' }, key: 'prefs', label: t('homestation.nav.prefsShort', 'Filters & hidden'), icon: <Filter size={16} />, testId: 'hs-nav-prefs' },
  ]
  const items: MenuItemSpec[] = entries.map(entry => ({
    id: entry.key,
    label: entry.label,
    icon: entry.icon,
    current: current === entry.key,
    testId: entry.testId,
    onSelect: () => nav.navigate(entry.page, { reset: true }),
  }))
  return { items, active: entries.some(entry => entry.key === current) }
}

function RailButton({ label, current, pressed, onClick, testId, dot, className, children }: { label: string; current?: boolean; pressed?: boolean; onClick: () => void; testId: string; dot?: boolean; className?: string; children: React.ReactNode }) {
  return (
    <button
      type="button"
      className={clsx('hs-rail-btn', className)}
      title={label}
      aria-label={label}
      aria-current={current ? 'page' : undefined}
      aria-pressed={pressed}
      onClick={onClick}
      data-testid={testId}
    >
      {children}
      {dot ? <span className="hs-unread-dot" style={{ top: 12, right: 14 }} aria-hidden="true" /> : null}
    </button>
  )
}

/** Desktop: a narrow rail with the five places people go most; the rest live under "More". */
export function NavRail({ searchOpen, onSearch }: NavProps) {
  const { t } = useI18n()
  const nav = useHsNav()
  const sync = useStoreSelector(selectSync)
  const [moreAnchor, setMoreAnchor] = useState<HTMLElement | null>(null)
  const more = useMoreItems()
  const current = currentName(nav.page)
  const goHome = () => nav.navigate({ name: 'feed' }, { reset: true })
  return (
    <nav className="flex h-full w-[72px] flex-shrink-0 flex-col items-center py-4" aria-label={t('homestation.nav.label', 'HomeStation')} data-testid="hs-sidebar">
      <button type="button" className="hs-logo flex h-12 w-12 items-center justify-center rounded-xl transition-transform active:scale-95" aria-label={t('homestation.title', 'HomeStation')} title={t('homestation.title', 'HomeStation')} onClick={goHome}>
        <Waypoints size={28} />
      </button>
      <div className="flex flex-1 flex-col items-center justify-center gap-2">
        <RailButton label={t('homestation.nav.feed', 'Reading feed')} current={current === 'feed' && !searchOpen} onClick={goHome} testId="hs-nav-feed">
          <Home size={24} />
        </RailButton>
        <RailButton label={t('homestation.nav.search', 'Search')} pressed={searchOpen} onClick={onSearch} testId="hs-search-toggle">
          <Search size={24} />
        </RailButton>
        <RailButton label={t('homestation.publish.title', 'New post')} current={current === 'publish'} className="is-create" onClick={() => nav.navigate({ name: 'publish' }, { reset: true })} testId="hs-publish-open">
          <span><Plus size={22} /></span>
        </RailButton>
        <RailButton
          label={sync.candidates ? t('homestation.nav.catchupCount', 'Catch up · {{n}} new', { n: sync.candidates }) : t('homestation.nav.catchup', 'Catch up')}
          current={current === 'candidates'}
          dot={sync.candidates > 0}
          onClick={() => nav.navigate({ name: 'candidates' }, { reset: true })}
          testId="hs-nav-candidates"
        >
          <Inbox size={24} />
        </RailButton>
        <RailButton label={t('homestation.nav.profile', 'My homepage')} current={current === 'profile'} onClick={() => nav.navigate({ name: 'profile' }, { reset: true })} testId="hs-nav-profile">
          <User size={24} />
        </RailButton>
      </div>
      <button
        type="button"
        className={clsx('hs-rail-btn', more.active && 'is-current')}
        title={t('homestation.nav.more', 'More')}
        aria-label={t('homestation.nav.more', 'More')}
        aria-haspopup="menu"
        aria-expanded={!!moreAnchor}
        onClick={event => setMoreAnchor(moreAnchor ? null : event.currentTarget)}
        data-testid="hs-nav-more"
      >
        <MenuIcon size={24} />
      </button>
      <Menu anchor={moreAnchor} open={!!moreAnchor} onClose={() => setMoreAnchor(null)} items={more.items} label={t('homestation.nav.more', 'More')} testId="hs-nav-more-menu" align="start" />
    </nav>
  )
}

/** Phone: five tabs along the bottom, the same five places as the desktop rail. */
export function MobileTabBar({ searchOpen, onSearch }: NavProps) {
  const { t } = useI18n()
  const nav = useHsNav()
  const sync = useStoreSelector(selectSync)
  const owner = useStoreSelector(selectOwner)
  const current = currentName(nav.page)
  const me = current === 'me'
  return (
    <nav className="hs-tabbar flex-shrink-0" aria-label={t('homestation.nav.label', 'HomeStation')} data-testid="hs-tabbar">
      <button type="button" className="hs-tabbar-btn" aria-label={t('homestation.nav.feed', 'Reading feed')} aria-current={current === 'feed' && !searchOpen ? 'page' : undefined} onClick={() => nav.navigate({ name: 'feed' }, { reset: true })} data-testid="hs-tab-feed">
        <Home size={26} />
      </button>
      <button type="button" className="hs-tabbar-btn" aria-label={t('homestation.nav.search', 'Search')} aria-pressed={searchOpen} onClick={onSearch} data-testid="hs-search-toggle">
        <Search size={26} />
      </button>
      <button type="button" className="hs-tabbar-btn is-create" aria-label={t('homestation.publish.title', 'New post')} onClick={() => nav.navigate({ name: 'publish' })} data-testid="hs-publish-open">
        <span><Plus size={24} /></span>
      </button>
      <button type="button" className="hs-tabbar-btn" aria-label={t('homestation.nav.catchup', 'Catch up')} aria-current={current === 'candidates' ? 'page' : undefined} onClick={() => nav.navigate({ name: 'candidates' }, { reset: true })} data-testid="hs-tab-candidates">
        <Inbox size={26} />
        {sync.candidates > 0 ? <span className="hs-unread-dot" style={{ top: 14, right: 'calc(50% - 16px)' }} aria-hidden="true" /> : null}
      </button>
      <button type="button" className="hs-tabbar-btn" aria-label={t('homestation.me.title', 'Me')} aria-current={me ? 'page' : undefined} onClick={() => nav.navigate({ name: 'me' }, { reset: true })} data-testid="hs-avatar">
        <span className="rounded-full" style={{ boxShadow: me ? '0 0 0 2px var(--hs-column), 0 0 0 3px var(--cp-text)' : undefined }}>
          <Avatar identity={owner} size={26} />
        </span>
      </button>
    </nav>
  )
}
