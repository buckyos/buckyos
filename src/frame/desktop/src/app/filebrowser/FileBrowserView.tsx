import { IconButton, useMediaQuery } from '@mui/material'
import clsx from 'clsx'
import { useCallback, useMemo, useState } from 'react'
import {
  Camera,
  ChevronRight,
  FolderPlus,
  Image as ImageIcon,
  LayoutGrid,
  List,
  Menu as MenuIcon,
  PanelRight,
  Plus,
  RefreshCw,
  Search,
  Upload as UploadIcon,
  X,
} from 'lucide-react'
import { useI18n } from '../../i18n/provider'
import {
  useMobileBackHandler,
  useMobileTitleOverride,
} from '../../desktop/windows/MobileNavContext'
import { MainContent } from './MainContent'
import { PreviewPanel } from './PreviewPanel'
import { SearchResultsPanel } from './SearchResultsPanel'
import { Sidebar } from './Sidebar'
import { StatusBar } from './StatusBar'
import { TopBar } from './TopBar'
import {
  defaultTabs,
  fileBrowserSnapshot,
  searchFiles,
} from './mock/data'
import type { BrowserTab, FileEntry, ViewMode } from './types'

interface HistoryState {
  back: string[]
  forward: string[]
}

export function FileBrowserView() {
  const { t } = useI18n()
  const isMobile = useMediaQuery('(max-width: 900px)')

  const [tabs, setTabs] = useState<BrowserTab[]>(defaultTabs)
  const [activeTabId, setActiveTabId] = useState(defaultTabs[0].id)
  const [history, setHistory] = useState<Record<string, HistoryState>>({
    [defaultTabs[0].id]: { back: [], forward: [] },
    [defaultTabs[1].id]: { back: [], forward: [] },
  })

  const [viewMode, setViewMode] = useState<ViewMode>('list')
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [activeTopicId, setActiveTopicId] = useState<string | null>(null)
  const [searchQuery, setSearchQuery] = useState('')
  const [advancedMode, setAdvancedMode] = useState(false)
  const [toast, setToast] = useState<string | null>(null)

  // Mobile-only panel states
  const [mobileSidebarOpen, setMobileSidebarOpen] = useState(false)
  const [mobilePreviewOpen, setMobilePreviewOpen] = useState(false)
  const [mobileUploadOpen, setMobileUploadOpen] = useState(false)

  const activeTab = tabs.find((tab) => tab.id === activeTabId) ?? tabs[0]
  const currentPath = activeTab?.path ?? '/home'

  const updateTab = useCallback(
    (tabId: string, next: Partial<BrowserTab>) => {
      setTabs((prev) => prev.map((tab) => (tab.id === tabId ? { ...tab, ...next } : tab)))
    },
    [],
  )

  const pushHistory = useCallback((tabId: string, path: string) => {
    setHistory((prev) => {
      const curr = prev[tabId] ?? { back: [], forward: [] }
      return {
        ...prev,
        [tabId]: { back: [...curr.back, path], forward: [] },
      }
    })
  }, [])

  const navigate = useCallback(
    (path: string, options?: { suppressHistory?: boolean }) => {
      if (!activeTab) return
      if (path === currentPath) return
      if (!options?.suppressHistory) pushHistory(activeTab.id, currentPath)
      const title = path.split('/').filter(Boolean).pop() ?? 'root'
      updateTab(activeTab.id, { path, title })
      setSelectedId(null)
      setActiveTopicId(null)
    },
    [activeTab, currentPath, pushHistory, updateTab],
  )

  const back = () => {
    if (!activeTab) return
    const hist = history[activeTab.id]
    if (!hist || hist.back.length === 0) return
    const previous = hist.back[hist.back.length - 1]
    setHistory((prev) => ({
      ...prev,
      [activeTab.id]: {
        back: hist.back.slice(0, -1),
        forward: [currentPath, ...hist.forward],
      },
    }))
    const title = previous.split('/').filter(Boolean).pop() ?? 'root'
    updateTab(activeTab.id, { path: previous, title })
    setSelectedId(null)
  }

  const forward = () => {
    if (!activeTab) return
    const hist = history[activeTab.id]
    if (!hist || hist.forward.length === 0) return
    const next = hist.forward[0]
    setHistory((prev) => ({
      ...prev,
      [activeTab.id]: {
        back: [...hist.back, currentPath],
        forward: hist.forward.slice(1),
      },
    }))
    const title = next.split('/').filter(Boolean).pop() ?? 'root'
    updateTab(activeTab.id, { path: next, title })
    setSelectedId(null)
  }

  const goUp = () => {
    const parent = currentPath.split('/').slice(0, -1).join('/') || '/'
    if (parent !== currentPath) navigate(parent)
  }

  const handleNewTab = () => {
    const id = `tab-${Date.now()}`
    const tab: BrowserTab = { id, title: 'Home', path: '/home' }
    setTabs((prev) => [...prev, tab])
    setHistory((prev) => ({ ...prev, [id]: { back: [], forward: [] } }))
    setActiveTabId(id)
  }

  const handleCloseTab = (id: string) => {
    setTabs((prev) => {
      const next = prev.filter((tab) => tab.id !== id)
      if (id === activeTabId && next.length) {
        setActiveTabId(next[0].id)
      }
      return next
    })
  }

  const showToast = (message: string) => {
    setToast(message)
    window.setTimeout(() => setToast(null), 2000)
  }

  const copyText = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text)
      showToast(t('filebrowser.toast.copied', 'Copied to clipboard'))
    } catch {
      showToast(t('filebrowser.toast.copyFailed', 'Copy failed'))
    }
  }

  const currentEntries = useMemo(() => {
    if (activeTopicId) {
      const topic = fileBrowserSnapshot.topics.find((t) => t.id === activeTopicId)
      if (!topic) return []
      const ids = new Set(topic.groups.flatMap((group) => group.fileIds))
      return Array.from(ids)
        .map((id) => fileBrowserSnapshot.entriesById[id])
        .filter((entry): entry is FileEntry => !!entry)
    }
    return fileBrowserSnapshot.entriesByPath[currentPath] ?? []
  }, [activeTopicId, currentPath])

  const selectedEntry = selectedId
    ? fileBrowserSnapshot.entriesById[selectedId] ?? null
    : null

  const searchHits = useMemo(
    () => (searchQuery.trim() ? searchFiles(searchQuery) : []),
    [searchQuery],
  )

  const topicContext = activeTopicId
    ? fileBrowserSnapshot.topics.find((t) => t.id === activeTopicId) ?? null
    : null

  const handleSelectTopic = (topicId: string) => {
    setActiveTopicId(topicId)
    setSelectedId(null)
    setSearchQuery('')
    if (!activeTab) return
    const topic = fileBrowserSnapshot.topics.find((t) => t.id === topicId)
    updateTab(activeTab.id, {
      title: topic?.title ?? 'Topic',
      path: `topic://${topicId}`,
    })
  }

  const handleOpenEntry = (entry: FileEntry) => {
    setSelectedId(entry.id)
    if (isMobile) setMobilePreviewOpen(true)
  }

  const handleOpenFolder = (path: string) => {
    navigate(path)
    setMobileSidebarOpen(false)
  }

  const hist = history[activeTabId] ?? { back: [], forward: [] }
  const searchActive = !!searchQuery.trim()

  const mobileTitleText = selectedEntry?.name ?? activeTab?.title ?? 'root'
  const mobileSubtitleText =
    selectedEntry?.summary ??
    topicContext?.description ??
    (currentPath === '/' ? t('filebrowser.mobile.rootHint', 'Root directory') : currentPath)

  const mobileTitleOverride = useMemo(
    () => (isMobile ? { title: mobileTitleText, subtitle: mobileSubtitleText } : null),
    [isMobile, mobileTitleText, mobileSubtitleText],
  )
  useMobileTitleOverride(mobileTitleOverride)

  const canMobileBack = isMobile && hist.back.length > 0
  useMobileBackHandler(canMobileBack ? back : null)

  // ─── Mobile layout ───
  if (isMobile) {
    const segments = currentPath.split('/').filter(Boolean)
    const crumbs: { label: string; path: string }[] = [{ label: 'root', path: '/' }]
    {
      let running = ''
      for (const segment of segments) {
        running += `/${segment}`
        crumbs.push({ label: segment, path: running })
      }
    }

    return (
      <div className="relative flex h-full w-full flex-col overflow-hidden" style={{ background: 'var(--cp-bg)' }}>
        {/* Operations bar: drawer toggle + search + view mode */}
        <div className="flex items-center gap-2 px-3 pt-2 pb-1">
          <IconButton
            size="small"
            onClick={() => setMobileSidebarOpen((v) => !v)}
            aria-label={t('filebrowser.mobile.places', 'Places')}
            className={clsx(
              mobileSidebarOpen &&
                '!bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_28%,var(--cp-surface))] !text-[color:var(--cp-text)]',
            )}
          >
            <MenuIcon size={16} />
          </IconButton>
          <div className="relative flex min-w-0 flex-1 items-center gap-1 rounded-full border border-[color:color-mix(in_srgb,var(--cp-border)_60%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-surface-2)_88%,transparent)] px-2 py-1">
            <Search size={14} className="ml-1 shrink-0 text-[color:var(--cp-muted)]" />
            <input
              type="text"
              value={searchQuery}
              onChange={(event) => setSearchQuery(event.target.value)}
              placeholder={t(
                'filebrowser.topbar.searchPlaceholder',
                'Search across files, folders, AI summaries…',
              )}
              className="min-w-0 flex-1 bg-transparent text-xs outline-none placeholder:text-[color:var(--cp-muted)]"
              style={{ color: 'var(--cp-text)' }}
            />
            {searchQuery ? (
              <IconButton
                size="small"
                onClick={() => setSearchQuery('')}
                aria-label={t('common.close', 'Close')}
              >
                <X size={12} />
              </IconButton>
            ) : null}
          </div>
          <div className="flex items-center rounded-full border border-[color:color-mix(in_srgb,var(--cp-border)_60%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-surface-2)_80%,transparent)]">
            <IconButton
              size="small"
              onClick={() => setViewMode('list')}
              className={clsx(
                viewMode === 'list' &&
                  '!bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_28%,var(--cp-surface))] !text-[color:var(--cp-text)]',
              )}
              aria-label={t('filebrowser.view.list', 'List view')}
            >
              <List size={14} />
            </IconButton>
            <IconButton
              size="small"
              onClick={() => setViewMode('icon')}
              className={clsx(
                viewMode === 'icon' &&
                  '!bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_28%,var(--cp-surface))] !text-[color:var(--cp-text)]',
              )}
              aria-label={t('filebrowser.view.icon', 'Icon view')}
            >
              <LayoutGrid size={14} />
            </IconButton>
          </div>
        </div>

        {/* Address bar: path crumbs + refresh on the right */}
        <div className="flex items-center gap-2 px-3 pb-2 pt-1">
          <div className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto text-[13px] text-[color:var(--cp-muted)]">
            {crumbs.map((crumb, idx) => (
              <div key={crumb.path} className="flex shrink-0 items-center gap-1">
                <button
                  type="button"
                  className={clsx(
                    'truncate rounded-md px-1.5 py-1',
                    idx === crumbs.length - 1 && 'font-semibold text-[color:var(--cp-text)]',
                  )}
                  onClick={() => navigate(crumb.path)}
                >
                  {crumb.label}
                </button>
                {idx < crumbs.length - 1 ? (
                  <ChevronRight size={13} className="opacity-60" />
                ) : null}
              </div>
            ))}
          </div>
          <button
            type="button"
            onClick={() => navigate(currentPath)}
            aria-label={t('filebrowser.topbar.refresh', 'Refresh')}
            className="shrink-0 p-1 text-[color:var(--cp-muted)] hover:text-[color:var(--cp-text)]"
          >
            <RefreshCw size={16} />
          </button>
        </div>

        <div className="flex-1 overflow-hidden">
          {searchActive ? (
            <SearchResultsPanel
              hits={searchHits}
              query={searchQuery}
              onSelect={handleOpenEntry}
            />
          ) : (
            <MainContent
              entries={currentEntries}
              viewMode={viewMode}
              selectedId={selectedId}
              onSelect={handleOpenEntry}
              onOpenFolder={handleOpenFolder}
              currentPath={currentPath}
              topicContext={topicContext}
              isMobile
            />
          )}
        </div>

        {/* Floating action button — upload */}
        <button
          type="button"
          onClick={() => setMobileUploadOpen(true)}
          aria-label={t('filebrowser.actions.upload', 'Upload')}
          className="absolute bottom-5 right-5 z-20 flex h-14 w-14 items-center justify-center rounded-full text-white shadow-[0_10px_28px_rgba(0,0,0,0.22)] transition active:scale-95"
          style={{ background: 'var(--cp-accent)' }}
        >
          <Plus size={26} />
        </button>

        {/* Upload action sheet */}
        {mobileUploadOpen ? (
          <div className="absolute inset-0 z-40 flex items-end">
            <div
              className="absolute inset-0 bg-black/50"
              onClick={() => setMobileUploadOpen(false)}
            />
            <div
              className="relative w-full rounded-t-[28px] border-t border-[color:var(--cp-border)] pb-5 pt-2"
              style={{ background: 'var(--cp-surface)' }}
            >
              <div className="mx-auto mb-2 h-1 w-10 rounded-full bg-[color:var(--cp-border)]" />
              <div className="px-5 pb-1 text-[13px] font-semibold text-[color:var(--cp-text)]">
                {t('filebrowser.upload.title', 'Add to this folder')}
              </div>
              <div className="grid grid-cols-4 gap-1 px-3 py-3">
                {[
                  {
                    key: 'files',
                    icon: <UploadIcon size={22} />,
                    label: t('filebrowser.upload.files', 'Files'),
                  },
                  {
                    key: 'photos',
                    icon: <ImageIcon size={22} />,
                    label: t('filebrowser.upload.photos', 'Photos'),
                  },
                  {
                    key: 'camera',
                    icon: <Camera size={22} />,
                    label: t('filebrowser.upload.camera', 'Camera'),
                  },
                  {
                    key: 'folder',
                    icon: <FolderPlus size={22} />,
                    label: t('filebrowser.upload.newFolder', 'New folder'),
                  },
                ].map((action) => (
                  <button
                    key={action.key}
                    type="button"
                    onClick={() => {
                      setMobileUploadOpen(false)
                      showToast(`${action.label} (mock)`)
                    }}
                    className="flex flex-col items-center gap-1.5 rounded-[16px] p-2 text-[11px] text-[color:var(--cp-text)] hover:bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_18%,transparent)]"
                  >
                    <div
                      className="flex h-12 w-12 items-center justify-center rounded-full text-[color:var(--cp-accent)]"
                      style={{
                        background:
                          'color-mix(in srgb, var(--cp-accent-soft) 32%, var(--cp-surface))',
                      }}
                    >
                      {action.icon}
                    </div>
                    <span className="text-center leading-tight">{action.label}</span>
                  </button>
                ))}
              </div>
            </div>
          </div>
        ) : null}

        {/* Sidebar drawer */}
        {mobileSidebarOpen ? (
          <div className="absolute inset-0 z-40 flex">
            <div
              className="absolute inset-0 bg-black/50"
              onClick={() => setMobileSidebarOpen(false)}
            />
            <div
              className="relative flex h-full w-2/3 flex-col gap-3 border-r border-[color:var(--cp-border)] p-3"
              style={{ background: 'var(--cp-surface)' }}
            >
              <Sidebar
                dfsRoots={fileBrowserSnapshot.dfsRoots}
                devices={fileBrowserSnapshot.devices}
                topics={fileBrowserSnapshot.topics}
                activePath={currentPath}
                activeTopicId={activeTopicId}
                advancedMode={advancedMode}
                onToggleAdvanced={setAdvancedMode}
                onNavigate={navigate}
                onSelectTopic={handleSelectTopic}
                compact
              />
            </div>
          </div>
        ) : null}

        {/* Preview bottom sheet */}
        {mobilePreviewOpen && selectedEntry ? (
          <div className="absolute inset-0 z-30 flex items-end">
            <div
              className="absolute inset-0 bg-black/40"
              onClick={() => setMobilePreviewOpen(false)}
            />
            <div
              className="relative flex h-[78%] w-full flex-col rounded-t-[28px] border-t border-[color:var(--cp-border)]"
              style={{ background: 'var(--cp-surface)' }}
            >
              <div className="flex items-center justify-between px-4 py-2">
                <span className="shell-kicker">
                  {t('filebrowser.mobile.preview', 'Preview')}
                </span>
                <button
                  type="button"
                  onClick={() => setMobilePreviewOpen(false)}
                  className="rounded-full border border-[color:var(--cp-border)] px-2.5 py-1 text-xs"
                >
                  {t('common.close', 'Close')}
                </button>
              </div>
              <div className="flex-1 overflow-hidden">
                <PreviewPanel
                  entry={selectedEntry}
                  topics={fileBrowserSnapshot.topics}
                  onJumpToTopic={(id) => {
                    handleSelectTopic(id)
                    setMobilePreviewOpen(false)
                  }}
                  onJumpToPath={(path) => {
                    navigate(path)
                    setMobilePreviewOpen(false)
                  }}
                  embedded
                />
              </div>
            </div>
          </div>
        ) : null}

        {toast ? (
          <div className="pointer-events-none absolute bottom-14 left-1/2 -translate-x-1/2 rounded-full bg-black/80 px-3 py-1.5 text-xs text-white">
            {toast}
          </div>
        ) : null}
      </div>
    )
  }

  // ─── Desktop layout ───
  return (
    <div
      className="flex h-full w-full flex-col overflow-hidden"
      style={{ background: 'var(--cp-bg)' }}
    >
      <TopBar
        tabs={tabs}
        activeTabId={activeTabId}
        onSelectTab={setActiveTabId}
        onCloseTab={handleCloseTab}
        onNewTab={handleNewTab}
        currentPath={currentPath}
        onNavigate={navigate}
        onBack={back}
        onForward={forward}
        onUp={goUp}
        canBack={hist.back.length > 0}
        canForward={hist.forward.length > 0}
        canUp={currentPath !== '/'}
        viewMode={viewMode}
        onViewModeChange={setViewMode}
        searchQuery={searchQuery}
        onSearchChange={setSearchQuery}
        onCopyPath={() => copyText(currentPath)}
      />

      <div className="relative flex flex-1 min-h-0">
        <aside className="hidden w-[260px] shrink-0 flex-col overflow-hidden border-r border-[color:color-mix(in_srgb,var(--cp-border)_60%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-surface)_82%,transparent)] px-2 pt-2 md:flex">
          <Sidebar
            dfsRoots={fileBrowserSnapshot.dfsRoots}
            devices={fileBrowserSnapshot.devices}
            topics={fileBrowserSnapshot.topics}
            activePath={currentPath}
            activeTopicId={activeTopicId}
            advancedMode={advancedMode}
            onToggleAdvanced={setAdvancedMode}
            onNavigate={navigate}
            onSelectTopic={handleSelectTopic}
          />
        </aside>

        <main className="flex min-w-0 flex-1 flex-col">
          {searchActive ? (
            <SearchResultsPanel
              hits={searchHits}
              query={searchQuery}
              onSelect={handleOpenEntry}
            />
          ) : (
            <MainContent
              entries={currentEntries}
              viewMode={viewMode}
              selectedId={selectedId}
              onSelect={handleOpenEntry}
              onOpenFolder={handleOpenFolder}
              currentPath={currentPath}
              topicContext={topicContext}
            />
          )}
        </main>

        <aside className="hidden w-[320px] shrink-0 flex-col overflow-hidden border-l border-[color:color-mix(in_srgb,var(--cp-border)_60%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-surface)_86%,transparent)] xl:flex">
          <div className="flex items-center justify-between border-b border-[color:color-mix(in_srgb,var(--cp-border)_60%,transparent)] px-4 py-2">
            <span className="shell-kicker">
              {t('filebrowser.preview.title', 'Preview & Meta')}
            </span>
            <PanelRight size={14} className="text-[color:var(--cp-muted)]" />
          </div>
          <div className="flex-1 overflow-hidden">
            <PreviewPanel
              entry={selectedEntry}
              topics={fileBrowserSnapshot.topics}
              onJumpToTopic={handleSelectTopic}
              onJumpToPath={navigate}
              embedded
            />
          </div>
        </aside>
      </div>

      <StatusBar
        currentPath={currentPath}
        totalCount={currentEntries.length}
        selection={selectedEntry}
        onCopy={copyText}
      />

      {toast ? (
        <div className="pointer-events-none absolute bottom-8 left-1/2 -translate-x-1/2 rounded-full bg-black/80 px-3 py-1.5 text-xs text-white">
          {toast}
        </div>
      ) : null}
    </div>
  )
}
