/**
 * Wires the mock data sources into the reader/target registries. The UI
 * imports this once; replacing mocks with the real backend swaps this file's
 * registrations only.
 */

import { registerMockReaders } from './mockReader'
import { registerCollectionReader } from '../mock/collections'

let installed = false

export function installFileBrowserData() {
  if (installed) return
  installed = true
  registerMockReaders()
  registerCollectionReader()
}
