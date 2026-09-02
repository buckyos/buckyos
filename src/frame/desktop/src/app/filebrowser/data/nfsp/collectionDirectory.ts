/**
 * NFSP collection directory. NFSP v1 has no list-collections method (the
 * server resolves `collection://<id>` but never enumerates), so the directory
 * keeps the set of known collection ids in localStorage per server and
 * validates each id against the server on refresh — ids that stopped
 * resolving are dropped, titles/counts always come from the server. This is
 * a documented client-side stopgap until the protocol grows enumeration.
 */

import type { Entry } from '../../../../api/nfsp_client'
import type { CollectionSummary } from '../../types'
import { registerCollectionDirectory } from '../collectionDirectory'
import { ensureSession, nfspBaseUrl, nfspClient } from './client'
import { nfspToUiError } from './errors'

const STORAGE_KEY_PREFIX = 'nfsp:fb:collections:'
const COUNT_DEPTH_LIMIT = 4

function storageKey(): string {
  let hash = 0x811c9dc5
  const base = nfspBaseUrl()
  for (let i = 0; i < base.length; i++) {
    hash ^= base.charCodeAt(i)
    hash = Math.imul(hash, 0x01000193)
  }
  return `${STORAGE_KEY_PREFIX}${(hash >>> 0).toString(36)}`
}

function loadKnownIds(): string[] {
  try {
    const raw = localStorage.getItem(storageKey())
    const parsed: unknown = raw ? JSON.parse(raw) : []
    return Array.isArray(parsed) ? parsed.filter((id): id is string => typeof id === 'string') : []
  } catch {
    return []
  }
}

function saveKnownIds(ids: string[]) {
  try {
    localStorage.setItem(storageKey(), JSON.stringify(ids))
  } catch {
    // storage unavailable: the directory degrades to session-local memory
  }
}

let knownIds: string[] = []
let summaries: CollectionSummary[] = []
let detailById = new Map<string, { title: string; description?: string }>()
let snapshotToken = 0
let refreshing = false
let refreshQueued = false
const listeners = new Set<() => void>()

function emit() {
  snapshotToken += 1
  for (const listener of listeners) listener()
}

/** Recursive reference count: ref entries count, groups don't (§2.4). */
async function countRefs(entries: Entry[], depth: number): Promise<number> {
  let count = 0
  for (const entry of entries) {
    if (entry.target.kind === 'group') {
      if (depth >= COUNT_DEPTH_LIMIT) continue
      const child = await nfspClient().list(entry.target.ref, { limit: 200 })
      if (child) count += await countRefs(child.entries, depth + 1)
    } else {
      count += 1
    }
  }
  return count
}

async function refreshNow(): Promise<void> {
  await ensureSession()
  const client = nfspClient()
  const nextSummaries: CollectionSummary[] = []
  const nextDetail = new Map<string, { title: string; description?: string }>()
  const liveIds: string[] = []
  for (const id of knownIds) {
    try {
      const info = await client.raw.openCollection(id, ['base'])
      const listing = await client.list({ uri: `collection://${id}` }, { limit: 200 })
      const refCount = listing ? await countRefs(listing.entries, 0) : 0
      const title = info.title ?? id
      liveIds.push(id)
      nextDetail.set(id, { title })
      nextSummaries.push({ id, title, refCount })
    } catch (err) {
      const ui = nfspToUiError(err)
      if (ui.code === 'NOT_FOUND' || ui.code === 'STALE') continue // dropped for good
      // Transient failure: keep the id, show the last known title.
      liveIds.push(id)
      const last = detailById.get(id)
      if (last) {
        nextDetail.set(id, last)
        nextSummaries.push({ id, title: last.title, refCount: 0 })
      }
    }
  }
  knownIds = liveIds
  summaries = nextSummaries
  detailById = nextDetail
  saveKnownIds(liveIds)
  emit()
}

function scheduleRefresh() {
  if (refreshing) {
    refreshQueued = true
    return
  }
  refreshing = true
  void refreshNow()
    .catch(() => {
      // Session-level failure: leave the last projection standing (§4.3).
    })
    .finally(() => {
      refreshing = false
      if (refreshQueued) {
        refreshQueued = false
        scheduleRefresh()
      }
    })
}

/** Membership mutations call this so sidebar counts follow (§4.6 success). */
export function refreshNfspCollectionDirectory() {
  scheduleRefresh()
}

export function registerNfspCollectionDirectory() {
  knownIds = loadKnownIds()
  scheduleRefresh()
  return registerCollectionDirectory({
    subscribe(listener) {
      listeners.add(listener)
      return () => listeners.delete(listener)
    },
    snapshot: () => snapshotToken,
    list: () => summaries,
    get: (id) => detailById.get(id),
    async create(title) {
      await ensureSession()
      try {
        const info = await nfspClient().raw.createCollection(title, undefined, ['base'])
        const id = info.collection_id
        if (!id) throw nfspToUiError({ code: 'INTERNAL', message: 'no collection id' })
        if (!knownIds.includes(id)) {
          knownIds = [...knownIds, id]
          saveKnownIds(knownIds)
        }
        detailById.set(id, { title: info.title ?? title })
        summaries = [...summaries, { id, title: info.title ?? title, refCount: 0 }]
        emit()
        scheduleRefresh()
        return id
      } catch (err) {
        const ui = nfspToUiError(err)
        throw new Error(ui.fallback)
      }
    },
  })
}
