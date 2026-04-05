/* ── TaskCenter store context ── */

import { createContext, useContext } from 'react'
import { TaskCenterMockStore } from '../mock/store'

export const TaskCenterStoreContext = createContext<TaskCenterMockStore | null>(null)

export function useTaskCenterStore(): TaskCenterMockStore {
  const store = useContext(TaskCenterStoreContext)
  if (!store) throw new Error('useTaskCenterStore must be used inside TaskCenterStoreContext')
  return store
}
