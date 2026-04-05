/* ── TaskCenter Task Detail Page ── */

import {
  ArrowLeft,
  Play,
  CheckCircle2,
  XCircle,
  Clock,
  Pause,
  AlertTriangle,
} from 'lucide-react'
import { useI18n } from '../../../i18n/provider'
import { useTaskCenterStore } from '../hooks/use-task-center-store'
import type { Task } from '../mock/types'
import type { TaskCenterNav } from '../components/layout/navigation'

function statusIcon(status: Task['status'], size = 16) {
  switch (status) {
    case 'running':
      return <Play size={size} />
    case 'paused':
      return <Pause size={size} />
    case 'completed':
      return <CheckCircle2 size={size} />
    case 'failed':
      return <XCircle size={size} />
    default:
      return <Clock size={size} />
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
    default:
      return 'var(--cp-muted)'
  }
}

function formatTime(iso: string | null) {
  if (!iso) return '—'
  const d = new Date(iso)
  return d.toLocaleString(undefined, {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  })
}

function InfoRow({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="flex items-start gap-3 py-2">
      <span className="text-xs font-medium w-28 shrink-0 pt-0.5" style={{ color: 'var(--cp-muted)' }}>
        {label}
      </span>
      <span className="text-sm" style={{ color: 'var(--cp-text)' }}>
        {value}
      </span>
    </div>
  )
}

function SubTaskRow({ task }: { task: Task }) {
  return (
    <div
      className="flex items-center gap-3 rounded-xl p-3"
      style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}
    >
      <div
        className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg"
        style={{
          background: `color-mix(in srgb, ${statusColor(task.status)} 14%, transparent)`,
          color: statusColor(task.status),
        }}
      >
        {statusIcon(task.status, 12)}
      </div>
      <div className="flex-1 min-w-0">
        <div className="text-sm truncate" style={{ color: 'var(--cp-text)' }}>
          {task.title}
        </div>
        {task.summary && (
          <div className="text-xs mt-0.5 truncate" style={{ color: 'var(--cp-muted)' }}>
            {task.summary}
          </div>
        )}
      </div>
      <span
        className="text-xs font-medium uppercase shrink-0"
        style={{ color: statusColor(task.status) }}
      >
        {task.status}
      </span>
      {task.progress != null && (
        <span className="text-xs shrink-0" style={{ color: 'var(--cp-muted)' }}>
          {task.progress}%
        </span>
      )}
    </div>
  )
}

interface TaskDetailPageProps {
  taskId: string
  onNavigate: (nav: TaskCenterNav) => void
}

export function TaskDetailPage({ taskId, onNavigate }: TaskDetailPageProps) {
  const store = useTaskCenterStore()
  const { t } = useI18n()
  const task = store.getTaskById(taskId)

  if (!task) {
    return (
      <div className="space-y-4">
        <button
          type="button"
          onClick={() => onNavigate({ page: 'tasks' })}
          className="flex items-center gap-1.5 text-sm transition-colors"
          style={{ color: 'var(--cp-accent)' }}
        >
          <ArrowLeft size={16} />
          {t('taskCenter.detail.back', 'Back to Tasks')}
        </button>
        <div
          className="flex flex-col items-center justify-center gap-3 py-16"
          style={{ color: 'var(--cp-muted)' }}
        >
          <AlertTriangle size={32} />
          <div className="text-sm">
            {t('taskCenter.detail.notFound', 'Task not found')}: {taskId}
          </div>
        </div>
      </div>
    )
  }

  const errorMsg =
    task.status === 'failed' && task.payload?.error
      ? String(task.payload.error)
      : null

  return (
    <div className="space-y-5">
      {/* Back link */}
      <button
        type="button"
        onClick={() => onNavigate({ page: 'tasks' })}
        className="flex items-center gap-1.5 text-sm transition-colors"
        style={{ color: 'var(--cp-accent)' }}
      >
        <ArrowLeft size={16} />
        {t('taskCenter.detail.back', 'Back to Tasks')}
      </button>

      {/* Header */}
      <div>
        <div className="flex items-center gap-2.5 mb-2">
          <div
            className="flex h-10 w-10 items-center justify-center rounded-xl"
            style={{
              background: `color-mix(in srgb, ${statusColor(task.status)} 14%, transparent)`,
              color: statusColor(task.status),
            }}
          >
            {statusIcon(task.status, 20)}
          </div>
          <div>
            <h1 className="text-base font-semibold" style={{ color: 'var(--cp-text)' }}>
              {task.title}
            </h1>
            <span
              className="text-xs font-medium uppercase"
              style={{ color: statusColor(task.status) }}
            >
              {task.status}
            </span>
          </div>
        </div>
        {task.summary && (
          <p className="text-sm" style={{ color: 'var(--cp-muted)' }}>
            {task.summary}
          </p>
        )}
      </div>

      {/* Progress bar */}
      {task.progress != null && (
        <div>
          <div
            className="h-2 w-full rounded-full overflow-hidden"
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
            {task.progress}% complete
          </div>
        </div>
      )}

      {/* Error message */}
      {errorMsg && (
        <div
          className="rounded-xl p-3 text-sm"
          style={{
            background: 'color-mix(in srgb, var(--cp-danger) 8%, var(--cp-surface))',
            border: '1px solid color-mix(in srgb, var(--cp-danger) 30%, var(--cp-border))',
            color: 'var(--cp-danger)',
          }}
        >
          {errorMsg}
        </div>
      )}

      {/* Info section */}
      <section
        className="rounded-2xl p-4"
        style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}
      >
        <h2
          className="text-xs font-semibold uppercase tracking-wide mb-2"
          style={{ color: 'var(--cp-muted)' }}
        >
          {t('taskCenter.detail.info', 'Task Information')}
        </h2>
        <div className="divide-y" style={{ borderColor: 'var(--cp-border)' }}>
          <InfoRow label="Task ID" value={task.taskId} />
          <InfoRow label="Root Task ID" value={task.rootTaskId} />
          <InfoRow label="Type" value={task.type} />
          <InfoRow label="Source" value={task.source} />
          <InfoRow label="Created" value={formatTime(task.createdAt)} />
          <InfoRow label="Started" value={formatTime(task.startedAt)} />
          <InfoRow label="Ended" value={formatTime(task.endedAt)} />
          <InfoRow label="Updated" value={formatTime(task.updatedAt)} />
          {task.schemaType && <InfoRow label="Schema" value={task.schemaType} />}
        </div>
      </section>

      {/* Sub-tasks */}
      {task.children.length > 0 && (
        <section>
          <h2
            className="text-xs font-semibold uppercase tracking-wide mb-3"
            style={{ color: 'var(--cp-muted)' }}
          >
            {t('taskCenter.detail.subtasks', 'Sub-tasks')} ({task.children.length})
          </h2>
          <div className="space-y-1.5">
            {task.children.map((child) => (
              <SubTaskRow key={child.taskId} task={child} />
            ))}
          </div>
        </section>
      )}

      {/* Raw payload */}
      {Object.keys(task.payload).length > 0 && (
        <section
          className="rounded-2xl p-4"
          style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}
        >
          <h2
            className="text-xs font-semibold uppercase tracking-wide mb-2"
            style={{ color: 'var(--cp-muted)' }}
          >
            {t('taskCenter.detail.payload', 'Extended Data')}
          </h2>
          <pre
            className="text-xs overflow-x-auto whitespace-pre-wrap"
            style={{ color: 'var(--cp-text)' }}
          >
            {JSON.stringify(task.payload, null, 2)}
          </pre>
        </section>
      )}
    </div>
  )
}
