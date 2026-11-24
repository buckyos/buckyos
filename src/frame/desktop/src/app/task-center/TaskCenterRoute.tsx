/* ── TaskCenter standalone route – supports ?taskid=xxx ── */

import { useEffect, useState } from 'react'
import { useSearchParams } from 'react-router-dom'
import clsx from 'clsx'
import { useI18n } from '../../i18n/provider'
import { useThemeMode } from '../../theme/provider'
import { useStandalonePage } from '../standalone/context'
import { StandalonePage } from '../standalone/StandalonePage'
import { TaskCenterStoreContext } from './hooks/use-task-center-store'
import { createTaskCenterModel } from '../../api/task_mgr'
import { TaskCenterShell } from './components/layout/TaskCenterShell'

function TaskCenterPage() {
  const [searchParams] = useSearchParams()
  const { locale } = useI18n()
  const { themeMode } = useThemeMode()
  const taskId = searchParams.get('taskid') ?? undefined
  const [store] = useState(() => createTaskCenterModel())
  useEffect(() => () => store.dispose(), [store])
  const inBrowser = useStandalonePage()?.container === 'browser'

  return (
    <div className={clsx('min-h-0 flex-1', inBrowser && 'md:p-5')}>
      <div
        className={clsx('mx-auto h-full w-full overflow-hidden', inBrowser && 'md:max-w-[1480px] md:rounded-2xl md:border md:shadow-[var(--cp-window-shadow)]')}
        style={{ borderColor: 'var(--cp-border)', background: 'var(--cp-bg)' }}
      >
        <TaskCenterStoreContext.Provider value={store}>
          <TaskCenterShell key={`${taskId}:${themeMode}:${locale}`} initialTaskId={taskId} />
        </TaskCenterStoreContext.Provider>
      </div>
    </div>
  )
}

export function TaskCenterRoute() {
  return <StandalonePage appId="task-center" routed>{() => <TaskCenterPage />}</StandalonePage>
}
