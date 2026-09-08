/**
 * Mock async search provider (UI_DATAMODEL.md §7.2 search fixtures).
 *
 * Deterministic scenarios are selected by query prefix so error/partial
 * coverage never depends on timing:
 *
 *   error:<q>     the whole search fails (retry keeps failing)
 *   partial:<q>   results return but sources are degraded → partial coverage
 *   unknown:<q>   hits carry an unregistered reason (renderer fallback path)
 *
 * Results page at 8 items per cursor page; `nextCursor` is an opaque offset.
 */

import type { SearchResultPage, SearchSourceStatus } from '../types'
import type { SearchRequest } from '../data/search'
import { registerSearchProvider } from '../data/search'
import { mockDelay } from '../data/mockReader'
import { searchFiles, mockEntriesAtPath } from './data'
import { sortEntriesForQuery } from '../data/sortEntries'
import { dfsPathOf } from '../data/urls'

function scopedSearch(query: string, request: SearchRequest) {
  const scope = request.scope
  if (scope && dfsPathOf(scope) === null) throw new Error('Search in this location is not supported. Choose all accessible files.')
  const path = scope ? dfsPathOf(scope)! : null
  return searchFiles(query).filter((hit) => (!path || path === '/' || hit.entry.path.startsWith(`${path}/`)) && (!request.kind || hit.entry.kind === request.kind) && (!request.modifiedSince || Date.parse(hit.entry.modifiedAt) >= Date.parse(request.modifiedSince)))
}

const PAGE_SIZE = 8

function pageOf(
  items: SearchResultPage['items'],
  cursor: string | undefined,
  partial: boolean,
  sources: SearchSourceStatus[],
): SearchResultPage {
  const offset = cursor ? Number.parseInt(cursor, 10) || 0 : 0
  const slice = items.slice(offset, offset + PAGE_SIZE)
  const nextOffset = offset + slice.length
  return {
    items: slice,
    partial,
    sources,
    nextCursor: nextOffset < items.length ? String(nextOffset) : undefined,
  }
}

async function mockSearch(request: SearchRequest): Promise<SearchResultPage> {
  await mockDelay(80, 180)
  const raw = request.query

  if (raw.startsWith('stress-index:')) {
    const entries = sortEntriesForQuery(mockEntriesAtPath('/home/stress-10k') ?? [], 'name', 'asc', true)
    const entry = entries[Number(raw.split(':')[1])]
    return pageOf(entry ? [{ entry, reason: 'filename', detail: 'Diagnostic result in a large folder' }] : [], undefined, false, [])
  }
  if (raw.startsWith('error:')) {
    throw new Error('Search backend unavailable (mock scenario)')
  }

  if (raw.startsWith('partial:')) {
    const hits = scopedSearch(raw.slice('partial:'.length), request)
    return pageOf(hits, request.cursor, true, [
      { mode: 'filename', state: 'ok', tookMs: 12 },
      { mode: 'fulltext', state: 'degraded', tookMs: 480, reason: 'index catching up' },
      { mode: 'semantic', state: 'error', reason: 'embedding service offline' },
    ])
  }

  if (raw.startsWith('unknown:')) {
    const hits = scopedSearch(raw.slice('unknown:'.length), request).map((hit) => ({
      ...hit,
      reason: 'graph_related',
      detail: `${hit.detail} (via knowledge graph)`,
    }))
    return pageOf(hits, request.cursor, false, [
      { mode: 'graph', state: 'ok', tookMs: 51 },
    ])
  }

  const hits = scopedSearch(raw, request)
  return pageOf(hits, request.cursor, false, [
    { mode: 'filename', state: 'ok', tookMs: 9 },
    { mode: 'semantic', state: 'ok', tookMs: 34 },
  ])
}

export function registerMockSearchProvider() {
  return registerSearchProvider({ search: mockSearch })
}
