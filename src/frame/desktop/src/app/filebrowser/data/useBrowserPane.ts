import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react'
import type { SelectModifiers } from '../MainContent'
import type { BrowserTab, HistoryState, PaneLocation, SortDir, SortKey, ViewMode } from '../types'
import type { FileItem } from './FolderReader'
import { useFolderList } from './useFolderList'
import { useSearch } from './search'
import { displayPath, fallbackTitle, normalizeUrl, parentUrl } from './urls'

interface TabState {
  tab: BrowserTab
  location: PaneLocation
  history: HistoryState
  viewMode: ViewMode
  sortKey: SortKey
  sortDir: SortDir
  selected: Map<string, FileItem>
  anchor: string | null
  revision: number
  locationKind?: import('../types').LocationKind
}

function stateOf(tab: BrowserTab): TabState {
  const path = normalizeUrl(tab.path)
  return {
    tab: { ...tab, path },
    location: { path, query: '', scope: 'current', kind: '', modified: '', scroll: 0 },
    history: { back: [], forward: [] },
    viewMode: (localStorage.getItem('files.view') as ViewMode) === 'icon' ? 'icon' : 'list',
    sortKey: 'name', sortDir: 'asc', selected: new Map(), anchor: null, revision: 0,
  }
}

const sessions = new Map<string, { states: TabState[]; activeTabId: string }>()

export function useBrowserPane(initialTabs: BrowserTab[], sessionKey?: string) {
  const [states, setStates] = useState(() => {
    const saved = sessionKey ? sessions.get(sessionKey) : undefined
    return saved ? saved.states.map((state) => ({ ...state, selected: new Map<string, FileItem>(), revision: state.revision + 1 })) : initialTabs.map(stateOf)
  })
  const [activeTabId, setActiveTabId] = useState(() => (sessionKey ? sessions.get(sessionKey)?.activeTabId : undefined) ?? initialTabs[0]?.id ?? '')
  useLayoutEffect(() => {
    if (!sessionKey) return
    sessions.set(sessionKey, { states, activeTabId })
    if (sessions.size > 20) sessions.delete(sessions.keys().next().value!)
  }, [sessionKey, states, activeTabId])
  const active = states.find((state) => state.tab.id === activeTabId) ?? states[0]
  const activeRef = useRef(active)
  useLayoutEffect(() => { activeRef.current = active }, [active])
  const fallback = stateOf({ id: '', title: 'Home', path: '/home' })
  const state = active ?? fallback
  const { location, selected, revision } = state
  const currentUrl = location.path
  const list = useFolderList(currentUrl, state.sortKey, state.sortDir, activeTabId)
  const search = useSearch(location.query, location.scope === 'all' ? undefined : currentUrl, location.kind, location.modified)
  const pendingReveal = useRef<{ path: string; tabId: string } | null>(null)
  const [revealIndex, setRevealIndex] = useState<number | null>(null)
  const [enumerating, setEnumerating] = useState(false)
  const enumeration = useRef<AbortController | null>(null)
  const [selectionError, setSelectionError] = useState<string | null>(null)
  const [selectionNotice, setSelectionNotice] = useState(false)

  const update = useCallback((fn: (state: TabState) => TabState) => {
    setStates((prev) => { const next = prev.map((state) => state.tab.id === activeTabId ? fn(state) : state); return next.some((state, i) => state !== prev[i]) ? next : prev })
  }, [activeTabId])
  const clearSelection = useCallback(() => update((state) => ({
    ...state, selected: new Map(), anchor: null, revision: state.revision + 1,
  })), [update])
  const applySelection = useCallback((keys: string[], picked?: Map<string, FileItem>) => {
    update((state) => ({ ...state, revision: state.revision + 1,
      selected: new Map(keys.flatMap((key) => {
        const item = picked?.get(key) ?? list.loadedItemByKey(key) ?? state.selected.get(key)
        return item ? [[key, { ...item, entry: { ...item.entry } }] as const] : []
      })),
    }))
  }, [list, update])

  const searchItems = [...new Map((search.state.data?.items ?? []).filter((hit) =>
    (!location.kind || hit.entry.kind === location.kind) &&
    (!location.modified || Date.parse(hit.entry.modifiedAt) >= Date.parse(location.modified)),
  ).map((hit) => [hit.entry.id, { key: hit.entry.id, entry: hit.entry }])).values()]
  const visibleItems = location.query.trim() ? searchItems : list.loadedKeys()
    .map((key) => list.loadedItemByKey(key)).filter((item): item is FileItem => !!item)
  const selectItem = (item: FileItem, modifiers: SelectModifiers = {}) => {
    cancelEnumeration()
    setSelectionNotice(false)
    if (modifiers.shift && state.anchor) {
      const keys = visibleItems.map((item) => item.key)
      const start = keys.indexOf(state.anchor), end = keys.indexOf(item.key)
      if (start >= 0 && end >= 0) {
        setSelectionNotice(list.hasMore || (list.totalCount ?? 0) > visibleItems.length)
        applySelection(keys.slice(Math.min(start, end), Math.max(start, end) + 1), new Map(visibleItems.map((item) => [item.key, item])))
        return
      }
    }
    update((state) => {
      const next = modifiers.toggle ? new Map(state.selected) : new Map<string, FileItem>()
      if (modifiers.toggle && next.has(item.key)) next.delete(item.key)
      else next.set(item.key, { ...item, entry: { ...item.entry } })
      return { ...state, selected: next, anchor: item.key, revision: state.revision + 1 }
    })
  }
  const selectLoaded = () => { cancelEnumeration(); applySelection(visibleItems.map((item) => item.key), new Map(visibleItems.map((item) => [item.key, item]))) }
  const cancelEnumeration = useCallback(() => {
    enumeration.current?.abort()
    enumeration.current = null
    setEnumerating(false)
  }, [])
  useEffect(() => cancelEnumeration, [activeTabId, currentUrl, location.query, location.scope, location.kind, location.modified, cancelEnumeration])
  const selectAll = async () => {
    if (location.query.trim()) { selectLoaded(); return }
    cancelEnumeration()
    const controller = new AbortController()
    enumeration.current = controller
    setEnumerating(true)
    setSelectionError(null)
    try {
      const items = await list.enumerate(controller.signal)
      if (!controller.signal.aborted && activeRef.current?.tab.id === activeTabId && activeRef.current.revision === revision) {
        applySelection(items.map((item) => item.key), new Map(items.map((item) => [item.key, item])))
      }
    } catch (err) {
      if (!controller.signal.aborted) setSelectionError(err instanceof Error ? err.message : String(err))
    } finally {
      if (enumeration.current === controller) { enumeration.current = null; setEnumerating(false) }
    }
  }

  const changeLocation = useCallback((next: PaneLocation, direction?: 'back' | 'forward') => {
    cancelEnumeration()
    setRevealIndex(null)
    update((state) => ({
      ...state,
      tab: { ...state.tab, path: next.path, title: fallbackTitle(next.path) }, location: next,
      history: direction === 'back'
        ? { back: state.history.back.slice(0, -1), forward: [state.location, ...state.history.forward] }
        : direction === 'forward'
          ? { back: [...state.history.back, state.location], forward: state.history.forward.slice(1) }
          : { back: [...state.history.back, state.location], forward: [] },
      selected: new Map(), anchor: null, revision: state.revision + 1,
    }))
  }, [update, cancelEnumeration])
  const navigate = useCallback((input: string) => {
    const path = normalizeUrl(input)
    if (path === currentUrl && !location.query) return
    changeLocation({ path, query: '', scope: 'current', kind: '', modified: '', scroll: 0 })
  }, [currentUrl, location.query, changeLocation])
  const setSearchQuery = (query: string) => {
    update((state) => ({ ...state,
      history: query && !state.location.query ? { back: [...state.history.back, state.location], forward: [] } : state.history,
      location: { ...state.location, query, scroll: 0 }, selected: new Map(), anchor: null, revision: state.revision + 1,
    }))
  }
  const setSearchScope = (scope: 'current' | 'all') => update((state) => ({ ...state,
    location: { ...state.location, scope, scroll: 0 }, selected: new Map(), revision: state.revision + 1,
  }))
  const setSearchFilter = (kind: string, modified: string) => update((state) => ({ ...state,
    location: { ...state.location, kind, modified, scroll: 0 }, selected: new Map(), revision: state.revision + 1,
  }))

  useEffect(() => list.subscribe(() => {
    const caps = list.capabilities
    update((state) => {
      const sortKey = state.locationKind === caps.kind && caps.sortKeys.includes(state.sortKey) ? state.sortKey : caps.defaultSortKey
      const sortDir = caps.sortDirs && !caps.sortDirs.includes(state.sortDir) ? caps.sortDirs[0] : state.sortDir
      const title = list.meta?.title ?? state.tab.title
      if (state.locationKind === caps.kind && state.sortKey === sortKey && state.sortDir === sortDir && state.tab.title === title) return state
      return { ...state, locationKind: caps.kind, sortKey, sortDir, tab: { ...state.tab, title } }
    })
  }), [list, update])

  useEffect(() => {
    const target = pendingReveal.current
    if (!target || target.tabId !== activeTabId || list.status !== 'ready') return
    const controller = new AbortController()
    pendingReveal.current = null
    setEnumerating(true)
    void list.findPath(target.path, controller.signal).then((found) => {
      if (controller.signal.aborted) return
      if (found) {
        applySelection([found.item.key], new Map([[found.item.key, found.item]]))
        setRevealIndex(found.index)
      } else setSelectionError('File is no longer in this location')
    }).catch((err: unknown) => {
      if (!controller.signal.aborted) setSelectionError(err instanceof Error ? err.message : String(err))
    }).finally(() => { if (!controller.signal.aborted) setEnumerating(false) })
    enumeration.current = controller
  }, [activeTabId, list, list.status, applySelection])

  return {
    tabs: states.map((state) => state.tab), activeTabId, setActiveTabId,
    activeTab: active?.tab ?? null, currentUrl, activeHistory: state.history, list,
    viewMode: state.viewMode,
    setViewMode: (viewMode: ViewMode) => { localStorage.setItem('files.view', viewMode); update((state) => ({ ...state, viewMode })) },
    sortKey: state.sortKey, sortDir: state.sortDir,
    setSortKey: (sortKey: SortKey) => update((state) => ({ ...state, sortKey })),
    setSortDir: (sortDir: SortDir) => update((state) => ({ ...state, sortDir })),
    selectedKeys: new Set(selected.keys()), selectedItemsMap: selected,
    applySelection, selectItem, selectAll, selectLoaded, clearSelection,
    enumerating, cancelEnumeration, selectionError, selectionNotice,
    searchQuery: location.query, setSearchQuery, search, searchItems,
    searchScope: location.scope, setSearchScope, searchKind: location.kind, searchModified: location.modified, setSearchFilter,
    scroll: location.scroll, setScroll: (scroll: number) => update((state) => ({ ...state, location: { ...state.location, scroll } })),
    revealIndex,
    contextToken: `${activeTabId}:${revision}`,
    isCurrent: () => activeRef.current?.tab.id === activeTabId && activeRef.current.revision === revision,
    reconcileMovedEntries: (ids: Set<string>) => setStates((prev) => prev.map((current) => {
      const captured = states.find((state) => state.tab.id === current.tab.id)
      if (!captured || captured.revision !== current.revision) return current
      const selected = new Map([...current.selected].filter(([, item]) => !ids.has(item.entry.id)))
      return selected.size === current.selected.size ? current : { ...current, selected, anchor: null, revision: current.revision + 1 }
    })),
    reconcileSelection: (remaining: FileItem[]) => setStates((prev) => prev.map((state) =>
      state.tab.id === activeTabId && state.revision === revision
        ? { ...state, selected: new Map(remaining.map((item) => [item.key, item])), revision: state.revision + 1 }
        : state)),
    navigate,
    revealOriginal: (path: string) => { pendingReveal.current = { path: displayPath(path), tabId: activeTabId }; navigate(parentUrl(normalizeUrl(path)) ?? '/'); if (normalizeUrl(parentUrl(normalizeUrl(path)) ?? '/') === currentUrl) list.reload() },
    back: () => { const prev = state.history.back.at(-1); if (prev) changeLocation(prev, 'back') },
    forward: () => { const next = state.history.forward[0]; if (next) changeLocation(next, 'forward') },
    goUp: () => { const up = parentUrl(currentUrl); if (up) navigate(up) },
    adoptTab: (tab: BrowserTab, history?: HistoryState, snapshot?: TabState) => { const next = snapshot ? { ...snapshot, tab: { ...tab, path: normalizeUrl(tab.path) } } : stateOf(tab); if (history) next.history = history; setStates((prev) => [...prev, next]); setActiveTabId(tab.id) },
    detachTab: (id: string) => {
      const removed = states.find((state) => state.tab.id === id)
      if (!removed) return null
      setStates((prev) => prev.filter((state) => state.tab.id !== id))
      if (id === activeTabId) setActiveTabId(states.find((state) => state.tab.id !== id)?.tab.id ?? '')
      return { tab: removed.tab, history: removed.history, state: removed }
    },
  }
}
