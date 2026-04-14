import { useMediaQuery } from '@mui/material'
import { useCallback, useMemo, useState } from 'react'
import { ArrowLeft, Info, PanelRight, Menu as MenuIcon } from 'lucide-react'
import { useI18n } from '../../i18n/provider'
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
import type { BrowserTab, FileEntry, TriggerRule, ViewMode } from './types'

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

  const handleSelectTrigger = (rule: TriggerRule) => {
    showToast(
      t('filebrowser.toast.triggerInspected', 'Inspected {{name}}', { name: rule.name }),
    )
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

  // ─── Mobile layout ───
  if (isMobile) {
    return (
      <div className="flex h-full w-full flex-col overflow-hidden" style={{ background: 'var(--cp-bg)' }}>
        <div className="flex items-center gap-2 border-b border-[color:color-mix(in_srgb,var(--cp-border)_60%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-surface)_90%,transparent)] px-3 py-2">
          <button
            type="button"
            className="inline-flex items-center gap-1 rounded-full border border-[color:var(--cp-border)] px-2.5 py-1 text-xs text-[color:var(--cp-text)]"
            onClick={() => setMobileSidebarOpen(true)}
          >
            <MenuIcon size={14} /> {t('filebrowser.mobile.places', 'Places')}
          </button>
          <div className="min-w-0 flex-1 truncate text-sm font-semibold text-[color:var(--cp-text)]">
            {activeTab?.title}
          </div>
          {selectedEntry ? (
            <button
              type="button"
              className="inline-flex items-center gap-1 rounded-full border border-[color:var(--cp-border)] px-2.5 py-1 text-xs text-[color:var(--cp-text)]"
              onClick={() => setMobilePreviewOpen(true)}
            >
              <Info size={14} />
            </button>
          ) : null}
        </div>

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
          compact
        />

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
            />
          )}
        </div>

        <StatusBar
          currentPath={currentPath}
          totalCount={currentEntries.length}
          selection={selectedEntry}
          onCopy={copyText}
        />

        {/* Sidebar drawer */}
        {mobileSidebarOpen ? (
          <div className="absolute inset-0 z-30 flex">
            <div
              className="absolute inset-0 bg-black/40"
              onClick={() => setMobileSidebarOpen(false)}
            />
            <div
              className="relative flex h-full w-[82%] max-w-[340px] flex-col gap-3 border-r border-[color:var(--cp-border)] p-3"
              style={{ background: 'var(--cp-surface)' }}
            >
              <div className="flex items-center gap-2">
                <button
                  type="button"
                  onClick={() => setMobileSidebarOpen(false)}
                  className="inline-flex items-center gap-1 rounded-full border border-[color:var(--cp-border)] px-2.5 py-1 text-xs"
                >
                  <ArrowLeft size={12} /> {t('common.back', 'Back')}
                </button>
                <span className="shell-kicker">
                  {t('filebrowser.mobile.navigation', 'Navigation')}
                </span>
              </div>
              <Sidebar
                dfsRoots={fileBrowserSnapshot.dfsRoots}
                devices={fileBrowserSnapshot.devices}
                topics={fileBrowserSnapshot.topics}
                triggers={fileBrowserSnapshot.triggers}
                activePath={currentPath}
                activeTopicId={activeTopicId}
                advancedMode={advancedMode}
                onToggleAdvanced={setAdvancedMode}
                onNavigate={navigate}
                onSelectTopic={handleSelectTopic}
                onSelectTrigger={handleSelectTrigger}
                compact
                onAfterNavigate={() => setMobileSidebarOpen(false)}
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
            triggers={fileBrowserSnapshot.triggers}
            activePath={currentPath}
            activeTopicId={activeTopicId}
            advancedMode={advancedMode}
            onToggleAdvanced={setAdvancedMode}
            onNavigate={navigate}
            onSelectTopic={handleSelectTopic}
            onSelectTrigger={handleSelectTrigger}
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
