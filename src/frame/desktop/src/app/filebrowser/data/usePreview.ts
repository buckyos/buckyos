/**
 * Preview enrichment state (UI_DATAMODEL.md §4.5).
 *
 * A preview target exists only for a single selection. The selected identity
 * stays visible immediately (from the FileItem itself); enrichment — meta
 * namespaces, topic joins, story — resolves through a pluggable enricher so
 * the panel renders skeletons and per-section empty states instead of
 * assuming synchronous data. The mock enricher simulates latency over the
 * already-projected entry; the NFSP enricher resolves `get_meta` namespaces
 * (§8.2 meta mapping) and merges them into the entry projection.
 */

import { useEffect, useRef, useState } from 'react'
import type { Topic } from '../types'
import type { FileItem } from './FolderReader'
import type { PreviewState } from './state'
import { dataIdle, dataLoading, dataSuccess } from './state'

export type PreviewEnricher = (item: FileItem) => Promise<FileItem>

/** Default: identity — base attributes only, optional fields stay absent. */
let enricher: PreviewEnricher = async (item) => item

export function registerPreviewEnricher(next: PreviewEnricher): () => void {
  const previous = enricher
  enricher = next
  return () => {
    if (enricher === next) enricher = previous
  }
}

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
    void enricher(item)
      .catch(() => item) // enrichment failure keeps base attributes (§4.5)
      .then((enriched) => {
        if (token.current !== current) return
        // Topic chips join known topic ids; unknown ids are ignored without
        // failing the preview (§2.12).
        const joined = (enriched.entry.topicIds ?? [])
          .map((id) => topics.find((topic) => topic.id === id))
          .filter((topic): topic is Topic => !!topic)
        setState(dataSuccess({ item: enriched, topics: joined }))
      })
  }, [item, topics])

  return state
}
