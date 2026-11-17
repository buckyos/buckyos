/**
 * Mock sidebar source fetchers (UI_DATAMODEL.md §4.3, §7.2).
 *
 * Deterministic failure scenario: `?fbFail=<source>[,<source>]` (dfs, host,
 * topics, children) makes that source's FIRST load fail; retry succeeds — exercising the
 * per-section inline error without timing dependence.
 */

import { registerSidebarSources } from '../data/sidebarSources'
import { mockDelay } from '../data/mockReader'
import { dfsPathOf, normalizeUrl } from '../data/urls'
import { fileBrowserSnapshot, mockEntriesAtPath } from './data'
import { mockHostInfo } from './host'

const failedOnce = new Set<string>()
const pending = new Map<string, Promise<unknown>>()

function failRequested(source: string): boolean {
  try {
    const params = new URLSearchParams(window.location.search)
    return (params.get('fbFail') ?? '').split(',').includes(source)
  } catch {
    return false
  }
}

function load<T>(source: string, data: T): Promise<T> {
  const existing = pending.get(source)
  if (existing) return existing as Promise<T>
  const attempt = mockDelay(60, 140).then(() => {
    if (failRequested(source) && !failedOnce.has(source)) {
      failedOnce.add(source)
      throw new Error(`Mock ${source} source failure (?fbFail=${source}) — retry succeeds`)
    }
    return data
  }).finally(() => pending.delete(source))
  pending.set(source, attempt)
  return attempt
}

export function registerMockSidebarSources() {
  return registerSidebarSources({
    dfs: () => load('dfs', fileBrowserSnapshot.dfsRoots),
    host: () => load('host', mockHostInfo()),
    topics: () => load('topics', fileBrowserSnapshot.topics),
    children: async (url) => {
      await mockDelay(40, 90)
      if (failRequested('children') && !failedOnce.has(`children:${url}`)) {
        failedOnce.add(`children:${url}`)
        throw new Error('Mock folder failure (?fbFail=children) — retry succeeds')
      }
      const path = dfsPathOf(url)
      const entries = path === null ? undefined : mockEntriesAtPath(path)
      if (!entries) throw new Error('This folder no longer exists')
      return {
        folders: entries
          .filter((entry) => entry.kind === 'folder')
          .sort((a, b) => a.name.localeCompare(b.name))
          .map((entry) => ({ name: entry.name, url: normalizeUrl(entry.path) })),
      }
    },
  })
}
