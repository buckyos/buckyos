/**
 * Preview provider registry — the single point where the Preview Component
 * picks its system backend (mirrors `filebrowser/data/install.ts`):
 *
 *   - mock runtime (`VITE_CP_USE_MOCK`, dev/e2e): generated fixtures + a
 *     simulated built-in Pipeline
 *   - buckyos runtime: NFSP over nfs_server (Source Resolver, container
 *     listing, and the `repr` Pipeline when the server advertises it)
 *
 * `?fbData=mock|nfsp` overrides per session so File Browser and Preview always
 * agree on the backend they talk to.
 */

import { isMockRuntime } from '../../runtime'
import type { PreviewProvider } from './types'

let provider: PreviewProvider | null = null

export function registerPreviewProvider(next: PreviewProvider): () => void {
  const previous = provider
  provider = next
  return () => {
    if (provider === next) provider = previous
  }
}

export function hasPreviewProvider(): boolean {
  return provider !== null
}

function dataModeOverride(): 'mock' | 'nfsp' | null {
  try {
    const raw = new URLSearchParams(window.location.search).get('fbData')
    return raw === 'mock' || raw === 'nfsp' ? raw : null
  } catch {
    return null
  }
}

let installing: Promise<PreviewProvider> | null = null

/** Lazily installs the runtime's provider (idempotent, concurrent-safe). */
export function ensurePreviewProvider(): Promise<PreviewProvider> {
  if (provider) return Promise.resolve(provider)
  if (installing) return installing
  const mode = dataModeOverride() ?? (isMockRuntime() ? 'mock' : 'nfsp')
  installing = (mode === 'mock'
    ? import('./mockProvider').then((m) => m.createMockPreviewProvider())
    : import('./nfspProvider').then((m) => m.createNfspPreviewProvider())
  ).then((created) => {
    if (!provider) provider = created
    installing = null
    return provider
  })
  return installing
}
