import {
  createContext,
  useCallback,
  useContext,
  useSyncExternalStore,
} from 'react'
import { MockDataStore } from '../mock/store'
import type {
  AIStatus,
  LocalModel,
  LogicalModelConfig,
  ProviderView,
  UsageSummary,
} from '../mock/types'

export const MockStoreContext = createContext<MockDataStore>(null!)

export function useMockStore(): MockDataStore {
  return useContext(MockStoreContext)
}

function useStoreSnapshot() {
  const store = useMockStore()
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
  const store = useMockStore()
  const subscribe = store.subscribe
  const getSummary = useCallback(() => store.getUsageSummary(), [store])
  return useSyncExternalStore(subscribe, getSummary)
}

export function useLocalModels(): LocalModel[] {
  return useStoreSnapshot().localModels
}

export function useLogicalModels(): LogicalModelConfig[] {
  return useStoreSnapshot().logicalModels
}
