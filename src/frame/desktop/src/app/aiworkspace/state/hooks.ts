import { createContext, useContext, useEffect, useState, useSyncExternalStore } from 'react'
import { SW_UPDATE_EVENT, serviceWorkerState, type ServiceWorkerState } from '../../../serviceWorker'
import { describeError, type SessionStatus } from '../api/session'
import type { AnnotationPayload, EntityEnvelope, Reference } from '../api/types'
import type { EditEntry } from './edits'
import type { WorkspaceStore } from './store'

export interface AnnotationMark { entityId: string; payload: AnnotationPayload; anchorState: string }

export const StoreContext = createContext<WorkspaceStore | null>(null)

export function useStore(): WorkspaceStore {
  const store = useContext(StoreContext)
  if (!store) throw new Error('aiworkspace: no open workspace in context')
  return store
}

/** Invalidation counter: `e:<entity_id>`, `outline`, or `any`. */
export function useVersion(key: string): number {
  const { versions } = useStore()
  return useSyncExternalStore(versions.subscribe, () => versions.get(key))
}

export function useEdit(id: string): EditEntry | undefined {
  const { edits } = useStore()
  return useSyncExternalStore(edits.subscribe, () => edits.get(id))
}

export function useEdits(): ReadonlyMap<string, EditEntry> {
  const { edits } = useStore()
  return useSyncExternalStore(edits.subscribe, edits.snapshot)
}

export function useSessionStatus(): SessionStatus {
  const { session } = useStore()
  return useSyncExternalStore((listener) => session.subscribeStatus(listener), () => session.status())
}

/** Online direct mode of a window that could not become the replica holder: read-only while the backend is unreachable (design §6.1). */
export function useDirectReadOnly(): boolean {
  const { session } = useStore()
  const status = useSessionStatus()
  const mode = session.mode()
  return mode.kind === 'direct' && mode.reason !== 'not_prepared' && status.kind === 'offline'
}

export function useLocksVersion(): number {
  const { locks } = useStore()
  return useSyncExternalStore(locks.subscribe, locks.snapshot)
}

export interface Loaded<T> { data: T | undefined; error: string | null; loading: boolean; reload: () => void }

/** Run `load` (memoise it with useCallback) on mount and whenever `version` moves; keeps the previous
 * data while reloading so views do not flicker. */
export function useLoad<T>(load: () => Promise<T>, version: number | string = 0): Loaded<T> {
  const [state, setState] = useState<{ data: T | undefined; error: string | null; settled: unknown }>({ data: undefined, error: null, settled: null })
  const [tick, setTick] = useState(0)
  useEffect(() => {
    let live = true
    const token = {}
    load().then(
      (data) => { if (live) setState({ data, error: null, settled: token }) },
      (error: unknown) => { if (live) setState((previous) => ({ data: previous.data, error: describeError(error), settled: token })) },
    )
    return () => { live = false }
  }, [load, version, tick])
  return { data: state.data, error: state.error, loading: state.settled === null, reload: () => setTick((value) => value + 1) }
}

/** What the views of one open workspace share besides the store: the outline and a few navigation actions. */
export interface WorkspaceUi {
  entities: EntityEnvelope[]
  byId: ReadonlyMap<string, EntityEnvelope>
  annotations: AnnotationMark[]
  openEntity: (entityId: string) => void
  annotate: (target: Reference, label: string) => void
}

export const WorkspaceUiContext = createContext<WorkspaceUi | null>(null)

export function useWorkspaceUi(): WorkspaceUi {
  const ui = useContext(WorkspaceUiContext)
  if (!ui) throw new Error('aiworkspace: no workspace view in context')
  return ui
}

function subscribeServiceWorker(listener: () => void) {
  window.addEventListener(SW_UPDATE_EVENT, listener)
  return () => window.removeEventListener(SW_UPDATE_EVENT, listener)
}

let lastServiceWorkerState: ServiceWorkerState = serviceWorkerState()
function serviceWorkerSnapshot(): ServiceWorkerState {
  const next = serviceWorkerState()
  if (next.unavailable !== lastServiceWorkerState.unavailable || next.updateWaiting !== lastServiceWorkerState.updateWaiting) lastServiceWorkerState = next
  return lastServiceWorkerState
}

/** Whether the application itself (HTML, scripts, WASM) is cached for a cold start without the network. */
export function useAppCache(): ServiceWorkerState {
  return useSyncExternalStore(subscribeServiceWorker, serviceWorkerSnapshot)
}
