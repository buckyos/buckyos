/* ── TaskCenter – app panel entry point ── */

import { useEffect, useState } from 'react'
import { TaskCenterStoreContext } from './hooks/use-task-center-store'
import { createTaskCenterModel } from '../../api/task_mgr'
import { TaskCenterShell } from './components/layout/TaskCenterShell'

export function TaskCenterAppPanel() {
  const [store] = useState(() => createTaskCenterModel())
  useEffect(() => () => store.dispose(), [store])

  return (
    <TaskCenterStoreContext.Provider value={store}>
      <TaskCenterShell />
    </TaskCenterStoreContext.Provider>
  )
}
