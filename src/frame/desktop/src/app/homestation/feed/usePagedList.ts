import { useCallback } from 'react'
import useSWRInfinite from 'swr/infinite'
import type { StoreDomain } from '../store/types'
import { useHomeStationStore, useStoreRevalidate } from '../store/context'

export interface PagedList<Page> {
  pages: Page[]
  isLoading: boolean
  isLoadingMore: boolean
  error: unknown
  hasMore: boolean
  loadMore: () => void
  reload: () => void
}

export function usePagedList<Page extends { nextCursor: string | null }>(
  scope: string,
  fetchPage: (scope: string, cursor: string | null) => Promise<Page>,
  domains: StoreDomain[],
): PagedList<Page> {
  const store = useHomeStationStore()
  const getKey = useCallback(
    (_index: number, previous: Page | null) => (previous && !previous.nextCursor ? null : (['hs', store.id, scope, previous?.nextCursor ?? null] as const)),
    [scope, store.id],
  )
  const swr = useSWRInfinite<Page>(getKey, ([, , pageScope, cursor]: readonly [string, string, string, string | null]) => fetchPage(pageScope, cursor), {
    revalidateOnFocus: false,
    revalidateFirstPage: false,
    shouldRetryOnError: false,
  })
  const { mutate, setSize, size } = swr
  useStoreRevalidate(domains, () => void mutate())
  const pages = swr.data ?? []
  const last = pages[pages.length - 1]
  return {
    pages,
    isLoading: !swr.data && !swr.error,
    isLoadingMore: !!swr.data && size > pages.length && !swr.error,
    error: swr.error,
    hasMore: !!last?.nextCursor,
    loadMore: () => void setSize(current => current + 1),
    reload: () => void mutate(),
  }
}
