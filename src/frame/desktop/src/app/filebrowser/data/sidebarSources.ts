/**
 * Sidebar data sources (UI_DATAMODEL.md §4.3).
 *
 * DFS roots, the computer (host realm) and topics each load through their own
 * async state so one unavailable source never blanks the whole browser. Folder
 * trees load their children on expand (`folderChildren`). Collections stay a
 * live store projection and are composed by the view.
 */

import { useCallback, useEffect, useRef, useState } from 'react'
import type { DfsNode, HostInfo, Topic, TreeFolder } from '../types'
import type { DataState } from './state'
import { dataError, dataLoading, dataSuccess, toUiError } from './state'

export interface FolderChildrenPage {
  folders: TreeFolder[]
  /** Continues the listing ("More…"); absent on the last page. */
  nextCursor?: string
}

export interface SidebarSourceFetchers {
  dfs(): Promise<DfsNode[]>
  /** The computer behind a Desktop Lite node, when the backend offers it to this user. */
  host(): Promise<HostInfo | null>
  topics(): Promise<Topic[]>
  /** Sub-folders of a dfs:// folder, by name, one page at a time. */
  children(url: string, cursor?: string): Promise<FolderChildrenPage>
}

let fetchers: SidebarSourceFetchers | null = null

export function registerSidebarSources(next: SidebarSourceFetchers): () => void {
  fetchers = next
  return () => {
    if (fetchers === next) fetchers = null
  }
}

export interface SidebarSource<T> {
  state: DataState<T>
  retry: () => void
}

function useSidebarSource<T>(load: () => Promise<T>): SidebarSource<T> {
  const [state, setState] = useState<DataState<T>>(() => dataLoading<T>())
  const token = useRef(0)

  const run = useCallback(() => {
    const current = ++token.current
    // Refresh keeps the last successful projection visible (§4.3 Progress).
    setState((prev) => dataLoading(prev.data))
    load()
      .then((data) => {
        if (token.current !== current) return
        setState(dataSuccess(data))
      })
      .catch((err: unknown) => {
        if (token.current !== current) return
        setState((prev) => dataError(toUiError(err), prev.data))
      })
  }, [load])

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- mount kick-off; the state already starts as 'loading', so no cascading render
    run()
    return () => {
      token.current += 1
    }
  }, [run])

  return { state, retry: run }
}

const missing = () =>
  Promise.reject(new Error('filebrowser: no sidebar sources registered'))

export function useSidebarDfs(): SidebarSource<DfsNode[]> {
  return useSidebarSource(useCallback(() => (fetchers ? fetchers.dfs() : missing()), []))
}

export function useSidebarHost(): SidebarSource<HostInfo | null> {
  return useSidebarSource(useCallback(() => (fetchers ? fetchers.host() : missing()), []))
}

export function useSidebarTopics(): SidebarSource<Topic[]> {
  return useSidebarSource(useCallback(() => (fetchers ? fetchers.topics() : missing()), []))
}

/** Session cache of the first page per folder; "More…" pages are not cached. */
const childCache = new Map<string, Promise<FolderChildrenPage>>()

export function dfsFolderChildren(url: string, cursor?: string): Promise<FolderChildrenPage> {
  if (!fetchers) return missing()
  if (cursor) return fetchers.children(url, cursor)
  let cached = childCache.get(url)
  if (!cached) {
    cached = fetchers.children(url).catch((err: unknown) => {
      childCache.delete(url)
      throw err
    })
    childCache.set(url, cached)
  }
  return cached
}

const childListeners = new Map<string, Set<() => void>>()

/** Drops cached children of `url` (a folder was created, renamed or removed in it) and reloads open tree nodes. */
export function forgetFolderChildren(url: string) {
  childCache.delete(url)
  childListeners.get(url)?.forEach((listener) => listener())
}

export function onFolderChildrenChanged(url: string, listener: () => void): () => void {
  let set = childListeners.get(url)
  if (!set) {
    set = new Set()
    childListeners.set(url, set)
  }
  set.add(listener)
  return () => {
    set.delete(listener)
    if (!set.size) childListeners.delete(url)
  }
}
