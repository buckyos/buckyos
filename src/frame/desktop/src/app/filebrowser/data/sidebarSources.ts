/**
 * Sidebar data sources (UI_DATAMODEL.md §4.3).
 *
 * DFS roots, devices, and topics each load through their own async state so
 * one unavailable source never blanks the whole browser. Collections stay a
 * live store projection (mock in-memory today) and are composed by the view.
 */

import { useCallback, useEffect, useRef, useState } from 'react'
import type { DeviceNode, DfsNode, Topic } from '../types'
import type { DataState } from './state'
import { dataError, dataLoading, dataSuccess, toUiError } from './state'

export interface SidebarSourceFetchers {
  dfs(): Promise<DfsNode[]>
  devices(): Promise<DeviceNode[]>
  topics(): Promise<Topic[]>
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

export function useSidebarDevices(): SidebarSource<DeviceNode[]> {
  return useSidebarSource(useCallback(() => (fetchers ? fetchers.devices() : missing()), []))
}

export function useSidebarTopics(): SidebarSource<Topic[]> {
  return useSidebarSource(useCallback(() => (fetchers ? fetchers.topics() : missing()), []))
}
