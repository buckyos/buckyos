/**
 * React binding for FileItemList. One instance per pane — dual panes never
 * share lists; cross-pane consistency for the same collection rides on
 * reader.watch invalidation, not on a shared cache.
 */

import { useCallback, useEffect, useMemo, useSyncExternalStore } from 'react'
import type { SortDir, SortKey } from '../types'
import type { FileItemList } from './FileItemList'
import { FileItemListImpl } from './FileItemList'

export function useFolderList(url: string, sortKey: SortKey, sortDir: SortDir): FileItemList {
  const list = useMemo(() => new FileItemListImpl(url), [url])

  useEffect(() => () => list.dispose(), [list])

  // Applying the query from an effect (not render) keeps reader creation /
  // watch subscription out of render and re-arms the list after a StrictMode
  // dispose.
  useEffect(() => {
    list.setQuery(sortKey, sortDir)
  }, [list, sortKey, sortDir])

  const subscribe = useCallback((onChange: () => void) => list.subscribe(onChange), [list])
  useSyncExternalStore(
    subscribe,
    () => list.snapshot,
    () => list.snapshot,
  )

  return list
}
