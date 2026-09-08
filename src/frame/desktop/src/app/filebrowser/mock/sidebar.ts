/**
 * Mock sidebar source fetchers (UI_DATAMODEL.md §4.3, §7.2).
 *
 * Deterministic failure scenario: `?fbFail=<source>[,<source>]` (dfs, devices,
 * topics) makes that source's FIRST load fail; retry succeeds — exercising the
 * per-section inline error without timing dependence.
 */

import { registerSidebarSources } from '../data/sidebarSources'
import { mockDelay } from '../data/mockReader'
import { fileBrowserSnapshot } from './data'

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
    devices: () => load('devices', fileBrowserSnapshot.devices),
    topics: () => load('topics', fileBrowserSnapshot.topics),
  })
}
