/* ── TaskCenter shell – responsive layout ── */

import { useCallback, useState, useEffect } from 'react'
import { useMediaQuery } from '@mui/material'
import { Sidebar } from './Sidebar'
import { MobileNavList } from './MobileNavList'
import { useMobileBackHandler } from '../../../../desktop/windows/MobileNavContext'
import type { TaskCenterPage, TaskCenterNav } from './navigation'

interface TaskCenterShellProps {
  initialTaskId?: string | null
  children: (nav: TaskCenterNav, navigate: (nav: TaskCenterNav) => void) => React.ReactNode
}

export function TaskCenterShell({ initialTaskId, children }: TaskCenterShellProps) {
  const [currentNav, setCurrentNav] = useState<TaskCenterNav | null>(
    initialTaskId ? { page: 'tasks', taskId: initialTaskId } : null,
  )
  const isMobile = useMediaQuery('(max-width: 767px)')

  // If initialTaskId changes (e.g. from route), update nav
  useEffect(() => {
    if (initialTaskId) {
      setCurrentNav({ page: 'tasks', taskId: initialTaskId })
    }
  }, [initialTaskId])

  const activeNav: TaskCenterNav | null = isMobile
    ? currentNav
    : (currentNav ?? { page: 'home' })

  const handleNavigate = (nav: TaskCenterNav) => setCurrentNav(nav)
  const handlePageNavigate = (page: TaskCenterPage) => setCurrentNav({ page })
  const handleBack = useCallback(() => setCurrentNav(null), [])

  useMobileBackHandler(isMobile && activeNav !== null ? handleBack : null)

  return (
    <div className="flex flex-col h-full w-full" style={{ background: 'var(--cp-bg)' }}>
      {isMobile ? (
        activeNav === null ? (
          <div className="flex-1 overflow-y-auto">
            <MobileNavList onNavigate={handlePageNavigate} />
          </div>
        ) : (
          <main className="flex-1 overflow-y-auto">
            <div className="px-4 pb-5 pt-2">
              {children(activeNav, handleNavigate)}
            </div>
          </main>
        )
      ) : (
        <div className="flex flex-1 min-h-0">
          <Sidebar currentPage={activeNav!.page} onNavigate={handlePageNavigate} />
          <main
            className="flex-1 overflow-y-auto desktop-scrollbar"
            style={{ background: 'var(--cp-bg)' }}
          >
            <div className="px-6 py-5 max-w-5xl">
              {children(activeNav!, handleNavigate)}
            </div>
          </main>
        </div>
      )}
    </div>
  )
}
