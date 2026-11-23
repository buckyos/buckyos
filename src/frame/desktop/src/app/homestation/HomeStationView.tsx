import { useMediaQuery } from '@mui/material'
import { ExternalLink, Link2, PanelRight, Waypoints } from 'lucide-react'
import { type CSSProperties, useCallback, useMemo, useRef, useState } from 'react'
import { WindowDialogProvider } from '../../desktop/windows/dialogs'
import { useI18n } from '../../i18n/provider'
import './homestation.css'
import { FollowedCandidates } from './candidates/FollowedCandidates'
import type { ReadingQuery } from './datamodel/types'
import { ItemDetail } from './detail/ItemDetail'
import { FeedPage } from './feed/FeedPage'
import { ReadingModeMenu } from './FilterBar'
import { ImmersiveMode } from './ImmersiveMode'
import { InfoPanel } from './InfoPanel'
import { COLUMN_GAP, COLUMN_MAX_WIDTH, INFO_PANEL_DEFAULT_WIDTH, INFO_PANEL_HIDE_BELOW, INFO_PANEL_MAX_WIDTH, INFO_PANEL_MIN_WIDTH } from './layout'
import { portalHref, portalShareUrl } from './links'
import { MePage } from './MePage'
import { installHomeStationPreviewSource } from './media'
import { MobileTabBar, NavRail } from './NavRail'
import { HsNavContext, useHsNav, type HsNav, type HsPage } from './navContext'
import { ComposePrefillContext, type HomeStationCompose } from './launch'
import { PublicProfileView, type PreviewReader } from './PublicProfileView'
import { FeedPreferences } from './prefs/FeedPreferences'
import { MyPublications } from './publish/MyPublications'
import { PublishComposer } from './publish/PublishComposer'
import { SavedList } from './saved/SavedList'
import { SourceManager } from './source/SourceManager'
import { useHomeStationStore, useStoreSelector } from './store/context'
import { HomeStationStoreProvider } from './store/HomeStationStoreProvider'
import type { HomeStationStore } from './store/types'
import type { ReadingMode } from './types'
import { ColumnBody, PageHeader } from './ui/primitives'
import { ToastHost } from './ui/ToastHost'
import { useToast } from './ui/toastContext'

installHomeStationPreviewSource()

const OWNER_READER = { kind: 'owner' } as const
const DEFAULT_QUERY: ReadingQuery = { filter: 'all', topicId: null, search: '', showFiltered: false }
const selectTopics = (store: HomeStationStore) => store.peekTopics()
const selectHome = (store: HomeStationStore) => store.peekHome()

function PageContent({ page, query, onQueryChange, readingMode, onReadingModeChange, isMobile, header, previewReader, onPreviewReaderChange, showSearch, onCloseSearch }: { page: HsPage; query: ReadingQuery; onQueryChange: (patch: Partial<ReadingQuery>) => void; readingMode: ReadingMode; onReadingModeChange: (mode: ReadingMode) => void; isMobile: boolean; header?: React.ReactNode; previewReader: PreviewReader; onPreviewReaderChange: (reader: PreviewReader) => void; showSearch: boolean; onCloseSearch: () => void }) {
  const { t } = useI18n()
  const store = useHomeStationStore()
  const topics = useStoreSelector(selectTopics)
  switch (page.name) {
    case 'feed':
      return <FeedPage query={query} onQueryChange={onQueryChange} readingMode={readingMode} onReadingModeChange={onReadingModeChange} topics={topics} isMobile={isMobile} header={header} showSearch={showSearch} onCloseSearch={onCloseSearch} />
    case 'detail':
      return <ItemDetail key={page.objId} objId={page.objId} />
    case 'profile':
      return (
        <div className="flex h-full min-h-0 flex-col">
          <ProfileHeader />
          <ColumnBody>
            <PublicProfileView owner={store.owner} previewReader={previewReader} onPreviewReaderChange={onPreviewReaderChange} />
            <div className="h-6" />
          </ColumnBody>
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
          <ColumnBody><PublishComposer /></ColumnBody>
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
        <ExternalLink size={18} />
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
        <Link2 size={18} />
      </button>
    </>
  ) : undefined
  return <PageHeader title={t('homestation.nav.profile', 'My homepage')} onBack={nav.isDesktop ? undefined : nav.back} backLabel={t('common.back', 'Back')} actions={actions} />
}

function PublishHeader() {
  const { t } = useI18n()
  const nav = useHsNav()
  return <PageHeader title={t('homestation.publish.title', 'New post')} onBack={nav.back} backLabel={t('common.back', 'Back')} />
}

function MobileFeedHeader({ readingMode, onReadingModeChange }: { readingMode: ReadingMode; onReadingModeChange: (mode: ReadingMode) => void }) {
  const { t } = useI18n()
  return (
    <header className="hs-column-head flex-shrink-0">
      <span />
      <span className="hs-logo flex items-center justify-center" role="img" aria-label={t('homestation.title', 'HomeStation')}><Waypoints size={26} /></span>
      <div className="flex items-center justify-end"><ReadingModeMenu readingMode={readingMode} onChange={onReadingModeChange} /></div>
    </header>
  )
}

export function HomeStationView({ compose }: { compose?: HomeStationCompose } = {}) {
  const { t } = useI18n()
  const isDesktop = useMediaQuery('(min-width: 769px)')
  const [pages, setPages] = useState<HsPage[]>(() => (compose ? [{ name: 'feed' }, { name: 'publish' }] : [{ name: 'feed' }]))
  const pendingCompose = useRef<HomeStationCompose | null>(compose ?? null)
  const prefill = useMemo(() => ({ peek: () => pendingCompose.current, clear: () => { pendingCompose.current = null } }), [])
  const [query, setQuery] = useState<ReadingQuery>(DEFAULT_QUERY)
  const [readingMode, setReadingMode] = useState<ReadingMode>('standard')
  const [showSearch, setShowSearch] = useState(false)
  const [previewReader, setPreviewReader] = useState<PreviewReader>('owner')
  const [width, setWidth] = useState(1280)
  const [infoOpen, setInfoOpen] = useState(false)
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

  const searchOpen = showSearch && page.name === 'feed'
  const toggleSearch = () => {
    if (page.name !== 'feed') {
      setPages([{ name: 'feed' }])
      setShowSearch(true)
    } else {
      setShowSearch(value => !value)
    }
  }
  const closeSearch = useCallback(() => setShowSearch(false), [])

  const docked = width >= INFO_PANEL_HIDE_BELOW

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

  const feedHeader = isDesktop ? (
    <PageHeader
      title={t('homestation.nav.feed', 'Reading feed')}
      actions={!infoOpen ? (
        <button type="button" className="hs-icon-btn" title={t('homestation.info.show', 'Show context')} aria-label={t('homestation.info.show', 'Show context')} onClick={() => setInfoOpen(true)} data-testid="hs-info-open">
          <PanelRight size={18} />
        </button>
      ) : null}
    />
  ) : (
    <MobileFeedHeader readingMode={readingMode} onReadingModeChange={setReadingMode} />
  )

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
      header={feedHeader}
      showSearch={searchOpen}
      onCloseSearch={closeSearch}
    />
  )

  const infoColumn = (
    <InfoPanel query={query} readingMode={readingMode} onQueryChange={onQueryChange} onClose={() => setInfoOpen(false)} />
  )

  return (
    <HomeStationStoreProvider>
      <ComposePrefillContext.Provider value={prefill}>
      <HsNavContext.Provider value={nav}>
        <div className={isDesktop ? 'hs-root hs-shell-desktop relative flex h-full w-full overflow-hidden' : 'hs-root hs-shell-mobile relative flex h-full w-full flex-col overflow-hidden'} style={{ background: isDesktop ? 'var(--hs-page)' : 'var(--hs-column)', cursor: resizing ? 'col-resize' : undefined }} data-testid="homestation" ref={layoutRef}>
          <ToastHost bottom={isDesktop ? 24 : 72}>
            <WindowDialogProvider surface={isDesktop ? 'desktop' : 'mobile'} permissions={{ fullscreen: false }}>
              {isDesktop ? (
                <>
                  <NavRail searchOpen={searchOpen} onSearch={toggleSearch} />
                  <div className="hs-workspace relative h-full min-w-0 flex-1" style={{
                    '--hs-column-max': `${COLUMN_MAX_WIDTH}px`,
                    '--hs-context-space': `${infoOpen && docked ? COLUMN_GAP + infoWidth : 0}px`,
                  } as CSSProperties}>
                    <main className="flex h-full min-w-0 flex-col">{content}</main>
                    {infoOpen && docked ? (
                      <div className="hs-docked-context absolute inset-y-0 flex">
                        <button
                          type="button"
                          className="group relative h-full flex-shrink-0"
                          style={{ width: COLUMN_GAP, cursor: 'col-resize', touchAction: 'none' }}
                          aria-label={t('homestation.info.resize', 'Resize context panel')}
                          onPointerDown={onSplitterDown}
                          onPointerMove={onSplitterMove}
                          onPointerUp={onSplitterUp}
                          onPointerCancel={onSplitterUp}
                        >
                          <span className="pointer-events-none absolute left-1/2 top-1/2 h-10 w-1 -translate-x-1/2 -translate-y-1/2 rounded-full opacity-0 transition-opacity group-hover:opacity-100" style={{ background: 'var(--hs-line-strong)', opacity: resizing ? 1 : undefined }} />
                        </button>
                        <aside className="flex h-full flex-shrink-0 flex-col" style={{ width: infoWidth }} aria-label={t('homestation.info.title', 'Context')}>{infoColumn}</aside>
                      </div>
                    ) : null}
                  </div>
                  {infoOpen && !docked ? (
                    <aside className="absolute inset-y-0 right-0 z-40 flex flex-col pl-4 pr-4" style={{ width: infoWidth + 32, background: 'var(--hs-page)', boxShadow: 'var(--cp-panel-shadow)' }} aria-label={t('homestation.info.title', 'Context')}>{infoColumn}</aside>
                  ) : null}
                </>
              ) : (
                <>
                  <div className="relative min-h-0 flex-1">{content}</div>
                  {page.name !== 'publish' ? <MobileTabBar searchOpen={searchOpen} onSearch={toggleSearch} /> : null}
                </>
              )}
              {readingMode === 'immersive' ? <ImmersiveMode query={query} onClose={() => setReadingMode('standard')} /> : null}
            </WindowDialogProvider>
          </ToastHost>
        </div>
      </HsNavContext.Provider>
      </ComposePrefillContext.Provider>
    </HomeStationStoreProvider>
  )
}
