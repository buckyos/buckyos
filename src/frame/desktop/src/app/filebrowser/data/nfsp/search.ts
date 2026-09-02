/**
 * NFSP search provider (UI_DATAMODEL.md §8.2 search rows).
 *
 * v1 serves `search.name` with a path cursor; the response mapping keeps
 * unknown future match modes (§2.9 extensible reasons) and produces its own
 * display-safe evidence text instead of forwarding server copy.
 */

import type { SearchHit, WireRef } from '../../../../api/nfsp_client'
import type { SearchResultItem, SearchResultPage } from '../../types'
import { classifyFileKind } from '../fileKinds'
import { registerSearchProvider } from '../search'
import { ensureSession, nfspClient } from './client'
import { nfspToError } from './errors'
import { cyfsToDfsPath, refIdOf, unixToIso } from './mapping'

const PAGE_LIMIT = 30

/** NFSP match_source → UI SearchReason; unknown modes pass through (§2.9). */
function reasonOf(matchSource: string): string {
  if (matchSource === 'name') return 'filename'
  if (matchSource === 'fulltext') return 'fulltext'
  if (matchSource === 'semantic') return 'ai_semantic'
  return matchSource
}

function detailOf(hit: SearchHit, query: string): string {
  const matcher =
    typeof hit.explain?.matcher === 'string' ? (hit.explain.matcher as string) : hit.match_source
  if (matcher.startsWith('name')) return `File name contains “${query}”`
  return `Matched via ${matcher}`
}

function mapHit(hit: SearchHit, query: string): SearchResultItem | null {
  const path = cyfsToDfsPath(hit.canonical_path) ?? hit.canonical_path
  const name = path.split('/').filter(Boolean).pop() ?? path
  const ref = hit.ref as WireRef | undefined
  if (!ref || typeof ref !== 'object' || !('type' in ref)) return null
  const kind = typeof hit.kind === 'string' ? hit.kind : null
  const isFolder = kind === 'dir' || kind === 'collection' || kind === 'group'
  const item: SearchResultItem = {
    entry: {
      id: refIdOf(ref),
      name,
      kind: isFolder ? 'folder' : kind === 'file' ? classifyFileKind(name) : 'other',
      path,
      modifiedAt: unixToIso(typeof hit.mtime === 'number' ? hit.mtime : undefined),
    },
    reason: reasonOf(hit.match_source),
    detail: detailOf(hit, query),
  }
  if (typeof hit.size === 'number' && kind === 'file') item.entry.sizeBytes = hit.size
  if (typeof hit.score === 'number') item.score = hit.score
  return item
}

export function registerNfspSearchProvider() {
  return registerSearchProvider({
    async search(request): Promise<SearchResultPage> {
      await ensureSession()
      try {
        const result = await nfspClient().raw.search(
          request.query,
          {
            limit: PAGE_LIMIT,
            cursor: request.cursor,
            scope: request.scope,
          },
          ['base'],
        )
        return {
          items: result.hits
            .map((hit) => mapHit(hit, request.query))
            .filter((item): item is SearchResultItem => item !== null),
          partial: result.partial,
          sources: result.sources.map((source) => ({
            mode: source.mode,
            state: source.state,
            tookMs: source.took_ms,
            reason: source.reason,
          })),
          nextCursor: result.next_cursor,
        }
      } catch (err) {
        throw nfspToError(err)
      }
    },
  })
}
