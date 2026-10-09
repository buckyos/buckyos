import { useCallback, useMemo } from 'react'
import type { ReadingPage, ReadingQuery } from '../datamodel/types'
import { useHomeStationStore } from '../store/context'
import { usePagedList } from './usePagedList'

export function readingViewKey(query: ReadingQuery) {
  return JSON.stringify([query.filter, query.topicId, query.search.trim().toLowerCase(), query.showFiltered])
}

export function useReadingList(query: ReadingQuery) {
  const store = useHomeStationStore()
  const viewKey = readingViewKey(query)
  const fetchPage = useCallback((scope: string, cursor: string | null): Promise<ReadingPage> => {
    const [filter, topicId, search, showFiltered] = JSON.parse(scope) as [ReadingQuery['filter'], string | null, string, boolean]
    return store.listReading({ filter, topicId, search, showFiltered }, cursor)
  }, [store])
  const list = usePagedList<ReadingPage>(viewKey, fetchPage, ['reading'])
  const objIds = useMemo(() => list.pages.flatMap(page => page.objIds), [list.pages])
  return { ...list, objIds, viewKey, meta: list.pages[0] }
}
