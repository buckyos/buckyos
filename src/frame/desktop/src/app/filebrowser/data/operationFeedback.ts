import { useSyncExternalStore } from 'react'
import type { ConflictChoice, OperationConflict, OperationResult } from './folderOps'

export interface ConflictRequest extends OperationConflict {
  ownerId?: string
  resolve: (choice: ConflictChoice, apply: boolean) => void
}
export interface BatchTask {
  ownerId?: string
  title: string
  total: number
  results: OperationResult[]
  running: boolean
  cancel: () => void
  retry: () => void
}
let snapshot: { batchTask: BatchTask | null; conflictRequest: ConflictRequest | null } = { batchTask: null, conflictRequest: null }
const pendingConflicts: ConflictRequest[] = []
const listeners = new Set<() => void>()
const subscribe = (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener) } }
const getSnapshot = () => snapshot
const emit = () => listeners.forEach((listener) => listener())
export const operationFeedback = {
  isRunning: () => !!snapshot.batchTask?.running,
  setBatchTask(task: BatchTask | null | ((previous: BatchTask | null) => BatchTask | null)) {
    snapshot = { ...snapshot, batchTask: typeof task === 'function' ? task(snapshot.batchTask) : task }
    emit()
  },
  requestConflict(conflict: OperationConflict, ownerId?: string): Promise<{ choice: ConflictChoice; apply: boolean }> {
    return new Promise((resolve) => {
      const request: ConflictRequest = { ...conflict, ownerId, resolve: (choice, apply) => {
        snapshot = { ...snapshot, conflictRequest: pendingConflicts.shift() ?? null }
        emit()
        resolve({ choice, apply })
      } }
      if (snapshot.conflictRequest) pendingConflicts.push(request)
      else { snapshot = { ...snapshot, conflictRequest: request }; emit() }
    })
  },
}
export function useOperationFeedback() { return useSyncExternalStore(subscribe, getSnapshot) }
