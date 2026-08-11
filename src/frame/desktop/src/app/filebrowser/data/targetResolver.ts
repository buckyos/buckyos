/**
 * Reference-target resolver registry. A collection member (or a folder link
 * item) stores only a canonical target URL; resolving it to a FileEntry goes
 * through here, keyed by scheme. `dfs://` registers today; share/remote
 * targets later register new schemes without touching the collection model.
 */

import type { FileEntry } from '../types'
import { urlScheme } from './urls'

export type TargetResolver = (targetUrl: string) => Promise<FileEntry | null>

const resolvers = new Map<string, TargetResolver>()

export function registerTargetResolver(scheme: string, resolver: TargetResolver): () => void {
  resolvers.set(scheme, resolver)
  return () => {
    if (resolvers.get(scheme) === resolver) resolvers.delete(scheme)
  }
}

/** Resolve a target URL; null = dangling (deleted target or unknown scheme). */
export async function resolveTarget(targetUrl: string): Promise<FileEntry | null> {
  const scheme = urlScheme(targetUrl)
  const resolver = scheme ? resolvers.get(scheme) : null
  if (!resolver) return null
  try {
    return await resolver(targetUrl)
  } catch {
    return null
  }
}
