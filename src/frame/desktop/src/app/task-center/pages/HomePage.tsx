/* ── TaskCenter Home page ── */

import { useState } from 'react'
import { Download } from 'lucide-react'
import { useTaskCenterStore } from '../hooks/use-task-center-store'
import { NewDownloadDialog } from '../components/NewDownloadDialog'
import { TaskRow } from '../components/TaskTree'
import { StatusDot } from '../components/taskView'
import { useTaskText, type Tone } from '../components/taskModel'
import type { SystemNotification, Task } from '../../../api/task_mgr'
import type { TaskCenterNav } from '../components/layout/navigation'

function severityTone(severity: SystemNotification['severity']): Tone {
  switch (severity) {
    case 'critical':
      return 'failed'
    case 'warning':
      return 'waiting'
    default:
      return 'running'
  }
}

interface HomePageProps {
  onNavigate: (nav: TaskCenterNav) => void
  selectedId?: string
}

export function HomePage({ onNavigate, selectedId }: HomePageProps) {
  const store = useTaskCenterStore()
  const text = useTaskText()
  const { t } = text
  const [, setTick] = useState(0)
  const [downloadOpen, setDownloadOpen] = useState(false)

  const runningTasks = store.getRunningTasks()
  const recentFinished = store.getRecentFinishedTasks().slice(0, 3)
  const pendingNotifications = store.getPendingNotifications()

  const handleNotificationAction = (id: string, action: string) => {
    store.handleNotification(id, action)
    setTick((n) => n + 1)
  }

  const taskList = (tasks: Task[]) => (
    <div className="tc-list">
      {tasks.map((task) => (
        <TaskRow
          key={task.taskId}
          task={task}
          text={text}
          selected={task.taskId === selectedId}
          onSelect={() => onNavigate({ page: 'home', taskId: task.taskId })}
        />
      ))}
    </div>
  )

  return (
    <div className="space-y-8">
      {runningTasks.length > 0 && (
        <section>
          <h2 className="tc-heading mb-2 px-1">
            {t('taskCenter.home.running', 'Running')}
            <span className="tc-num">{runningTasks.length}</span>
          </h2>
          {taskList(runningTasks)}
        </section>
      )}

      {recentFinished.length > 0 && (
        <section>
          <h2 className="tc-heading mb-2 px-1">{t('taskCenter.home.recent', 'Recently finished')}</h2>
          {taskList(recentFinished)}
        </section>
      )}

      {pendingNotifications.length > 0 && (
        <section>
          <h2 className="tc-heading mb-2 px-1">
            {t('taskCenter.home.notifications', 'System notifications')}
            <span className="tc-num">{pendingNotifications.length}</span>
          </h2>
          <div className="tc-surface">
            {pendingNotifications.map((notification, index) => (
              <article
                key={notification.id}
                className="flex gap-2 p-3"
                style={index > 0 ? { borderTop: '1px solid var(--tc-line)' } : undefined}
              >
                <span className="tc-row-dot">
                  <StatusDot tone={severityTone(notification.severity)} />
                </span>
                <div className="min-w-0 flex-1">
                  <div className="flex items-baseline justify-between gap-3">
                    <h3 className="text-sm font-medium leading-5">{notification.title}</h3>
                    <span className="tc-num tc-muted shrink-0 text-xs">{text.listTime(notification.createdAt)}</span>
                  </div>
                  <p className="tc-muted mt-0.5 text-[13px] leading-5">{notification.summary}</p>
                  <div className="mt-3 flex flex-wrap gap-2">
                    {notification.actions.map((action) => (
                      <button
                        key={action}
                        type="button"
                        onClick={() => handleNotificationAction(notification.id, action)}
                        className="tc-btn"
                        data-variant={action === 'approve' || action === 'confirm' ? 'primary' : undefined}
                      >
                        {t(`taskCenter.action.${action}`, action)}
                      </button>
                    ))}
                  </div>
                </div>
              </article>
            ))}
          </div>
        </section>
      )}

      <section>
        <button
          type="button"
          onClick={() => setDownloadOpen(true)}
          className="tc-btn h-9"
          data-testid="task-center-new-download"
        >
          <Download size={16} strokeWidth={1.75} />
          {t('taskCenter.download.new', 'New download')}
        </button>
      </section>
      <NewDownloadDialog
        open={downloadOpen}
        onClose={() => setDownloadOpen(false)}
        onCreated={(taskId) => {
          setDownloadOpen(false)
          onNavigate({ page: 'tasks', taskId })
        }}
      />

      {runningTasks.length === 0 && recentFinished.length === 0 && pendingNotifications.length === 0 && (
        <p className="tc-empty">{t('taskCenter.home.empty', 'No running tasks or pending notifications')}</p>
      )}
    </div>
  )
}
