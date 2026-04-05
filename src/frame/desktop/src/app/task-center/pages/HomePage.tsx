/* ── TaskCenter Home Page ── */

import { useState } from 'react'
import {
  Play,
  CheckCircle2,
  XCircle,
  AlertTriangle,
  Clock,
  ChevronRight,
  Shield,
  Plus,
  Pause,
} from 'lucide-react'
import { useI18n } from '../../../i18n/provider'
import { useTaskCenterStore } from '../hooks/use-task-center-store'
import type { Task, SystemNotification } from '../mock/types'
import type { TaskCenterNav } from '../components/layout/navigation'

function statusIcon(status: Task['status']) {
  switch (status) {
    case 'running':
      return <Play size={14} />
    case 'paused':
      return <Pause size={14} />
    case 'completed':
      return <CheckCircle2 size={14} />
    case 'failed':
      return <XCircle size={14} />
    case 'cancelled':
      return <Clock size={14} />
    default:
      return <Clock size={14} />
  }
}

function statusColor(status: Task['status']) {
  switch (status) {
    case 'running':
      return 'var(--cp-accent)'
    case 'paused':
      return 'var(--cp-warning)'
    case 'completed':
      return 'var(--cp-success)'
    case 'failed':
      return 'var(--cp-danger)'
    case 'cancelled':
      return 'var(--cp-muted)'
    default:
      return 'var(--cp-muted)'
  }
}

function severityColor(severity: SystemNotification['severity']) {
  switch (severity) {
    case 'critical':
      return 'var(--cp-danger)'
    case 'warning':
      return 'var(--cp-warning)'
    default:
      return 'var(--cp-accent)'
  }
}

function formatTime(iso: string) {
  const d = new Date(iso)
  return d.toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })
}

function TaskCard({
  task,
  onOpen,
}: {
  task: Task
  onOpen: () => void
}) {
  return (
    <button
      type="button"
      onClick={onOpen}
      className="w-full rounded-2xl p-4 text-left transition-colors"
      style={{
        background: 'var(--cp-surface)',
        border: '1px solid var(--cp-border)',
      }}
    >
      <div className="flex items-start justify-between gap-3">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 mb-1">
            <span style={{ color: statusColor(task.status) }}>{statusIcon(task.status)}</span>
            <span
              className="text-xs font-medium uppercase tracking-wide"
              style={{ color: statusColor(task.status) }}
            >
              {task.status}
            </span>
            <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>
              · {task.source}
            </span>
          </div>
          <div className="text-sm font-medium truncate" style={{ color: 'var(--cp-text)' }}>
            {task.title}
          </div>
          <div className="text-xs mt-0.5 truncate" style={{ color: 'var(--cp-muted)' }}>
            {task.summary}
          </div>
          {task.progress != null && (
            <div className="mt-2.5">
              <div
                className="h-1.5 w-full rounded-full overflow-hidden"
                style={{ background: 'var(--cp-border)' }}
              >
                <div
                  className="h-full rounded-full transition-all"
                  style={{
                    width: `${task.progress}%`,
                    background: statusColor(task.status),
                  }}
                />
              </div>
              <div className="text-xs mt-1" style={{ color: 'var(--cp-muted)' }}>
                {task.progress}%
              </div>
            </div>
          )}
        </div>
        <ChevronRight size={16} style={{ color: 'var(--cp-muted)', marginTop: 2 }} />
      </div>
      <div className="text-xs mt-2" style={{ color: 'var(--cp-muted)' }}>
        Updated {formatTime(task.updatedAt)}
      </div>
    </button>
  )
}

function NotificationCard({
  notification,
  onAction,
}: {
  notification: SystemNotification
  onAction: (action: string) => void
}) {
  return (
    <div
      className="rounded-2xl p-4"
      style={{
        background: 'var(--cp-surface)',
        border: '1px solid var(--cp-border)',
      }}
    >
      <div className="flex items-start gap-3">
        <div
          className="flex h-8 w-8 shrink-0 items-center justify-center rounded-xl"
          style={{
            background: `color-mix(in srgb, ${severityColor(notification.severity)} 14%, transparent)`,
            color: severityColor(notification.severity),
          }}
        >
          {notification.severity === 'critical' ? (
            <Shield size={16} />
          ) : (
            <AlertTriangle size={16} />
          )}
        </div>
        <div className="flex-1 min-w-0">
          <div className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>
            {notification.title}
          </div>
          <div className="text-xs mt-0.5" style={{ color: 'var(--cp-muted)' }}>
            {notification.summary}
          </div>
          <div className="flex gap-2 mt-3">
            {notification.actions.map((action) => (
              <button
                key={action}
                type="button"
                onClick={() => onAction(action)}
                className="rounded-lg px-3 py-1.5 text-xs font-medium transition-colors"
                style={{
                  background:
                    action === 'approve' || action === 'confirm'
                      ? 'var(--cp-accent)'
                      : 'var(--cp-surface-2)',
                  color:
                    action === 'approve' || action === 'confirm'
                      ? 'white'
                      : 'var(--cp-text)',
                  border: '1px solid var(--cp-border)',
                }}
              >
                {action.charAt(0).toUpperCase() + action.slice(1)}
              </button>
            ))}
          </div>
        </div>
      </div>
      <div className="text-xs mt-2" style={{ color: 'var(--cp-muted)' }}>
        {formatTime(notification.createdAt)}
      </div>
    </div>
  )
}

interface HomePageProps {
  onNavigate: (nav: TaskCenterNav) => void
}

export function HomePage({ onNavigate }: HomePageProps) {
  const store = useTaskCenterStore()
  const { t } = useI18n()
  const [, setTick] = useState(0)

  const runningTasks = store.getRunningTasks()
  const recentFinished = store.getRecentFinishedTasks().slice(0, 3)
  const pendingNotifications = store.getPendingNotifications()

  const handleNotificationAction = (id: string, action: string) => {
    store.handleNotification(id, action)
    setTick((n) => n + 1)
  }

  return (
    <div className="space-y-6">
      {/* Running Tasks */}
      {runningTasks.length > 0 && (
        <section>
          <h2
            className="text-xs font-semibold uppercase tracking-wide mb-3"
            style={{ color: 'var(--cp-muted)' }}
          >
            {t('taskCenter.home.running', 'Running Tasks')} ({runningTasks.length})
          </h2>
          <div className="space-y-2">
            {runningTasks.map((task) => (
              <TaskCard
                key={task.taskId}
                task={task}
                onOpen={() => onNavigate({ page: 'tasks', taskId: task.taskId })}
              />
            ))}
          </div>
        </section>
      )}

      {/* Recent Finished */}
      {recentFinished.length > 0 && (
        <section>
          <h2
            className="text-xs font-semibold uppercase tracking-wide mb-3"
            style={{ color: 'var(--cp-muted)' }}
          >
            {t('taskCenter.home.recent', 'Recently Finished')}
          </h2>
          <div className="space-y-2">
            {recentFinished.map((task) => (
              <TaskCard
                key={task.taskId}
                task={task}
                onOpen={() => onNavigate({ page: 'tasks', taskId: task.taskId })}
              />
            ))}
          </div>
        </section>
      )}

      {/* System Notifications */}
      {pendingNotifications.length > 0 && (
        <section>
          <h2
            className="text-xs font-semibold uppercase tracking-wide mb-3"
            style={{ color: 'var(--cp-muted)' }}
          >
            {t('taskCenter.home.notifications', 'System Notifications')} ({pendingNotifications.length})
          </h2>
          <div className="space-y-2">
            {pendingNotifications.map((notif) => (
              <NotificationCard
                key={notif.id}
                notification={notif}
                onAction={(action) => handleNotificationAction(notif.id, action)}
              />
            ))}
          </div>
        </section>
      )}

      {/* Create Task */}
      <section>
        <button
          type="button"
          className="flex items-center gap-2 rounded-2xl px-4 py-3 text-sm font-medium transition-colors"
          style={{
            background: 'var(--cp-surface)',
            color: 'var(--cp-accent)',
            border: '1px solid var(--cp-border)',
          }}
        >
          <Plus size={16} />
          {t('taskCenter.home.createTask', 'Create Task')}
        </button>
      </section>

      {/* Empty state */}
      {runningTasks.length === 0 && recentFinished.length === 0 && pendingNotifications.length === 0 && (
        <div
          className="text-center py-12 text-sm"
          style={{ color: 'var(--cp-muted)' }}
        >
          {t('taskCenter.home.empty', 'No active tasks or pending notifications.')}
        </div>
      )}
    </div>
  )
}
