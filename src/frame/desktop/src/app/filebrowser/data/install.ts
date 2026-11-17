/**
 * Data-source installation — the single point where the File Browser picks
 * its backend (UI_DATAMODEL.md §8.1, §9 item 6):
 *
 *   - mock runtime (`VITE_CP_USE_MOCK`, dev/e2e): in-memory fixtures
 *   - buckyos runtime: the NFSP adapter over nfs-server (`/nfs/v1/*`)
 *
 * `?fbData=mock|nfsp` overrides per-session for diagnostics — e.g.
 * `?fbData=nfsp&fbServer=http://127.0.0.1:3260` runs the adapter against a
 * standalone nfs_server from the mock dev build. The UI imports this once;
 * everything else resolves through the registries.
 */

import { isMockRuntime } from '../../../runtime'
import { registerCollectionReader } from '../mock/collections'
import { registerMockFolderOps } from '../mock/folderOps'
import { registerMockPermissionSource } from '../mock/permissions'
import { registerMockSearchProvider } from '../mock/search'
import { registerMockSidebarSources } from '../mock/sidebar'
import { registerMockTransferExecutor } from '../mock/transfers'
import { mockDelay, registerMockReaders } from './mockReader'
import { installNfspFileBrowserData } from './nfsp/install'
import { registerMyHomeProvider } from './myHome'
import { registerHostReaders } from './hostRealm'
import { registerMockHostBackend } from '../mock/host'
import { registerPreviewEnricher } from './usePreview'

let installed = false
let installedMode: 'mock' | 'nfsp' = 'mock'

/** Which backend the File Browser runs on (mock fixtures or NFSP). */
export function fileBrowserDataMode(): 'mock' | 'nfsp' {
  return installedMode
}

function dataModeOverride(): 'mock' | 'nfsp' | null {
  try {
    const raw = new URLSearchParams(window.location.search).get('fbData')
    return raw === 'mock' || raw === 'nfsp' ? raw : null
  } catch {
    return null
  }
}

function installMockData() {
  registerMockReaders()
  registerCollectionReader()
  registerMockSearchProvider()
  registerMockSidebarSources()
  registerMockTransferExecutor()
  registerMockFolderOps()
  registerMockPermissionSource()
  // The mock fixtures treat `/home` itself as the signed-in user's home.
  registerMyHomeProvider(async () => ({ path: '/home', userId: null }))
  // Preview enrichment: simulated latency over the already-projected entry.
  registerPreviewEnricher(async (item) => {
    await mockDelay(50, 120)
    return item
  })
}

export function installFileBrowserData() {
  if (installed) return
  installed = true
  const mode = dataModeOverride() ?? (isMockRuntime() ? 'mock' : 'nfsp')
  installedMode = mode
  registerHostReaders()
  if (mode === 'mock') {
    installMockData()
    registerMockHostBackend()
  } else installNfspFileBrowserData()
}
