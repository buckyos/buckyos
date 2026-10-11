/* ── Shared presentation cache ──
 *
 * One cache per (Zone, viewer): a profile visible to one account must never show up for
 * another, so switching either drops every entry. Concurrent requests for a key share one
 * load; a failed load is retried after FAILED_RETRY_MS or when forced.
 */

import { useEffect, useSyncExternalStore } from 'react'
import { buckyos, namelib } from 'buckyos'
import { isMockRuntime } from '../runtime'
import { presentDidDocument, presentUnresolvedDid, type ObjectPresentation, type PresentationKind } from './model'

export const FAILED_RETRY_MS = 30_000

type Loader = () => Promise<ObjectPresentation>

interface Entry {
  value: ObjectPresentation
  at: number
}

export class PresentationStore {
  private scope = ''
  private entries = new Map<string, Entry>()
  private pending = new Map<string, Promise<ObjectPresentation | undefined>>()
  private listeners = new Set<() => void>()
  private generation = 0
  private readonly now: () => number

  constructor(now: () => number = Date.now) {
    this.now = now
  }

  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => { this.listeners.delete(listener) }
  }

  get = (key: string): ObjectPresentation | undefined => this.entries.get(key)?.value

  getScope = () => this.scope

  setScope(scope: string) {
    if (scope === this.scope) return
    this.scope = scope
    this.generation += 1
    this.entries.clear()
    this.pending.clear()
    this.notify()
  }

  /** Data the caller already holds (a list response, a save result) replaces the cached entry. */
  put(...items: ObjectPresentation[]) {
    const at = this.now()
    for (const item of items) this.entries.set(item.key, { value: item, at })
    if (items.length > 0) this.notify()
  }

  invalidate(key: string) {
    this.entries.delete(key)
    this.notify()
  }

  ensure(key: string, loader: Loader, options: { force?: boolean } = {}): Promise<ObjectPresentation | undefined> {
    const cached = this.entries.get(key)
    const stale = !cached || (cached.value.status === 'failed' && this.now() - cached.at >= FAILED_RETRY_MS)
    if (!options.force && !stale) return Promise.resolve(cached?.value)
    const inflight = this.pending.get(key)
    if (inflight) return inflight
    const generation = this.generation
    const promise = loader()
      .then((value) => {
        if (generation !== this.generation) return undefined
        this.entries.set(key, { value, at: this.now() })
        return value
      })
      .catch(() => undefined)
      .finally(() => {
        if (this.pending.get(key) === promise) this.pending.delete(key)
        if (generation === this.generation) this.notify()
      })
    this.pending.set(key, promise)
    return promise
  }

  private notify() {
    this.listeners.forEach((listener) => listener())
  }
}

export const presentationStore = new PresentationStore()

/** The cached presentation for `key`; `load` runs when it is missing or a failure is due for retry. */
export function usePresentation(key: string | undefined, load?: Loader): ObjectPresentation | undefined {
  const value = useSyncExternalStore(
    presentationStore.subscribe,
    () => (key ? presentationStore.get(key) : undefined),
  )
  const needsLoad = Boolean(key && load) && (!value || value.status === 'failed')
  useEffect(() => {
    if (!key || !load || !needsLoad) return
    void presentationStore.ensure(key, load)
  }, [key, load, needsLoad])
  return value
}

/** `/1.0/identifiers/<did>` (the Zone's system-config resolver) answers both bare and W3C-envelope documents. */
export async function fetchDidDocument(did: string, fetcher: typeof fetch = fetch): Promise<unknown> {
  const response = await fetcher(`/1.0/identifiers/${encodeURIComponent(did)}`, {
    headers: { accept: 'application/json' },
  })
  if (!response.ok) throw new Error(`resolve ${did}: HTTP ${response.status}`)
  const body = await response.json() as Record<string, unknown>
  const document = body && typeof body === 'object' && 'didDocument' in body ? body.didDocument : body
  if (typeof document === 'string') return namelib.decodeJwtClaimWithoutVerify(document)
  if (!document || typeof document !== 'object') throw new Error(`resolve ${did}: no document`)
  return document
}

/** Resolves any DID to its presentation through its document; a failure yields the DID's readable name. */
export function didPresentationLoader(did: string, kind: PresentationKind, locale: string): Loader {
  return async () => {
    if (isMockRuntime()) return presentUnresolvedDid(did, kind, 'failed')
    try {
      const bridged = await buckyos.resolve_did(did).catch(() => null)
      const document = bridged ? namelib.encodedDocumentToJsonValue(bridged) : await fetchDidDocument(did)
      return presentDidDocument(did, document, locale)
    } catch {
      return presentUnresolvedDid(did, kind, 'failed')
    }
  }
}
