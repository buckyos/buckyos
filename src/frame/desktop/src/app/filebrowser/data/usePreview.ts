/**
 * Preview enrichment state (UI_DATAMODEL.md §4.5).
 *
 * A preview target exists only for a single selection. The selected identity
 * stays visible immediately (from the FileItem itself); enrichment — meta
 * namespaces, topic joins, story — resolves through this async state so the
 * panel renders skeletons and per-section empty states instead of assuming
 * synchronous data. The mock resolves from the already-projected entry;
 * the NFSP adapter later resolves `get_meta` namespaces here.
 */

import { useEffect, useRef, useState } from 'react'
import type { Topic } from '../types'
import type { FileItem } from './FolderReader'
import type { PreviewState } from './state'
import { dataIdle, dataLoading, dataSuccess } from './state'
import { mockDelay } from './mockReader'

export function usePreview(item: FileItem | null, topics: Topic[]): PreviewState {
  const [state, setState] = useState<PreviewState>(dataIdle)
  const token = useRef(0)

  useEffect(() => {
    if (!item) {
      token.current += 1
      // eslint-disable-next-line react-hooks/set-state-in-effect -- selection cleared: drop enrichment synchronously so no stale preview flashes
      setState(dataIdle())
      return
    }
    const current = ++token.current
    setState(dataLoading())
    void mockDelay(50, 120).then(() => {
      if (token.current !== current) return
      // Topic chips join known topic ids; unknown ids are ignored without
      // failing the preview (§2.12).
      const joined = (item.entry.topicIds ?? [])
        .map((id) => topics.find((topic) => topic.id === id))
        .filter((topic): topic is Topic => !!topic)
      setState(dataSuccess({ item, topics: joined }))
    })
  }, [item, topics])

  return state
}
