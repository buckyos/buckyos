import { createContext, useCallback, useContext, useEffect, useState, useSyncExternalStore, type ReactNode } from 'react'
import { SW_UPDATE_EVENT, serviceWorkerState, type ServiceWorkerState } from '../../../serviceWorker'
import { describeError, type SessionStatus } from '../api/session'
import type { CapturedAnchor } from '../anchors/registry'
import type { AnchorInfo, AnnotationPayload, AnnotationRead, EntityEnvelope, FreshnessInfo, Json } from '../api/types'
import type { EditEntry } from './edits'
import type { WorkspaceStore } from './store'

export interface AnnotationMark { entityId: string; payload: AnnotationPayload; anchor: AnchorInfo; envelope: AnnotationRead }

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

export interface WriteAccess {
  /** The entity may be edited right now (capability present and, if required, the lock is held). */
  editable: boolean
  held: boolean
  required: boolean
}

/** Can the entity be written now: capability, write lock (design §2.11) and the direct-mode read-only state. */
export function useWriteAccess(entity: EntityEnvelope | undefined, capability: 'update' | 'append' = 'update'): WriteAccess {
  const store = useStore()
  useLocksVersion()
  const readOnlyNow = useDirectReadOnly()
  if (!entity) return { editable: false, held: false, required: false }
  const capable = entity.capabilities.includes(capability) && !readOnlyNow
  const required = entity.write_policy === 'lock_required'
  const held = store.locks.isHeld(entity.entity_id)
  return { editable: capable && (!required || held), held, required }
}

/** Whole-tree subscription: re-renders on every outline change (trees, navigation). */
export function useOutlineVersion(): number {
  const { outline } = useStore()
  return useSyncExternalStore(outline.subscribe, outline.snapshot)
}

/** One entity's envelope, re-rendering only when that entity changes (a Block's own structural entry). */
export function useEntity(id: string): EntityEnvelope | undefined {
  const { outline } = useStore()
  const subscribe = useCallback((listener: () => void) => outline.subscribeEntity(id, listener), [outline, id])
  return useSyncExternalStore(subscribe, () => outline.get(id))
}

/** Freshness of one entity as the core computed it (`doc.freshness`), kept current while mounted. */
export function useFreshness(id: string | null): FreshnessInfo | undefined {
  const { freshness } = useStore()
  const subscribe = useCallback((listener: () => void) => (id ? freshness.watch(id, listener) : () => undefined), [freshness, id])
  return useSyncExternalStore(subscribe, () => (id ? freshness.get(id) : undefined))
}

/** One entry of the user work state (phase two §4.4). */
export function useUserState<T extends Json = Json>(key: string): T | undefined {
  const { userState } = useStore()
  const subscribe = useCallback((listener: () => void) => userState.subscribeKey(key, listener), [userState, key])
  return useSyncExternalStore(subscribe, () => userState.get<T>(key))
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
  /** Annotations in scope (the active Surface's content, or the selected data): anchored to what is shown, or placed there. */
  annotations: AnnotationMark[]
  /** Show an entity: its detail in the data-source view, or its Block on the canvas. */
  openEntity: (entityId: string) => void
  /** Start writing an annotation; null without the `comment` capability (or in a mode that refuses it). */
  annotate: ((anchor: CapturedAnchor) => void) | null
  activeAnnotation: string | null
  /** Select an annotation: its place in the content and its entry in the panel light up. */
  setActiveAnnotation: (entityId: string | null) => void
  /** Render an embedded Cell (rich text `object_embed`) read-only at `depth`. */
  renderCell: (cellId: string, depth: number) => ReactNode
  /** The annotations panel shows only those targeting this entity (a note's replies), or all of them. */
  annotationFilter: string | null
  showAnnotations: (targetId: string | null) => void
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
