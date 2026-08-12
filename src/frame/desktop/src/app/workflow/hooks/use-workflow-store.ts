/* ── Workflow store context ── */

import { createContext, useContext } from 'react'
import { WorkflowMockStore } from '../mock/store'

export const WorkflowStoreContext = createContext<WorkflowMockStore | null>(null)

export function useWorkflowStore(): WorkflowMockStore {
  const store = useContext(WorkflowStoreContext)
  if (!store) throw new Error('useWorkflowStore must be used inside WorkflowStoreContext')
  return store
}
