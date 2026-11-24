/* ── TaskCenter – app panel entry point ── */

import { useEffect, useState } from 'react'
import { TaskCenterStoreContext } from './hooks/use-task-center-store'
import { createTaskCenterModel } from '../../api/task_mgr'
import { TaskCenterShell } from './components/layout/TaskCenterShell'
import { taskCenterLaunchSchema } from './launch'
import type { AppContentLoaderProps } from '../types'

export function TaskCenterAppPanel({ launch }: AppContentLoaderProps) {
  const parsed = taskCenterLaunchSchema.safeParse(launch?.payload)
  const [store] = useState(() => createTaskCenterModel())
  useEffect(() => () => store.dispose(), [store])

  return (
    <TaskCenterStoreContext.Provider value={store}>
      <TaskCenterShell key={launch?.requestId ?? 'default'} initialTaskId={parsed.success ? parsed.data.taskId : undefined} />
    </TaskCenterStoreContext.Provider>
  )
}
