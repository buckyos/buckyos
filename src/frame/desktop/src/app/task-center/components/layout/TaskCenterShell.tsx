/* ── TaskCenter shell – responsive layout ── */

import { useCallback, useState, useEffect } from 'react'
import { useMediaQuery } from '@mui/material'
import { Sidebar } from './Sidebar'
import { MobileTabBar } from './MobileTabBar'
import { useMobileBackHandler } from '../../../../desktop/windows/MobileNavContext'
import type { TaskCenterPage, TaskCenterNav } from './navigation'

interface TaskCenterShellProps {
  initialTaskId?: string | null
  children: (nav: TaskCenterNav, navigate: (nav: TaskCenterNav) => void) => React.ReactNode
}

export function TaskCenterShell({ initialTaskId, children }: TaskCenterShellProps) {
  const [currentNav, setCurrentNav] = useState<TaskCenterNav>(
    initialTaskId ? { page: 'tasks', taskId: initialTaskId } : { page: 'home' },
  )
  const isMobile = useMediaQuery('(max-width: 767px)')

  // If initialTaskId changes (e.g. from route), update nav
  useEffect(() => {
    if (initialTaskId) {
      setCurrentNav({ page: 'tasks', taskId: initialTaskId })
    }
  }, [initialTaskId])

  const handleNavigate = (nav: TaskCenterNav) => setCurrentNav(nav)
  const handlePageNavigate = (page: TaskCenterPage) => setCurrentNav({ page })
  const handleBack = useCallback(() => setCurrentNav({ page: currentNav.page }), [currentNav.page])

  useMobileBackHandler(isMobile && currentNav.taskId ? handleBack : null)

  return (
    <div className="flex flex-col h-full w-full" style={{ background: 'var(--cp-bg)' }}>
      {isMobile ? (
        <>
          <MobileTabBar currentNav={currentNav} onNavigate={handlePageNavigate} />
          <main className="flex-1 overflow-y-auto">
            <div className="px-4 py-4">
              {children(currentNav, handleNavigate)}
            </div>
          </main>
        </>
      ) : (
        <div className="flex flex-1 min-h-0">
          <Sidebar currentPage={currentNav.page} onNavigate={handlePageNavigate} />
          <main
            className="flex-1 overflow-y-auto desktop-scrollbar"
            style={{ background: 'var(--cp-bg)' }}
          >
            <div className="px-6 py-5 max-w-5xl">
              {children(currentNav, handleNavigate)}
            </div>
          </main>
        </div>
      )}
    </div>
  )
}
