import { useMediaQuery } from '@mui/material'
import { ExternalLink, Hash, Link2, PanelRightOpen, PenSquare, Search, X } from 'lucide-react'
import { useCallback, useMemo, useRef, useState } from 'react'
import { WindowDialogProvider } from '../../desktop/windows/dialogs'
import { useI18n } from '../../i18n/provider'
import './homestation.css'
import { FollowedCandidates } from './candidates/FollowedCandidates'
import type { ReadingQuery } from './datamodel/types'
import { ItemDetail } from './detail/ItemDetail'
import { FeedPage } from './feed/FeedPage'
import { ImmersiveMode } from './ImmersiveMode'
import { InfoPanel } from './InfoPanel'
import { INFO_PANEL_DEFAULT_WIDTH, INFO_PANEL_HIDE_BELOW, INFO_PANEL_MAX_WIDTH, INFO_PANEL_MIN_WIDTH, PANEL_SPLITTER_WIDTH, SIDEBAR_COLLAPSE_BELOW, SIDEBAR_COLLAPSED_WIDTH, SIDEBAR_WIDTH } from './layout'
import { portalHref, portalShareUrl } from './links'
import { MePage } from './MePage'
import { installHomeStationPreviewSource } from './media'
import { HsNavContext, useHsNav, type HsNav, type HsPage } from './navContext'
import { PublicProfileView, type PreviewReader } from './PublicProfileView'
import { FeedPreferences } from './prefs/FeedPreferences'
import { MyPublications } from './publish/MyPublications'
import { PublishComposer } from './publish/PublishComposer'
import { SavedList } from './saved/SavedList'
import { SidebarPanel } from './SidebarPanel'
import { SourceManager } from './source/SourceManager'
import { useHomeStationStore, useStoreSelector } from './store/context'
import { HomeStationStoreProvider } from './store/HomeStationStoreProvider'
import type { HomeStationStore } from './store/types'
import type { ReadingMode } from './types'
import { Avatar, PageHeader } from './ui/primitives'
import { ToastHost } from './ui/ToastHost'
import { useToast } from './ui/toastContext'

installHomeStationPreviewSource()

const OWNER_READER = { kind: 'owner' } as const
const DEFAULT_QUERY: ReadingQuery = { filter: 'all', topicId: null, search: '', showFiltered: false }
const selectTopics = (store: HomeStationStore) => store.peekTopics()
const selectOwner = (store: HomeStationStore) => store.peekIdentity(store.owner)
const selectHome = (store: HomeStationStore) => store.peekHome()

function TopBar({ query, onQueryChange, showSearch, onToggleSearch, onAvatar, trailing, isMobile }: { query: ReadingQuery; onQueryChange: (patch: Partial<ReadingQuery>) => void; showSearch: boolean; onToggleSearch: () => void; onAvatar?: () => void; trailing?: React.ReactNode; isMobile: boolean }) {
  const { t } = useI18n()
  const owner = useStoreSelector(selectOwner)
  const topics = useStoreSelector(selectTopics)
  return (
    <div style={{ borderBottom: '1px solid var(--hs-divider)' }}>
      <div className="flex items-center gap-3 px-4 py-2">
        <button type="button" className="flex-shrink-0 rounded-full" onClick={onAvatar} aria-label={t('homestation.me.title', 'Me')} data-testid="hs-avatar">
          <Avatar identity={owner} size={36} />
        </button>
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-semibold">{owner.name}</p>
          <p className="truncate text-[11px]" style={{ color: 'var(--cp-muted)' }}>{t('homestation.topbar.subtitle', 'Your reading feed · HomeStation')}</p>
        </div>
        <button type="button" className="hs-icon-btn" aria-pressed={showSearch} aria-label={t('homestation.search.toggle', 'Search in this view')} onClick={onToggleSearch} data-testid="hs-search-toggle">
          <Search size={18} />
        </button>
        {trailing}
      </div>
      {showSearch ? (
        <div className="px-4 pb-2">
          <div className="flex items-center gap-2 rounded-xl px-3 py-1.5" style={{ background: 'var(--hs-subtle-bg)' }}>
            <Search size={15} style={{ color: 'var(--cp-muted)' }} />
            <input
              type="search"
              value={query.search}
              autoFocus
              onChange={event => onQueryChange({ search: event.target.value })}
              placeholder={t('homestation.search.placeholder', 'Search in this view (title, text, author)')}
              aria-label={t('homestation.search.placeholder', 'Search in this view (title, text, author)')}
              className="min-w-0 flex-1 bg-transparent text-sm outline-none"
              data-testid="hs-search-input"
            />
            <button type="button" aria-label={t('homestation.search.clear', 'Clear search')} onClick={() => { onQueryChange({ search: '' }); onToggleSearch() }} style={{ color: 'var(--cp-muted)' }}>
              <X size={15} />
            </button>
          </div>
          {isMobile ? (
            <div className="hs-scroll-x mt-2 flex gap-1.5 overflow-x-auto">
              {topics.map(topic => (
                <button key={topic.id} type="button" className="hs-chip" aria-pressed={query.topicId === topic.id} onClick={() => onQueryChange({ topicId: query.topicId === topic.id ? null : topic.id })} data-testid={`hs-topic-${topic.id}`}>
                  <Hash size={12} />
                  {topic.name}
                </button>
              ))}
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  )
}

function PageContent({ page, query, onQueryChange, readingMode, onReadingModeChange, isMobile, header, previewReader, onPreviewReaderChange }: { page: HsPage; query: ReadingQuery; onQueryChange: (patch: Partial<ReadingQuery>) => void; readingMode: ReadingMode; onReadingModeChange: (mode: ReadingMode) => void; isMobile: boolean; header?: React.ReactNode; previewReader: PreviewReader; onPreviewReaderChange: (reader: PreviewReader) => void }) {
  const { t } = useI18n()
  const store = useHomeStationStore()
  const topics = useStoreSelector(selectTopics)
  switch (page.name) {
    case 'feed':
      return <FeedPage query={query} onQueryChange={onQueryChange} readingMode={readingMode} onReadingModeChange={onReadingModeChange} topics={topics} isMobile={isMobile} header={header} />
    case 'detail':
      return <ItemDetail key={page.objId} objId={page.objId} />
    case 'profile':
      return (
        <div className="flex h-full min-h-0 flex-col">
          <ProfileHeader />
          <div className="desktop-scrollbar mx-auto min-h-0 w-full max-w-[760px] flex-1 overflow-y-auto">
            <PublicProfileView owner={store.owner} previewReader={previewReader} onPreviewReaderChange={onPreviewReaderChange} />
            <div className="h-20" />
          </div>
        </div>
      )
    case 'published':
      return <MyPublications />
    case 'candidates':
      return <FollowedCandidates />
    case 'saved':
      return <SavedList key={page.kind} kind={page.kind} />
    case 'sources':
      return <SourceManager />
    case 'prefs':
      return <FeedPreferences />
    case 'me':
      return <MePage />
    case 'publish':
      return (
        <div className="flex h-full min-h-0 flex-col">
          <PublishHeader />
          <div className="desktop-scrollbar min-h-0 flex-1 overflow-y-auto"><PublishComposer /></div>
        </div>
      )
    default:
      return <p className="p-4 text-sm">{t('homestation.state.unknownPage', 'Unknown page')}</p>
  }
}

function ProfileHeader() {
  const { t } = useI18n()
  const nav = useHsNav()
  const toast = useToast()
  const home = useStoreSelector(selectHome)
  const actions = home ? (
    <>
      <a className="hs-icon-btn" href={portalHref(home.user)} target="_blank" rel="noreferrer" title={t('homestation.profile.openPublic', 'Open my public page')} aria-label={t('homestation.profile.openPublic', 'Open my public page')} data-testid="hs-profile-open-public">
        <ExternalLink size={16} />
      </a>
      <button
        type="button"
        className="hs-icon-btn"
        title={t('homestation.profile.copyLink', 'Copy link to my page')}
        aria-label={t('homestation.profile.copyLink', 'Copy link to my page')}
        data-testid="hs-profile-copy-link"
        onClick={() => {
          const url = portalShareUrl(home.user)
          void navigator.clipboard?.writeText(url).then(() => toast({ text: t('homestation.profile.linkCopied', 'Link copied: {{url}}', { url }) }), () => toast({ text: url }))
        }}
      >
        <Link2 size={16} />
      </button>
    </>
  ) : undefined
  return <PageHeader title={t('homestation.nav.profile', 'My homepage')} subtitle={t('homestation.profile.subtitle', 'Your home feed, read the way visitors read it')} onBack={nav.isDesktop ? undefined : nav.back} backLabel={t('common.back', 'Back')} actions={actions} />
}

function PublishHeader() {
  const { t } = useI18n()
  const nav = useHsNav()
  return <PageHeader title={t('homestation.publish.title', 'New post')} onBack={nav.back} backLabel={t('common.back', 'Back')} />
}

export function HomeStationView() {
  const { t } = useI18n()
  const isDesktop = useMediaQuery('(min-width: 769px)')
  const [pages, setPages] = useState<HsPage[]>([{ name: 'feed' }])
  const [query, setQuery] = useState<ReadingQuery>(DEFAULT_QUERY)
  const [readingMode, setReadingMode] = useState<ReadingMode>('standard')
  const [showSearch, setShowSearch] = useState(false)
  const [previewReader, setPreviewReader] = useState<PreviewReader>('owner')
  const [width, setWidth] = useState(1280)
  const [infoPref, setInfoPref] = useState<'auto' | 'open' | 'closed'>('auto')
  const [infoWidth, setInfoWidth] = useState(INFO_PANEL_DEFAULT_WIDTH)
  const [resizing, setResizing] = useState(false)
  const resizeRef = useRef<{ pointerId: number; startX: number; startWidth: number } | null>(null)
  const observerRef = useRef<ResizeObserver | null>(null)

  const layoutRef = useCallback((element: HTMLDivElement | null) => {
    observerRef.current?.disconnect()
    if (!element) return
    const observer = new ResizeObserver(entries => setWidth(entries[0]?.contentRect.width ?? 1280))
    observer.observe(element)
    observerRef.current = observer
  }, [])

  const page = pages[pages.length - 1]
  const onQueryChange = useCallback((patch: Partial<ReadingQuery>) => setQuery(current => ({ ...current, ...patch })), [])

  const nav = useMemo<HsNav>(() => ({
    page,
    perspective: 'owner',
    reader: OWNER_READER,
    isDesktop,
    navigate: (next, options) => setPages(current => (options?.reset ? (next.name === 'feed' ? [next] : [{ name: 'feed' }, next]) : [...current, next])),
    back: () => setPages(current => (current.length > 1 ? current.slice(0, -1) : current)),
    openDetail: objId => setPages(current => [...current, { name: 'detail', objId }]),
    showFilteredInFeed: () => {
      setQuery(current => ({ ...current, showFiltered: true }))
      setPages([{ name: 'feed' }])
    },
  }), [isDesktop, page])

  const docked = width >= INFO_PANEL_HIDE_BELOW
  const infoOpen = infoPref === 'open' || (infoPref === 'auto' && docked)
  const sidebarCollapsed = width < SIDEBAR_COLLAPSE_BELOW

  const selectTopic = (topicId: string | null) => {
    onQueryChange({ topicId })
    setPages([{ name: 'feed' }])
  }

  const onSplitterDown = (event: React.PointerEvent<HTMLButtonElement>) => {
    resizeRef.current = { pointerId: event.pointerId, startX: event.clientX, startWidth: infoWidth }
    setResizing(true)
    event.currentTarget.setPointerCapture(event.pointerId)
    event.preventDefault()
  }
  const onSplitterMove = (event: React.PointerEvent<HTMLButtonElement>) => {
    const state = resizeRef.current
    if (!state || state.pointerId !== event.pointerId) return
    setInfoWidth(Math.min(Math.max(state.startWidth - (event.clientX - state.startX), INFO_PANEL_MIN_WIDTH), INFO_PANEL_MAX_WIDTH))
  }
  const onSplitterUp = (event: React.PointerEvent<HTMLButtonElement>) => {
    if (!resizeRef.current || resizeRef.current.pointerId !== event.pointerId) return
    resizeRef.current = null
    setResizing(false)
    event.currentTarget.releasePointerCapture(event.pointerId)
  }

  const content = (
    <PageContent
      page={page}
      query={query}
      onQueryChange={onQueryChange}
      readingMode={readingMode}
      onReadingModeChange={setReadingMode}
      isMobile={!isDesktop}
      previewReader={previewReader}
      onPreviewReaderChange={setPreviewReader}
      header={isDesktop ? undefined : (
        <TopBar query={query} onQueryChange={onQueryChange} showSearch={showSearch} onToggleSearch={() => setShowSearch(value => !value)} onAvatar={() => nav.navigate({ name: 'me' })} isMobile />
      )}
    />
  )

  return (
    <HomeStationStoreProvider>
      <HsNavContext.Provider value={nav}>
        <div className="hs-root relative flex h-full w-full overflow-hidden" style={{ background: 'var(--cp-bg)', cursor: resizing ? 'col-resize' : undefined }} data-testid="homestation" ref={layoutRef}>
          <ToastHost>
            <WindowDialogProvider surface={isDesktop ? 'desktop' : 'mobile'} permissions={{ fullscreen: false }}>
              {isDesktop ? (
                <div className="flex h-full w-full min-w-0">
                  <div className="h-full flex-shrink-0" style={{ width: sidebarCollapsed ? SIDEBAR_COLLAPSED_WIDTH : SIDEBAR_WIDTH }}>
                    <SidebarPanel collapsed={sidebarCollapsed} activeTopicId={query.topicId} onSelectTopic={selectTopic} />
                  </div>
                  <main className="flex h-full min-w-0 flex-1 flex-col">
                    {page.name === 'feed' ? (
                      <TopBar
                        query={query}
                        onQueryChange={onQueryChange}
                        showSearch={showSearch}
                        onToggleSearch={() => setShowSearch(value => !value)}
                        onAvatar={() => nav.navigate({ name: 'profile' }, { reset: true })}
                        isMobile={false}
                        trailing={!infoOpen ? (
                          <button type="button" className="hs-icon-btn" aria-label={t('homestation.info.show', 'Show context panel')} onClick={() => setInfoPref('open')} data-testid="hs-info-open">
                            <PanelRightOpen size={17} />
                          </button>
                        ) : null}
                      />
                    ) : null}
                    {content}
                  </main>
                  {infoOpen && docked ? (
                    <button
                      type="button"
                      className="relative h-full flex-shrink-0"
                      style={{ width: PANEL_SPLITTER_WIDTH, marginLeft: -PANEL_SPLITTER_WIDTH / 2, marginRight: -PANEL_SPLITTER_WIDTH / 2, cursor: 'col-resize', zIndex: 5, touchAction: 'none' }}
                      aria-label={t('homestation.info.resize', 'Resize context panel')}
                      onPointerDown={onSplitterDown}
                      onPointerMove={onSplitterMove}
                      onPointerUp={onSplitterUp}
                      onPointerCancel={onSplitterUp}
                    >
                      <span className="pointer-events-none absolute inset-y-4 left-1/2 -translate-x-1/2 rounded-full" style={{ width: resizing ? 3 : 1, background: resizing ? 'var(--cp-accent)' : 'var(--hs-divider)' }} />
                    </button>
                  ) : null}
                  {infoOpen ? (
                    <div
                      className={docked ? 'h-full flex-shrink-0' : 'absolute inset-y-0 right-0 z-40 shadow-xl'}
                      style={{ width: infoWidth, background: docked ? 'var(--cp-bg)' : 'var(--hs-panel-bg)', borderLeft: '1px solid var(--hs-divider)' }}
                    >
                      <InfoPanel query={query} readingMode={readingMode} onQueryChange={onQueryChange} onClose={() => setInfoPref('closed')} />
                    </div>
                  ) : null}
                </div>
              ) : (
                // 待确认（TODO §13）：移动端是否改为 PRD 的底部五栏导航，临时保留顶栏 + 发布按钮，新页面入口放在“我”页
                <div className="relative flex h-full w-full min-w-0 flex-col">
                  {content}
                  {page.name === 'feed' ? (
                    <button
                      type="button"
                      onClick={() => nav.navigate({ name: 'publish' })}
                      className="absolute z-20 flex h-14 w-14 items-center justify-center rounded-full shadow-lg"
                      style={{ right: 16, bottom: 'calc(16px + var(--sab))', background: 'var(--cp-accent)', color: 'var(--hs-on-accent)' }}
                      aria-label={t('homestation.publish.title', 'New post')}
                      data-testid="hs-fab"
                    >
                      <PenSquare size={22} />
                    </button>
                  ) : null}
                </div>
              )}
              {readingMode === 'immersive' ? <ImmersiveMode query={query} onClose={() => setReadingMode('standard')} /> : null}
            </WindowDialogProvider>
          </ToastHost>
        </div>
      </HsNavContext.Provider>
    </HomeStationStoreProvider>
  )
}
