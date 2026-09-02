/**
 * NFSP sidebar sources (UI_DATAMODEL.md §4.3).
 *
 * DFS roots come from listing the namespace root (export roots) plus one
 * level of child folders for the tree. Devices and topics have no NFSP v1
 * backing (referral device readers and AI topic views are later stages —
 * §9 items 9/10), so those sources resolve to their empty states instead of
 * pretending data exists.
 */

import type { DfsNode } from '../../types'
import { registerSidebarSources } from '../sidebarSources'
import { ensureSession, nfspClient } from './client'
import { nfspToError } from './errors'

function rootKind(name: string): DfsNode['kind'] {
  if (name === 'home') return 'home'
  if (name === 'public') return 'public'
  if (name === 'shared') return 'shared'
  if (name === 'private' || name === 'privacy') return 'privacy'
  return 'generic'
}

async function childFolders(rootName: string): Promise<DfsNode[] | undefined> {
  const listing = await nfspClient()
    .list(`/${rootName}`, { limit: 50, order: 'name', filter: { kind: ['dir'] } })
    .catch(() => null)
  if (!listing) return undefined
  const children = listing.entries
    .filter((entry) => entry.target.kind === 'dir')
    .map(
      (entry): DfsNode => ({
        id: `dfs-${rootName}-${entry.name}`,
        name: entry.name,
        path: `/${rootName}/${entry.name}`,
        kind: 'generic',
      }),
    )
  return children.length ? children : undefined
}

async function loadDfsRoots(): Promise<DfsNode[]> {
  await ensureSession()
  const listing = await nfspClient()
    .list('/', { limit: 50, order: 'name' })
    .catch((err: unknown) => {
      throw nfspToError(err)
    })
  if (!listing) throw nfspToError({ code: 'NOT_FOUND', message: 'root unavailable' })
  return Promise.all(
    listing.entries.map(async (entry): Promise<DfsNode> => {
      const name = entry.name
      return {
        id: `dfs-${name}`,
        name: name.charAt(0).toUpperCase() + name.slice(1),
        path: `/${name}`,
        kind: rootKind(name),
        children: await childFolders(name),
      }
    }),
  )
}

export function registerNfspSidebarSources() {
  return registerSidebarSources({
    dfs: loadDfsRoots,
    // Empty states, not fabricated data: the sidebar renders its section-
    // specific "none available" copy for these until their backends exist.
    devices: async () => [],
    topics: async () => [],
  })
}
