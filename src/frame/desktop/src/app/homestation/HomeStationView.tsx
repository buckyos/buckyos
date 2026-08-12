import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useMediaQuery } from '@mui/material'
import {
  ChevronLeft,
  ChevronRight,
  Hash,
  PenSquare,
  Rss,
  Search,
  X,
} from 'lucide-react'
import { useI18n } from '../../i18n/provider'
import { FeedList } from './FeedList'
import { FilterBar } from './FilterBar'
import { InfoPanel } from './InfoPanel'
import { ImmersiveVideoMode } from './ImmersiveVideoMode'
import { PublicProfileView } from './PublicProfileView'
import { ArticleDetail } from './detail/ArticleDetail'
import { ImageDetail } from './detail/ImageDetail'
import { VideoDetail } from './detail/VideoDetail'
import { QuickPublishComposer } from './publish/QuickPublishComposer'
import { SourceManager } from './source/SourceManager'
import {
  filterFeedObjects,
  mockFeedObjects,
  mockSources,
  mockTopics,
  mockUserProfile,
} from './mock/data'
import {
  INFO_PANEL_DEFAULT_WIDTH,
  INFO_PANEL_MAX_WIDTH,
  INFO_PANEL_MIN_WIDTH,
  PANEL_SPLITTER_WIDTH,
} from './layout'
import type {
  FeedFilter,
  FeedObject,
  MobileView,
  ReadingMode,
} from './types'

export function HomeStationView() {
  const { t } = useI18n()
  const isDesktop = useMediaQuery('(min-width: 769px)')

  /* ── Core State ��─ */
  const [activeFilter, setActiveFilter] = useState<FeedFilter>('all')
  const [activeTopicId, setActiveTopicId] = useState<string | null>(null)
  const [readingMode, setReadingMode] = useState<ReadingMode>('standard')
  const [selectedFeedId, setSelectedFeedId] = useState<string | null>(null)
  const [searchQuery, setSearchQuery] = useState('')
  const [showSearch, setShowSearch] = useState(false)

  /* ── Feed State (mutable for interactions) ── */
  const [feedObjects, setFeedObjects] = useState<FeedObject[]>(() => [...mockFeedObjects])

  /* ── Mobile State ── */
  const [mobileView, setMobileView] = useState<MobileView>('feed')

  /* ── Desktop Panel State ���─ */
  const [showInfoPanel, setShowInfoPanel] = useState(true)
  const [infoPanelWidth, setInfoPanelWidth] = useState(INFO_PANEL_DEFAULT_WIDTH)
  const [isResizingInfoPanel, setIsResizingInfoPanel] = useState(false)

  /* ── Refs ── */
  const desktopLayoutRef = useRef<HTMLDivElement>(null)
  const infoPanelWidthRef = useRef(INFO_PANEL_DEFAULT_WIDTH)
  const infoPanelResizeRef = useRef<{ pointerId: number; startX: number; startWidth: number } | null>(null)

  /* ── Derived Data ── */
  const filteredFeeds = useMemo(
    () => filterFeedObjects(feedObjects, activeFilter, activeTopicId),
    [feedObjects, activeFilter, activeTopicId],
  )

  const selectedFeed = useMemo(
    () => (selectedFeedId ? feedObjects.find((f) => f.id === selectedFeedId) ?? null : null),
    [selectedFeedId, feedObjects],
  )

  /* ── Clamp Helpers ── */
  const clampInfoPanelWidth = useCallback(
    (w: number) => Math.min(Math.max(w, INFO_PANEL_MIN_WIDTH), INFO_PANEL_MAX_WIDTH), [],
  )

  /* ── Sync refs ── */
  useEffect(() => { infoPanelWidthRef.current = infoPanelWidth }, [infoPanelWidth])

  useEffect(() => {
    const el = desktopLayoutRef.current
    if (!isDesktop || !el) return

    const ro = new ResizeObserver(() => {
      setInfoPanelWidth((prev) => clampInfoPanelWidth(prev))
    })
    ro.observe(el)
    return () => ro.disconnect()
  }, [clampInfoPanelWidth, isDesktop])

  /* ── Interaction Handlers ── */
  const handleToggleLike = useCallback((id: string) => {
    setFeedObjects((prev) =>
      prev.map((f) =>
        f.id !== id
          ? f
          : {
            ...f,
            interactions: {
              ...f.interactions,
              isLiked: !f.interactions.isLiked,
              likeCount: f.interactions.likeCount + (f.interactions.isLiked ? -1 : 1),
            },
          },
      ),
    )
  }, [])

  const handleToggleBookmark = useCallback((id: string) => {
    setFeedObjects((prev) =>
      prev.map((f) =>
        f.id !== id
          ? f
          : {
            ...f,
            interactions: {
              ...f.interactions,
              isBookmarked: !f.interactions.isBookmarked,
            },
          },
      ),
    )
  }, [])

  const handleRepost = useCallback((id: string) => {
    setFeedObjects((prev) =>
      prev.map((f) =>
        f.id !== id
          ? f
          : {
            ...f,
            interactions: {
              ...f.interactions,
              isReposted: !f.interactions.isReposted,
              repostCount: f.interactions.repostCount + (f.interactions.isReposted ? -1 : 1),
            },
          },
      ),
    )
  }, [])

  const handleSelectFeed = useCallback((id: string) => {
    setSelectedFeedId(id)
    if (!isDesktop) setMobileView('detail')
  }, [isDesktop])

  const handleBack = useCallback(() => {
    setSelectedFeedId(null)
    setMobileView('feed')
  }, [])

  const handlePublish = useCallback((text: string) => {
    const newFeed: FeedObject = {
      id: `feed-new-${Date.now()}`,
      author: { id: mockUserProfile.id, name: mockUserProfile.name, sourceType: 'did', isVerified: true },
      contentType: 'text',
      text,
      media: [],
      topics: [],
      interactions: { likeCount: 0, commentCount: 0, repostCount: 0, isLiked: false, isBookmarked: false, isReposted: false },
      createdAt: Date.now(),
      sourceId: 'self',
    }
    setFeedObjects((prev) => [newFeed, ...prev])
    if (!isDesktop) {
      setMobileView('feed')
    }
  }, [isDesktop])

  const handleReadingModeChange = useCallback((mode: ReadingMode) => {
    if (mode === 'immersive-video') {
      if (!isDesktop) setMobileView('immersive')
      else setReadingMode(mode)
    }
    setReadingMode(mode)
  }, [isDesktop])

  /* ── Info Panel Splitter ── */
  const handleInfoPanelSplitterPointerDown = useCallback((e: React.PointerEvent<HTMLButtonElement>) => {
    infoPanelResizeRef.current = { pointerId: e.pointerId, startX: e.clientX, startWidth: infoPanelWidthRef.current }
    setIsResizingInfoPanel(true)
    e.currentTarget.setPointerCapture(e.pointerId)
    e.preventDefault()
  }, [])

  const handleInfoPanelSplitterPointerMove = useCallback((e: React.PointerEvent<HTMLButtonElement>) => {
    if (!infoPanelResizeRef.current || infoPanelResizeRef.current.pointerId !== e.pointerId) return
    const next = clampInfoPanelWidth(infoPanelResizeRef.current.startWidth - (e.clientX - infoPanelResizeRef.current.startX))
    infoPanelWidthRef.current = next
    setInfoPanelWidth(next)
  }, [clampInfoPanelWidth])

  const handleInfoPanelSplitterPointerUp = useCallback((e: React.PointerEvent<HTMLButtonElement>) => {
    if (!infoPanelResizeRef.current || infoPanelResizeRef.current.pointerId !== e.pointerId) return
    infoPanelResizeRef.current = null
    setIsResizingInfoPanel(false)
    e.currentTarget.releasePointerCapture(e.pointerId)
  }, [])

  /* ── Immersive overlay ── */
  if (readingMode === 'immersive-video' && isDesktop) {
    const videoFeeds = feedObjects.filter(
      (f) => f.contentType === 'video' || f.media.some((m) => m.type === 'video'),
    )
    return (
      <ImmersiveVideoMode
        feeds={videoFeeds.length > 0 ? videoFeeds : feedObjects}
        onToggleLike={handleToggleLike}
        onToggleBookmark={handleToggleBookmark}
        onRepost={handleRepost}
        onClose={() => setReadingMode('standard')}
      />
    )
  }

  /* ── Mobile Layout ── */
  if (!isDesktop) {
    return (
      <div className="relative flex h-full w-full flex-col" style={{ background: 'var(--cp-bg)' }}>
        {/* Immersive mode overlay */}
        {mobileView === 'immersive' ? (
          <ImmersiveVideoMode
            feeds={feedObjects.filter(
              (f) => f.contentType === 'video' || f.media.some((m) => m.type === 'video'),
            )}
            onToggleLike={handleToggleLike}
            onToggleBookmark={handleToggleBookmark}
            onRepost={handleRepost}
            onClose={() => { setMobileView('feed'); setReadingMode('standard') }}
          />
        ) : null}

        {/* Detail view */}
        {mobileView === 'detail' && selectedFeed ? (
          <div className="h-full">
            {selectedFeed.contentType === 'article' && selectedFeed.body ? (
              <ArticleDetail feed={selectedFeed} t={t} onBack={handleBack} onToggleLike={handleToggleLike} onToggleBookmark={handleToggleBookmark} />
            ) : selectedFeed.media.some((m) => m.type === 'video') ? (
              <VideoDetail feed={selectedFeed} t={t} onBack={handleBack} onToggleLike={handleToggleLike} onToggleBookmark={handleToggleBookmark} />
            ) : selectedFeed.media.some((m) => m.type === 'image') ? (
              <ImageDetail feed={selectedFeed} t={t} onBack={handleBack} onToggleLike={handleToggleLike} onToggleBookmark={handleToggleBookmark} />
            ) : (
              <ArticleDetail feed={selectedFeed} t={t} onBack={handleBack} onToggleLike={handleToggleLike} onToggleBookmark={handleToggleBookmark} />
            )}
          </div>
        ) : null}

        {/* Profile view */}
        {mobileView === 'profile' ? (
          <div className="flex h-full flex-col">
            {/* Back header */}
            <div className="flex items-center gap-2 px-2 py-2" style={{ borderBottom: '1px solid var(--cp-border)' }}>
              <button
                type="button"
                onClick={() => { setMobileView('feed') }}
                className="flex h-9 w-9 items-center justify-center rounded-xl"
                style={{ color: 'var(--cp-text)' }}
              >
                <ChevronLeft size={20} />
              </button>
              <span className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>
                {t('homestation.profile', 'Profile')}
              </span>
            </div>
            <div className="flex-1 overflow-y-auto">
              <PublicProfileView
                profile={mockUserProfile}
                feeds={feedObjects.filter((f) => f.author.id === mockUserProfile.id)}
                t={t}
                onSelectFeed={handleSelectFeed}
                onToggleLike={handleToggleLike}
                onToggleBookmark={handleToggleBookmark}
                onRepost={handleRepost}
              />
              {/* Sources entry */}
              <div className="px-4 pb-6">
                <button
                  type="button"
                  onClick={() => setMobileView('sources')}
                  className="flex w-full items-center gap-3 rounded-xl px-4 py-3"
                  style={{ background: 'color-mix(in srgb, var(--cp-text) 5%, transparent)' }}
                >
                  <Rss size={18} style={{ color: 'var(--cp-accent)' }} />
                  <span className="flex-1 text-left text-sm font-medium" style={{ color: 'var(--cp-text)' }}>
                    {t('homestation.manageSources', 'Manage Sources')}
                  </span>
                  <ChevronRight size={16} style={{ color: 'var(--cp-muted)' }} />
                </button>
              </div>
            </div>
          </div>
        ) : null}

        {/* Publish view */}
        {mobileView === 'publish' ? (
          <div className="flex h-full flex-col">
            <div className="flex items-center gap-2 px-2 py-2" style={{ borderBottom: '1px solid var(--cp-border)' }}>
              <button type="button" onClick={() => setMobileView('feed')} className="flex h-9 w-9 items-center justify-center rounded-xl" style={{ color: 'var(--cp-text)' }}>
                <ChevronLeft size={20} />
              </button>
              <span className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{t('homestation.tabPublish', 'Publish')}</span>
            </div>
            <div className="flex-1 overflow-y-auto">
              <QuickPublishComposer t={t} onPublish={handlePublish} />
            </div>
          </div>
        ) : null}

        {/* Sources view */}
        {mobileView === 'sources' ? (
          <div className="flex h-full flex-col">
            <div className="flex items-center gap-2 px-2 py-2" style={{ borderBottom: '1px solid var(--cp-border)' }}>
              <button type="button" onClick={() => { setMobileView('profile') }} className="flex h-9 w-9 items-center justify-center rounded-xl" style={{ color: 'var(--cp-text)' }}>
                <ChevronLeft size={20} />
              </button>
              <span className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{t('homestation.tabSources', 'Sources')}</span>
            </div>
            <div className="flex-1 overflow-y-auto">
              <SourceManager sources={mockSources} t={t} />
            </div>
          </div>
        ) : null}

        {/* Feed view (default) */}
        {mobileView === 'feed' ? (
          <>
            {/* Top bar: avatar + name + search */}
            <div className="flex items-center gap-3 px-4 py-2" style={{ borderBottom: '1px solid var(--cp-border)' }}>
              <button
                type="button"
                onClick={() => { setMobileView('profile') }}
                className="flex h-9 w-9 flex-shrink-0 items-center justify-center rounded-full text-sm font-semibold"
                style={{
                  background: 'color-mix(in srgb, var(--cp-accent) 15%, transparent)',
                  color: 'var(--cp-accent)',
                }}
              >
                {mockUserProfile.name.charAt(0)}
              </button>
              <div className="min-w-0 flex-1">
                <p className="truncate text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{mockUserProfile.name}</p>
                <p className="truncate text-[11px]" style={{ color: 'var(--cp-muted)' }}>
                  {mockUserProfile.bio ?? t('homestation.title', 'HomeStation')}
                </p>
              </div>
              <button
                type="button"
                onClick={() => setShowSearch((v) => !v)}
                className="flex h-9 w-9 flex-shrink-0 items-center justify-center rounded-xl"
                style={{ color: 'var(--cp-muted)', background: 'color-mix(in srgb, var(--cp-text) 7%, transparent)' }}
              >
                <Search size={18} />
              </button>
            </div>

            {/* Search panel with Topics (conditional) */}
            {showSearch ? (
              <div style={{ borderBottom: '1px solid var(--cp-border)' }}>
                <div className="flex items-center gap-2 px-4 py-2">
                  <Search size={16} style={{ color: 'var(--cp-muted)' }} />
                  <input
                    type="text"
                    value={searchQuery}
                    onChange={(e) => setSearchQuery(e.target.value)}
                    placeholder={t('homestation.searchPlaceholder', 'Search feeds...')}
                    className="flex-1 bg-transparent text-sm outline-none"
                    style={{ color: 'var(--cp-text)' }}
                    autoFocus
                  />
                  <button type="button" onClick={() => { setShowSearch(false); setSearchQuery('') }} style={{ color: 'var(--cp-muted)' }}>
                    <X size={16} />
                  </button>
                </div>
                {/* Topics list */}
                <div className="flex flex-wrap gap-2 px-4 pb-3 pt-1">
                  <span className="text-[11px] font-medium" style={{ color: 'var(--cp-muted)', lineHeight: '28px' }}>
                    {t('homestation.topics', 'Topics')}:
                  </span>
                  {mockTopics.map((topic) => (
                    <button
                      key={topic.id}
                      type="button"
                      onClick={() => {
                        setActiveTopicId(activeTopicId === topic.id ? null : topic.id)
                        setActiveFilter('all')
                        setShowSearch(false)
                        setSearchQuery('')
                      }}
                      className="flex items-center gap-1 rounded-full px-2.5 py-1 text-xs font-medium transition-colors"
                      style={{
                        background: activeTopicId === topic.id
                          ? 'var(--cp-accent)'
                          : 'color-mix(in srgb, var(--cp-text) 8%, transparent)',
                        color: activeTopicId === topic.id ? 'white' : 'var(--cp-text)',
                      }}
                    >
                      <Hash size={12} />
                      {topic.name}
                    </button>
                  ))}
                </div>
              </div>
            ) : null}

            {/* Feed list with filter bar (scrolls together) */}
            <div className="flex-1 overflow-y-auto">
              <FilterBar
                activeFilter={activeFilter}
                activeTopicId={activeTopicId}
                readingMode={readingMode}
                topics={mockTopics}
                t={t}
                onFilterChange={setActiveFilter}
                onTopicSelect={setActiveTopicId}
                onReadingModeChange={handleReadingModeChange}
                isMobile
              />
              <FeedList
                feeds={filteredFeeds}
                readingMode={readingMode}
                t={t}
                onSelectFeed={handleSelectFeed}
                onToggleLike={handleToggleLike}
                onToggleBookmark={handleToggleBookmark}
                onRepost={handleRepost}
                scrollable={false}
              />
            </div>

            {/* FAB */}
            <button
              type="button"
              onClick={() => setMobileView('publish')}
              className="absolute z-20 flex h-14 w-14 items-center justify-center rounded-full shadow-lg"
              style={{
                right: 16,
                bottom: 16,
                background: 'var(--cp-accent)',
                color: 'white',
              }}
            >
              <PenSquare size={22} />
            </button>
          </>
        ) : null}

      </div>
    )
  }

  /* ── Desktop Layout ── */
  return (
    <div
      ref={desktopLayoutRef}
      className="flex h-full w-full"
      style={{
        background: 'var(--cp-bg)',
        zIndex: 1,
        cursor: isResizingInfoPanel ? 'col-resize' : 'default',
      }}
    >
      {/* Center: Feed column */}
      <div className="flex h-full min-w-0 flex-1 flex-col" style={{ borderRight: '1px solid var(--cp-border)' }}>
        {/* Desktop top bar - mobile style */}
        <div className="flex items-center gap-3 px-4 py-2">
          <button
            type="button"
            className="flex h-9 w-9 flex-shrink-0 items-center justify-center rounded-full text-sm font-semibold"
            style={{
              background: 'color-mix(in srgb, var(--cp-accent) 15%, transparent)',
              color: 'var(--cp-accent)',
            }}
          >
            {mockUserProfile.name.charAt(0)}
          </button>
          <div className="min-w-0 flex-1">
            <p className="truncate text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{mockUserProfile.name}</p>
            <p className="truncate text-[11px]" style={{ color: 'var(--cp-muted)' }}>
              {mockUserProfile.bio ?? t('homestation.title', 'HomeStation')}
            </p>
          </div>
          <button
            type="button"
            onClick={() => setShowSearch((v) => !v)}
            className="flex h-9 w-9 flex-shrink-0 items-center justify-center rounded-xl"
            style={{ color: 'var(--cp-muted)', background: 'color-mix(in srgb, var(--cp-text) 7%, transparent)' }}
          >
            <Search size={18} />
          </button>
          {!showInfoPanel ? (
            <button
              type="button"
              onClick={() => setShowInfoPanel(true)}
              className="flex h-8 w-8 items-center justify-center rounded-xl"
              style={{ color: 'var(--cp-muted)', background: 'color-mix(in srgb, var(--cp-text) 7%, transparent)' }}
              title={t('homestation.showInfoPanel', 'Show info panel')}
            >
              <ChevronLeft size={16} />
            </button>
          ) : null}
        </div>

        {/* Search panel with Topics (conditional) */}
        {showSearch ? (
          <div style={{ borderBottom: '1px solid var(--cp-border)' }}>
            <div className="flex items-center gap-2 px-4 py-2">
              <Search size={16} style={{ color: 'var(--cp-muted)' }} />
              <input
                type="text"
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                placeholder={t('homestation.searchPlaceholder', 'Search feeds...')}
                className="flex-1 bg-transparent text-sm outline-none"
                style={{ color: 'var(--cp-text)' }}
                autoFocus
              />
              <button type="button" onClick={() => { setShowSearch(false); setSearchQuery('') }} style={{ color: 'var(--cp-muted)' }}>
                <X size={16} />
              </button>
            </div>
            <div className="flex flex-wrap gap-2 px-4 pb-3 pt-1">
              <span className="text-[11px] font-medium" style={{ color: 'var(--cp-muted)', lineHeight: '28px' }}>
                {t('homestation.topics', 'Topics')}:
              </span>
              {mockTopics.map((topic) => (
                <button
                  key={topic.id}
                  type="button"
                  onClick={() => {
                    setActiveTopicId(activeTopicId === topic.id ? null : topic.id)
                    setActiveFilter('all')
                    setShowSearch(false)
                    setSearchQuery('')
                  }}
                  className="flex items-center gap-1 rounded-full px-2.5 py-1 text-xs font-medium transition-colors"
                  style={{
                    background: activeTopicId === topic.id
                      ? 'var(--cp-accent)'
                      : 'color-mix(in srgb, var(--cp-text) 8%, transparent)',
                    color: activeTopicId === topic.id ? 'white' : 'var(--cp-text)',
                  }}
                >
                  <Hash size={12} />
                  {topic.name}
                </button>
              ))}
            </div>
          </div>
        ) : null}

        {selectedFeed ? (
          <div className="flex-1 overflow-y-auto">
            {selectedFeed.contentType === 'article' && selectedFeed.body ? (
              <ArticleDetail feed={selectedFeed} t={t} onBack={handleBack} onToggleLike={handleToggleLike} onToggleBookmark={handleToggleBookmark} />
            ) : selectedFeed.media.some((m) => m.type === 'video') ? (
              <VideoDetail feed={selectedFeed} t={t} onBack={handleBack} onToggleLike={handleToggleLike} onToggleBookmark={handleToggleBookmark} />
            ) : selectedFeed.media.some((m) => m.type === 'image') ? (
              <ImageDetail feed={selectedFeed} t={t} onBack={handleBack} onToggleLike={handleToggleLike} onToggleBookmark={handleToggleBookmark} />
            ) : (
              <ArticleDetail feed={selectedFeed} t={t} onBack={handleBack} onToggleLike={handleToggleLike} onToggleBookmark={handleToggleBookmark} />
            )}
          </div>
        ) : (
          <>
            <FilterBar
              activeFilter={activeFilter}
              activeTopicId={activeTopicId}
              readingMode={readingMode}
              topics={mockTopics}
              t={t}
              onFilterChange={setActiveFilter}
              onTopicSelect={setActiveTopicId}
              onReadingModeChange={handleReadingModeChange}
            />
            <div className="flex-1 overflow-hidden">
              <FeedList
                feeds={filteredFeeds}
                readingMode={readingMode}
                t={t}
                onSelectFeed={handleSelectFeed}
                onToggleLike={handleToggleLike}
                onToggleBookmark={handleToggleBookmark}
                onRepost={handleRepost}
              />
            </div>
          </>
        )}
      </div>

      {/* Info panel splitter */}
      {showInfoPanel ? (
        <button
          type="button"
          className="group relative h-full flex-shrink-0"
          onPointerDown={handleInfoPanelSplitterPointerDown}
          onPointerMove={handleInfoPanelSplitterPointerMove}
          onPointerUp={handleInfoPanelSplitterPointerUp}
          onPointerCancel={handleInfoPanelSplitterPointerUp}
          title={t('homestation.resizeInfoPanel', 'Resize info panel')}
          style={{
            width: PANEL_SPLITTER_WIDTH,
            marginLeft: -(PANEL_SPLITTER_WIDTH / 2),
            marginRight: -(PANEL_SPLITTER_WIDTH / 2),
            cursor: 'col-resize',
            background: isResizingInfoPanel ? 'color-mix(in srgb, var(--cp-accent) 8%, transparent)' : 'transparent',
            zIndex: 10,
            touchAction: 'none',
          }}
        >
          <span
            className="pointer-events-none absolute inset-y-0 left-1/2 -translate-x-1/2 rounded-full transition-all duration-150"
            style={{
              width: isResizingInfoPanel ? 3 : 1,
              top: 18,
              bottom: 18,
              background: isResizingInfoPanel ? 'var(--cp-accent)' : 'color-mix(in srgb, var(--cp-border) 92%, transparent)',
              boxShadow: isResizingInfoPanel ? '0 0 0 4px color-mix(in srgb, var(--cp-accent) 12%, transparent)' : 'none',
            }}
          />
        </button>
      ) : null}

      {/* Right info panel */}
      {showInfoPanel ? (
        <div
          className="h-full flex-shrink-0"
          style={{
            width: infoPanelWidth,
            minWidth: INFO_PANEL_MIN_WIDTH,
            maxWidth: INFO_PANEL_MAX_WIDTH,
            background: 'var(--cp-bg)',
            transition: isResizingInfoPanel ? 'none' : 'width 220ms var(--cp-ease-emphasis)',
          }}
        >
          <InfoPanel
            activeFilter={activeFilter}
            activeTopicId={activeTopicId}
            readingMode={readingMode}
            topics={mockTopics}
            t={t}
            onPublish={handlePublish}
            onSelectTopic={(id) => { setActiveTopicId(id); setActiveFilter('all') }}
            onClose={() => setShowInfoPanel(false)}
          />
        </div>
      ) : null}
    </div>
  )
}
