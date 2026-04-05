/* ── TaskCenter standalone route – supports ?taskid=xxx ── */

import { useState } from 'react'
import { useSearchParams } from 'react-router-dom'
import { useI18n } from '../../i18n/provider'
import { useThemeMode } from '../../theme/provider'
import { TaskCenterStoreContext } from './hooks/use-task-center-store'
import { TaskCenterMockStore } from './mock/store'
import { TaskCenterShell } from './components/layout/TaskCenterShell'
import type { TaskCenterNav } from './components/layout/navigation'
import { HomePage } from './pages/HomePage'
import { TasksPage } from './pages/TasksPage'
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
    return <TaskDetailPage taskId={nav.taskId} onNavigate={onNavigate} />
  }

  switch (nav.page) {
    case 'home':
      return <HomePage onNavigate={onNavigate} />
    case 'tasks':
      return <TasksPage onNavigate={onNavigate} />
    case 'events':
      return <SystemEventsPage onNavigate={onNavigate} />
    default:
      return <HomePage onNavigate={onNavigate} />
  }
}

export function TaskCenterRoute() {
  const [searchParams] = useSearchParams()
  const { locale } = useI18n()
  const { themeMode } = useThemeMode()
  const taskId = searchParams.get('taskid') ?? undefined
  const [store] = useState(() => new TaskCenterMockStore())

  return (
    <main className="min-h-dvh bg-[color:var(--cp-bg)] px-0 py-0 md:px-5 md:py-5">
      <div
        className="mx-auto h-dvh w-full overflow-hidden md:h-[calc(100dvh-2.5rem)] md:max-w-[1480px] md:rounded-[28px] md:border md:shadow-[var(--cp-window-shadow)]"
        style={{
          borderColor: 'var(--cp-border)',
          background:
            'linear-gradient(180deg, color-mix(in srgb, var(--cp-surface) 96%, transparent), color-mix(in srgb, var(--cp-surface-2) 94%, transparent))',
          backdropFilter: 'blur(20px)',
        }}
      >
        <TaskCenterStoreContext.Provider value={store}>
          <TaskCenterShell
            key={`${taskId}:${themeMode}:${locale}`}
            initialTaskId={taskId}
          >
            {(nav, navigate) => <PageRouter nav={nav} onNavigate={navigate} />}
          </TaskCenterShell>
        </TaskCenterStoreContext.Provider>
      </div>
    </main>
  )
}
