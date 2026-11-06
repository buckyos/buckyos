/* ── Workflow store context ── */

import { createContext, useContext, useSyncExternalStore } from 'react'
import { WorkflowStore } from '../api/store'

export const WorkflowStoreContext = createContext<WorkflowStore | null>(null)

export function useWorkflowStore(): WorkflowStore {
  const store = useContext(WorkflowStoreContext)
  if (!store) throw new Error('useWorkflowStore must be used inside WorkflowStoreContext')
  useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot)
  return store
}
