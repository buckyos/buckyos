import {
  createContext,
  useCallback,
  useContext,
  useSyncExternalStore,
} from 'react'
import type {
  AICCMgr,
  AIStatus,
  LocalModel,
  ProviderView,
  RouteTrace,
  GlobalRoutingView,
  UsageEvent,
  UsageSummary,
  UsageTrendPoint,
} from '../../../api/aicc_mgr'

export const AICCStoreContext = createContext<AICCMgr>(null!)

export function useAICCStore(): AICCMgr {
  return useContext(AICCStoreContext)
}

function useStoreSnapshot() {
  const store = useAICCStore()
  return useSyncExternalStore(store.subscribe, store.getSnapshot)
}

export function useAIStatus(): AIStatus {
  return useStoreSnapshot().aiStatus
}

export function useProviders(): ProviderView[] {
  return useStoreSnapshot().providers
}

export function useProvider(id: string): ProviderView | undefined {
  const snapshot = useStoreSnapshot()
  return snapshot.providers.find((p) => p.config.id === id)
}

export function useUsageSummary(): UsageSummary {
  const store = useAICCStore()
  const subscribe = store.subscribe
  const getSummary = useCallback(() => store.getUsageSummary(), [store])
  return useSyncExternalStore(subscribe, getSummary)
}

export function useUsageTrend(granularity = 'day'): UsageTrendPoint[] {
  const store = useAICCStore()
  const subscribe = store.subscribe
  const getTrend = useCallback(() => store.getUsageTrend(granularity), [store, granularity])
  return useSyncExternalStore(subscribe, getTrend)
}

export function useUsageEvents(): UsageEvent[] {
  return useStoreSnapshot().usageEvents
}

export function useLocalModels(): LocalModel[] {
  return useStoreSnapshot().localModels
}

export function useGlobalRoutingView(): GlobalRoutingView {
  return useStoreSnapshot().routingView
}

export function useRouteTraces(): RouteTrace[] {
  return useStoreSnapshot().routeTraces
}
