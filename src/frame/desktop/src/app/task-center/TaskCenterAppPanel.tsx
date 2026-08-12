/* ── TaskCenter – app panel entry point ── */

import { useState } from 'react'
import type { AppContentLoaderProps } from '../types'
import { TaskCenterStoreContext } from './hooks/use-task-center-store'
import { createTaskCenterModel } from '../../api/task_mgr'
import { TaskCenterShell } from './components/layout/TaskCenterShell'
import type { TaskCenterNav } from './components/layout/navigation'
import { HomePage } from './pages/HomePage'
import { TasksPage } from './pages/TasksPage'
import { ScheduledTasksPage } from './pages/ScheduledTasksPage'
import { TaskDetailPage } from './pages/TaskDetailPage'
import { SystemEventsPage } from './pages/SystemEventsPage'

function PageRouter({
  nav,
  onNavigate,
}: {
  nav: TaskCenterNav
  onNavigate: (nav: TaskCenterNav) => void
}) {
  if (nav.taskId) {
    return <TaskDetailPage taskId={nav.taskId} backPage={nav.page} onNavigate={onNavigate} />
  }

  switch (nav.page) {
    case 'home':
      return <HomePage onNavigate={onNavigate} />
    case 'tasks':
      return <TasksPage onNavigate={onNavigate} />
    case 'schedules':
      return <ScheduledTasksPage onNavigate={onNavigate} />
    case 'events':
      return <SystemEventsPage onNavigate={onNavigate} />
    default:
      return <HomePage onNavigate={onNavigate} />
  }
}

export function TaskCenterAppPanel(_props: AppContentLoaderProps) {
  const [store] = useState(() => createTaskCenterModel())

  return (
    <TaskCenterStoreContext.Provider value={store}>
      <TaskCenterShell>
        {(nav, navigate) => <PageRouter nav={nav} onNavigate={navigate} />}
      </TaskCenterShell>
    </TaskCenterStoreContext.Provider>
  )
}
