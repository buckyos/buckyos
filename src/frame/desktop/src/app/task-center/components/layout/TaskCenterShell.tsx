/* ── TaskCenter shell: pages beside a detail panel on a desktop, pages then detail on a phone ── */

import { useCallback, useLayoutEffect, useRef, useState, type KeyboardEvent } from 'react'
import { useMediaQuery } from '@mui/material'
import { useI18n } from '../../../../i18n/provider'
import { Sidebar } from './Sidebar'
import { MobileTabBar } from './MobileTabBar'
import { useMobileBackHandler } from '../../../../desktop/windows/MobileNavContext'
import type { TaskCenterPage, TaskCenterNav } from './navigation'
import { HomePage } from '../../pages/HomePage'
import { TasksPage } from '../../pages/TasksPage'
import { ScheduledTasksPage } from '../../pages/ScheduledTasksPage'
import { SystemEventsPage } from '../../pages/SystemEventsPage'
import { TaskDetailPage } from '../../pages/TaskDetailPage'
import '../../taskCenter.css'

/** Below this width the detail panel covers the list instead of narrowing it. */
const OVERLAY_BELOW = 900

interface TaskCenterShellProps {
  /**
   * Initial navigation target. Only read on mount: the standalone route
   * remounts the shell (via `key`) whenever the `?taskid` query changes, so
   * no effect is needed to resync it.
   */
  initialTaskId?: string | null
}

export function TaskCenterShell({ initialTaskId }: TaskCenterShellProps) {
  const { t } = useI18n()
  const [nav, setNav] = useState<TaskCenterNav>(
    initialTaskId ? { page: 'tasks', taskId: initialTaskId } : { page: 'home' },
  )
  const isMobile = useMediaQuery('(max-width: 767px)')
  const rootRef = useRef<HTMLDivElement | null>(null)
  const mainRef = useRef<HTMLElement | null>(null)
  const listScroll = useRef(0)
  const [width, setWidth] = useState(0)

  useLayoutEffect(() => {
    const root = rootRef.current
    if (!root) return
    const observer = new ResizeObserver(([entry]) => setWidth(entry.contentRect.width))
    observer.observe(root)
    return () => observer.disconnect()
  }, [isMobile])

  // A phone shows the detail in place of the list: the detail starts at its
  // top and going back returns to where the list was.
  const mobileTaskId = isMobile ? nav.taskId : undefined
  useLayoutEffect(() => {
    mainRef.current?.scrollTo({ top: mobileTaskId ? 0 : listScroll.current })
  }, [nav.page, mobileTaskId])

  const navigate = (next: TaskCenterNav) => {
    if (next.page !== nav.page) listScroll.current = 0
    else if (!nav.taskId && next.taskId) listScroll.current = mainRef.current?.scrollTop ?? 0
    setNav(next)
  }
  const handlePage = (page: TaskCenterPage) => navigate({ page })
  const close = useCallback(() => setNav((current) => ({ page: current.page })), [])

  useMobileBackHandler(isMobile && nav.taskId ? close : null)

  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    // Dialogs render in portals; their Escape is theirs.
    if (event.key !== 'Escape' || !nav.taskId || !rootRef.current?.contains(event.target as Node)) return
    close()
  }

  const selectedId = isMobile ? undefined : nav.taskId
  const pageProps = { onNavigate: navigate, selectedId }
  const page = (() => {
    switch (nav.page) {
      case 'tasks':
        return <TasksPage {...pageProps} followFocus={!isMobile && nav.taskId != null} />
      case 'schedules':
        return <ScheduledTasksPage {...pageProps} />
      case 'events':
        return <SystemEventsPage {...pageProps} />
      default:
        return <HomePage {...pageProps} />
    }
  })()
  const detail = (variant: 'panel' | 'page') =>
    nav.taskId ? (
      <TaskDetailPage
        key={nav.taskId}
        taskId={nav.taskId}
        page={nav.page}
        variant={variant}
        onNavigate={navigate}
        onClose={close}
      />
    ) : null

  if (isMobile) {
    return (
      <div ref={rootRef} className="tc flex h-full w-full flex-col">
        <MobileTabBar currentPage={nav.page} onNavigate={handlePage} />
        <main ref={mainRef} className="min-h-0 flex-1 overflow-y-auto">
          <div className="p-4">
            <div hidden={nav.taskId != null}>{page}</div>
            {detail('page')}
          </div>
        </main>
      </div>
    )
  }

  return (
    <div ref={rootRef} className="tc flex h-full w-full" onKeyDown={handleKeyDown}>
      <Sidebar currentPage={nav.page} onNavigate={handlePage} />
      <div className="relative flex min-w-0 flex-1">
        <main ref={mainRef} className="desktop-scrollbar min-w-0 flex-1 overflow-y-auto">
          <div className="px-6 py-5">{page}</div>
        </main>
        {nav.taskId && (
          <aside
            className="tc-panel"
            data-overlay={width > 0 && width < OVERLAY_BELOW ? '' : undefined}
            aria-label={t('taskCenter.detail.info', 'Details')}
            data-testid="task-detail-panel"
          >
            {detail('panel')}
          </aside>
        )}
      </div>
    </div>
  )
}
