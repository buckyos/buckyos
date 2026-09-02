/**
 * Async search provider boundary (UI_DATAMODEL.md §2.9/§4.4).
 *
 * Search is a scoped query against the backend — never an emulation over the
 * loaded listing window. The UI consumes `SearchViewState` from `useSearch`;
 * the active provider (mock today, NFSP `search` later) owns execution and
 * cursor continuation.
 */

import { useCallback, useEffect, useRef, useState } from 'react'
import type { SearchResultPage, SearchResultItem } from '../types'
import type { SearchViewState } from './state'
import { dataError, dataIdle, dataLoading, dataSuccess, toUiError } from './state'

export interface SearchRequest {
  query: string
  scope?: string
  /** Continuation from a previous page's `nextCursor`. */
  cursor?: string
}

export interface SearchProvider {
  search(request: SearchRequest): Promise<SearchResultPage>
}

let activeProvider: SearchProvider | null = null

export function registerSearchProvider(provider: SearchProvider): () => void {
  activeProvider = provider
  return () => {
    if (activeProvider === provider) activeProvider = null
  }
}

/** Grouping derivation (§2.9): unknown reasons surface, never dropped. */
export type SearchReasonGroup = 'traditional' | 'ai' | 'other'

export function searchReasonGroup(reason: string): SearchReasonGroup {
  if (reason === 'filename' || reason === 'folder' || reason === 'fulltext') {
    return 'traditional'
  }
  if (reason === 'ai_semantic' || reason === 'ai_topic') return 'ai'
  return 'other'
}

export interface SearchController {
  /** Accumulated result page — items merged across cursor continuations. */
  state: SearchViewState
  /** Request the next cursor page (no-op when none or already loading). */
  loadMore: () => void
  /** Re-run the query from scratch. */
  retry: () => void
}

const DEBOUNCE_MS = 200

/**
 * Blank input never reaches the provider (§3). Stale responses are discarded
 * by run token; cursor pages append to the accumulated items.
 */
export function useSearch(query: string, scope?: string): SearchController {
  const [state, setState] = useState<SearchViewState>(dataIdle<SearchResultPage>)
  const stateRef = useRef(state)
  const runToken = useRef(0)
  const trimmed = query.trim()

  useEffect(() => {
    stateRef.current = state
  }, [state])

  const run = useCallback(
    (cursor?: string, previous?: SearchResultPage | null) => {
      if (!trimmed) return
      const token = ++runToken.current
      setState(dataLoading(previous ?? null))
      const provider = activeProvider
      if (!provider) {
        setState(
          dataError({
            code: 'UNSUPPORTED',
            messageKey: 'filebrowser.search.noProvider',
            fallback: 'Search is not available',
            retryable: false,
          }),
        )
        return
      }
      provider
        .search({ query: trimmed, scope, cursor })
        .then((page) => {
          if (runToken.current !== token) return
          const merged: SearchResultPage = previous
            ? { ...page, items: [...previous.items, ...page.items] }
            : page
          setState(dataSuccess(merged))
        })
        .catch((err: unknown) => {
          if (runToken.current !== token) return
          setState(dataError(toUiError(err), previous ?? null))
        })
    },
    [trimmed, scope],
  )

  useEffect(() => {
    if (!trimmed) {
      runToken.current += 1
      // eslint-disable-next-line react-hooks/set-state-in-effect -- reset to idle when the query is cleared; no fetch happens for blank input
      setState(dataIdle())
      return
    }
    const timer = window.setTimeout(() => run(), DEBOUNCE_MS)
    return () => window.clearTimeout(timer)
  }, [trimmed, scope, run])

  const loadMore = useCallback(() => {
    const current = stateRef.current
    if (current.status !== 'success') return
    const cursor = current.data?.nextCursor
    if (cursor) run(cursor, current.data)
  }, [run])

  const retry = useCallback(() => run(), [run])

  return { state, loadMore, retry }
}

/** Convenience for renderers: bucket accumulated items by reason group. */
export function groupSearchItems(items: SearchResultItem[]) {
  const groups: Record<SearchReasonGroup, SearchResultItem[]> = {
    traditional: [],
    ai: [],
    other: [],
  }
  for (const item of items) groups[searchReasonGroup(item.reason)].push(item)
  return groups
}
