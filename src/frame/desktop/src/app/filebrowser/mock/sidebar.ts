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

function failRequested(source: string): boolean {
  try {
    const params = new URLSearchParams(window.location.search)
    return (params.get('fbFail') ?? '').split(',').includes(source)
  } catch {
    return false
  }
}

async function load<T>(source: string, data: T): Promise<T> {
  await mockDelay(60, 140)
  if (failRequested(source) && !failedOnce.has(source)) {
    failedOnce.add(source)
    throw new Error(`Mock ${source} source failure (?fbFail=${source}) — retry succeeds`)
  }
  return data
}

export function registerMockSidebarSources() {
  return registerSidebarSources({
    dfs: () => load('dfs', fileBrowserSnapshot.dfsRoots),
    devices: () => load('devices', fileBrowserSnapshot.devices),
    topics: () => load('topics', fileBrowserSnapshot.topics),
  })
}
