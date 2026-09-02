/**
 * Wires the mock data sources into the reader/search/sidebar/transfer
 * registries. The UI imports this once; replacing mocks with the real backend
 * (the NFSP adapter stage) swaps this file's registrations only.
 */

import { registerMockReaders } from './mockReader'
import { registerCollectionReader } from '../mock/collections'
import { registerMockSearchProvider } from '../mock/search'
import { registerMockSidebarSources } from '../mock/sidebar'
import { registerMockTransferExecutor } from '../mock/transfers'

let installed = false

export function installFileBrowserData() {
  if (installed) return
  installed = true
  registerMockReaders()
  registerCollectionReader()
  registerMockSearchProvider()
  registerMockSidebarSources()
  registerMockTransferExecutor()
}
