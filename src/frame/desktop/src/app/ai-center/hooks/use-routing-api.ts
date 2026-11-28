import { useCallback, useSyncExternalStore } from 'react'
import useSWR, { useSWRConfig } from 'swr'
import { createRoutingApi, type RoutingApi } from '../../../api/aicc_routing'
import type { AICCMgr } from '../../../api/aicc_mgr'
import { useAICCStore } from './use-aicc-store'

const apis = new WeakMap<AICCMgr, RoutingApi>()

/** One routing API per AICC store, so every page shares the same SWR keys. */
export function useRoutingApi(): RoutingApi {
  const store = useAICCStore()
  let api = apis.get(store)
  if (!api) {
    api = createRoutingApi(store)
    apis.set(store, api)
  }
  return api
}

/** The usecase table of one view; reloads when Providers change. */
export function useUsecaseTable(view: string | null) {
  const api = useRoutingApi()
  const store = useAICCStore()
  const version = useSyncExternalStore(store.subscribe, store.getSnapshotVersion)
  return useSWR(view ? ['aicc-usecases', api, view, version] : null, () => api.usecases({ view: view ?? 'self' }), { keepPreviousData: true })
}

const AFFECTED_BY_A_LAYER = new Set(['aicc-usecases', 'aicc-provider-impact', 'aicc-routing-groups'])

/** A layer changed: every view of the usecase table, a Provider's impact preview and the groups' versions may differ. */
export function useInvalidateUsecases() {
  const { mutate } = useSWRConfig()
  return useCallback(() => mutate((key) => Array.isArray(key) && AFFECTED_BY_A_LAYER.has(key[0])), [mutate])
}

/** The viewer's routing groups and App bindings. */
export function useRoutingGroups() {
  const api = useRoutingApi()
  return useSWR(['aicc-routing-groups', api], async () => {
    const [groups, bindings] = await Promise.all([api.listGroups(), api.listBindings()])
    return { groups, bindings }
  }, { keepPreviousData: true })
}

/** Apps and Agents a group can be bound to; empty when the list cannot be loaded. */
export function useRoutingApps() {
  const api = useRoutingApi()
  return useSWR(['aicc-routing-apps', api], () => api.listApps().catch(() => []), { revalidateOnFocus: false })
}
