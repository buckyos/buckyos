import { createContext, useContext, useRef, useSyncExternalStore } from 'react'
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

/**
 * Subscribe to a slice of the store. The component re-renders only when `isEqual`
 * (default `Object.is`) says the selected value changed — use this in per-block views so a
 * camera move or a drag of another block does not re-render every block on the sheet.
 */
export function useStoreSelector<T>(selector: (s: StoreState) => T, isEqual: (a: T, b: T) => boolean = Object.is): T {
  const { store } = useCanvasEditor()
  const cache = useRef<{ state: StoreState; selector: (s: StoreState) => T; value: T } | null>(null)
  const getSnapshot = () => {
    const state = store.getState()
    const c = cache.current
    if (c && c.state === state && c.selector === selector) return c.value
    const value = selector(state)
    // keep the previous reference when equal so useSyncExternalStore sees a stable snapshot
    const kept = c && isEqual(c.value, value) ? c.value : value
    cache.current = { state, selector, value: kept }
    return kept
  }
  return useSyncExternalStore(store.subscribe, getSnapshot, getSnapshot)
}

export function shallowEqualArray<T>(a: readonly T[], b: readonly T[]): boolean {
  if (a === b) return true
  if (a.length !== b.length) return false
  for (let i = 0; i < a.length; i++) if (!Object.is(a[i], b[i])) return false
  return true
}
