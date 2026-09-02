/**
 * Local write→read invalidation bus for the NFSP adapter.
 *
 * NfsBrowserClient invalidates its *cache* on direct writes but only fires
 * listeners from watch events / revalidation. Same-client correctness (§8.4:
 * "watch is not required for same-client correctness") therefore needs this
 * small bus: folder ops notify the parent path they changed, and readers on
 * that path reload immediately instead of waiting for the SSE round-trip.
 */

const listenersByPath = new Map<string, Set<() => void>>()

export function watchDfsPath(path: string, listener: () => void): () => void {
  let set = listenersByPath.get(path)
  if (!set) {
    set = new Set()
    listenersByPath.set(path, set)
  }
  set.add(listener)
  return () => {
    set.delete(listener)
    if (set.size === 0) listenersByPath.delete(path)
  }
}

export function notifyDfsPath(path: string) {
  for (const listener of listenersByPath.get(path) ?? []) listener()
}
