/* ── TaskCenter store context ── */

import { createContext, useContext, useSyncExternalStore } from 'react'
import type { TaskCenterModel } from '../../../api/task_mgr'

export const TaskCenterStoreContext = createContext<TaskCenterModel | null>(null)

export function useTaskCenterStore(): TaskCenterModel {
  const store = useContext(TaskCenterStoreContext)
  if (!store) throw new Error('useTaskCenterStore must be used inside TaskCenterStoreContext')
  useSyncExternalStore(
    (listener) => store.subscribe(listener),
    () => store.getSnapshot(),
    () => store.getSnapshot(),
  )
  return store
}
