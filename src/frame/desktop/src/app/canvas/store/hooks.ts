import { createContext, useContext, useSyncExternalStore } from 'react'
import type { CanvasStore, StoreState } from './canvas-store'
import type { WishRunner } from '../agent/runner'

export interface CanvasEditorContextValue {
  store: CanvasStore
  runner: WishRunner
}

export const CanvasEditorContext = createContext<CanvasEditorContextValue | null>(null)

export function useCanvasEditor(): CanvasEditorContextValue {
  const ctx = useContext(CanvasEditorContext)
  if (!ctx) throw new Error('CanvasEditorContext missing')
  return ctx
}

export function useStoreState(): StoreState {
  const { store } = useCanvasEditor()
  return useSyncExternalStore(store.subscribe, store.getState, store.getState)
}
