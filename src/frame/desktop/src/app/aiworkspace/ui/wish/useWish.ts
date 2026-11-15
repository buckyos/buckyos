/* What a wish's card, its records and its properties read (许愿格交互升级 §3.1): the wish's data with its history, its
 * freshness, the run this window follows and the main state the core computes from them. Each piece follows its own
 * changes: the entity's version, the freshness store, the wish service. */

import { useCallback, useEffect, useSyncExternalStore } from 'react'
import type { ReadOk } from '../../api/session'
import type { Json, WishPayloadRead } from '../../api/types'
import { aiwsT } from '../../i18n'
import { useEntity, useFreshness, useLoad, useStore, useVersion } from '../../state/hooks'
import type { WorkspaceStore } from '../../state/store'
import { isActive } from './WishService'

export function useWish(wishId: string) {
  const store = useStore()
  const entity = useEntity(wishId)
  const version = useVersion(`e:${wishId}`)
  const load = useCallback(() => store.readBatched<WishPayloadRead>(wishId), [store, wishId])
  const read = useLoad<ReadOk<WishPayloadRead>>(load, version)
  const current = read.data?.entity_id === wishId ? read.data : undefined
  const freshness = useFreshness(wishId)
  const wishVersion = useSyncExternalStore(store.wish.subscribe, store.wish.snapshot)
  useEffect(() => { if (wishId) void store.wish.restore(wishId) }, [store, wishId])
  const payload = current?.content.payload
  const loadMain = useCallback(() => store.wish.mainStatus(wishId, freshness, payload), [store, wishId, freshness, payload])
  const main = useLoad(loadMain, wishVersion).data ?? null
  const run = store.wish.run(wishId)
  const busy = store.wish.busy.get(wishId)
  const active = isActive(run) ? run : undefined
  const runningStage = active?.stage ?? (busy === 'analyzing' ? 'analyze' : busy === 'coding' ? 'code' : busy === 'running' ? 'run' : undefined)
  return {
    entity, payload, keyRevs: current?.content.key_revs ?? {}, loading: !current && !read.error, readError: current ? null : read.error,
    freshness, main, run, active, busy, runningStage, flowError: store.wish.errors.get(wishId),
  }
}

export type WishState = ReturnType<typeof useWish>

/** Write (or remove, `null`) one key of the wish, expecting the revision it was read at. */
export async function setWishKey(store: WorkspaceStore, wishId: string, keyRevs: Record<string, number>, key: string, value: Json | null, label: string): Promise<void> {
  const op = value === null
    ? { op: 'entity.unset_keys' as const, entity_id: wishId, keys: [{ key, expect: { rev: keyRevs[key] ?? 0 } }] }
    : { op: 'entity.set_keys' as const, entity_id: wishId, keys: [{ key, value, expect: { rev: keyRevs[key] ?? 0 } }] }
  const outcome = await store.submit({ editId: `wish:${wishId}:${key}`, label, operations: [op] })
  if (outcome.status === 'conflict' || outcome.status === 'rejected') throw new Error(aiwsT('aiworkspace.wish.notSaved', { code: outcome.code }))
}
