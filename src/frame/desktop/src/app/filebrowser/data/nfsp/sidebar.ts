/**
 * NFSP sidebar sources (UI_DATAMODEL.md §4.3).
 *
 * DFS roots come from listing the namespace root (export roots); their
 * sub-folders load on expand. The computer comes from `hello.realms` (`host`,
 * advertised to the zone owner on Desktop Lite). AI topic views are a later
 * stage (§9 item 9), so that source resolves to its empty state.
 */

import type { DfsNode } from '../../types'
import { registerSidebarSources } from '../sidebarSources'
import { dfsPathOf, normalizeUrl } from '../urls'
import { effectiveListLimit, ensureSession, nfspClient } from './client'
import { nfspToError } from './errors'

const TREE_PAGE = 200

function rootKind(name: string): DfsNode['kind'] {
  if (name === 'home') return 'home'
  if (name === 'public') return 'public'
  if (name === 'shared') return 'shared'
  if (name === 'private' || name === 'privacy') return 'privacy'
  return 'generic'
}

async function loadDfsRoots(): Promise<DfsNode[]> {
  await ensureSession()
  const listing = await nfspClient()
    .list('/', { limit: 50, order: 'name' })
    .catch((err: unknown) => {
      throw nfspToError(err)
    })
  if (!listing) throw nfspToError({ code: 'NOT_FOUND', message: 'root unavailable' })
  return listing.entries.map((entry): DfsNode => ({
    id: `dfs-${entry.name}`,
    name: entry.name.charAt(0).toUpperCase() + entry.name.slice(1),
    path: `/${entry.name}`,
    kind: rootKind(entry.name),
  }))
}

async function loadChildren(url: string, cursor?: string) {
  const path = dfsPathOf(url)
  if (path === null) return { folders: [] }
  await ensureSession()
  const listing = await nfspClient()
    .list(path, { limit: effectiveListLimit(TREE_PAGE), order: 'name', filter: { kind: ['dir'] }, cursor }, ['base', 'access'], { cache: 'no-cache' })
    .catch((err: unknown) => {
      throw nfspToError(err)
    })
  if (!listing) throw nfspToError({ code: 'NOT_FOUND', message: 'folder unavailable' })
  return {
    folders: listing.entries
      .filter((entry) => entry.target.kind === 'dir')
      .map((entry) => ({
        name: entry.name,
        url: normalizeUrl(path === '/' ? `/${entry.name}` : `${path}/${entry.name}`),
        locked: entry.target.access_level === 'none',
      })),
    nextCursor: listing.truncated && listing.next_cursor ? listing.next_cursor : undefined,
  }
}

export function registerNfspSidebarSources() {
  return registerSidebarSources({
    dfs: loadDfsRoots,
    host: async () => {
      const hello = await ensureSession()
      const realm = hello.realms.find((item) => item.id === 'host')
      return realm ? { label: realm.label || 'Computer' } : null
    },
    topics: async () => [],
    children: loadChildren,
  })
}
