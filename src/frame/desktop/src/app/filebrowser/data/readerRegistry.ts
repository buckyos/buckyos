/**
 * URL-scheme → FolderReader registry (same data-driven pattern as the
 * context-menu registry). Plugging in a real backend = registering a new
 * provider; the UI keeps resolving through `resolveReader()`.
 */

import type { FolderReader } from './FolderReader'
import { normalizeUrl, urlScheme } from './urls'

export interface ReaderProvider {
  scheme: string // 'dfs' | 'view' | 'collection' | ...
  create(url: string): FolderReader
}

const providers = new Map<string, ReaderProvider>()

/** Register a provider; returns a disposer that unregisters it. */
export function registerReaderProvider(provider: ReaderProvider): () => void {
  providers.set(provider.scheme, provider)
  return () => {
    if (providers.get(provider.scheme) === provider) providers.delete(provider.scheme)
  }
}

export function resolveReader(url: string): FolderReader {
  const canonical = normalizeUrl(url)
  const scheme = urlScheme(canonical)
  const provider = scheme ? providers.get(scheme) : null
  if (!provider) {
    throw new Error(`filebrowser: no reader registered for "${canonical}"`)
  }
  return provider.create(canonical)
}
