/* ── TaskCenter Task Detail Page ── */

import { useEffect, useMemo, useState } from 'react'
import {
  ArrowLeft,
  Play,
  CheckCircle2,
  XCircle,
  Clock,
  Pause,
  AlertTriangle,
  MessageSquare,
  CircleStop,
} from 'lucide-react'
import { useI18n } from '../../../i18n/provider'
import { useTaskCenterStore } from '../hooks/use-task-center-store'
import { notifyCardPath, type Task } from '../../../api/task_mgr'
import { ContentPreview } from '../../../components/ContentPreview'
import { formatBytes } from '../../../sysdlg/shareFormat'
import type { TaskCenterNav, TaskCenterPage } from '../components/layout/navigation'

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
      <span className="min-w-0 flex-1 break-all text-sm" style={{ color: 'var(--cp-text)' }}>
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

function TaskResults({ task }: { task: Task }) {
  const { t } = useI18n()
  const [activeObjectId, setActiveObjectId] = useState<string | null>(null)
  const artifacts = useMemo(() => {
    const record = (value: unknown): Record<string, unknown> =>
      value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {}
    if (task.schemaType !== 'aicc.compute/v1') return []
    const values = record(record(task.payload.result).output).artifacts
    if (!Array.isArray(values)) return []
    return values.flatMap((value) => {
      const artifact = record(value)
      const resource = record(artifact.resource)
      if (resource.kind !== 'named_object' || typeof resource.obj_id !== 'string' || !resource.obj_id.trim()) return []
      const size = record(artifact.metadata).size_bytes
      return [{
        source: { kind: 'object-id' as const, objectId: resource.obj_id.trim() },
        name: typeof artifact.name === 'string' && artifact.name.trim() ? artifact.name : resource.obj_id,
        mime: typeof artifact.mime === 'string' ? artifact.mime : undefined,
        size: typeof size === 'number' && Number.isFinite(size) && size >= 0 ? size : undefined,
      }]
    })
  }, [task.payload, task.schemaType])
  const active = artifacts.find((artifact) => artifact.source.objectId === activeObjectId)
  if (!artifacts.length) return null

  return (
    <section
      className="min-w-0 space-y-3 rounded-2xl p-4"
      style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}
      data-testid="task-results"
    >
      <h2 className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>
        {t('taskCenter.detail.results', 'Results')}
      </h2>
      {artifacts.map((artifact, index) => (
        <div key={`${artifact.source.objectId}:${index}`} className="flex min-w-0 flex-wrap items-center gap-3">
          <div className="min-w-0 flex-1 basis-40">
            <p className="break-all text-sm" style={{ color: 'var(--cp-text)' }}>{artifact.name}</p>
            <p className="break-all text-xs" style={{ color: 'var(--cp-muted)' }}>
              {[artifact.mime, artifact.size === undefined ? undefined : formatBytes(artifact.size)].filter(Boolean).join(' · ')}
            </p>
          </div>
          <button
            type="button"
            className="rounded-lg px-3 py-2 text-sm font-medium"
            style={{ color: 'var(--cp-accent)', border: '1px solid var(--cp-border)' }}
            aria-pressed={activeObjectId === artifact.source.objectId}
            onClick={() => setActiveObjectId(artifact.source.objectId)}
            data-testid="task-result-preview"
          >
            {t('taskCenter.detail.previewResult', 'Preview')}
          </button>
        </div>
      ))}
      {active && (
        <div className="overflow-hidden rounded-xl" style={{ height: 'clamp(280px, 55vh, 560px)' }}>
          <ContentPreview
            key={active.source.objectId}
            source={active.source}
            uiMode="visible"
            autoFocus
            onRequestExit={() => setActiveObjectId(null)}
            data-testid="task-result-content-preview"
          />
        </div>
      )}
    </section>
  )
}

interface TaskDetailPageProps {
  taskId: string
  backPage?: TaskCenterPage
  onNavigate: (nav: TaskCenterNav) => void
}

export function TaskDetailPage({ taskId, backPage = 'tasks', onNavigate }: TaskDetailPageProps) {
  const store = useTaskCenterStore()
  const { t } = useI18n()
  const [copyState, setCopyState] = useState('')
  const [canceling, setCanceling] = useState(false)
  const [cancelError, setCancelError] = useState<string | null>(null)
  // Lists carry metadata only; the payload/progress of a finished task is
  // fetched when its detail page opens.
  useEffect(() => {
    void store.loadTaskDetail(taskId)
  }, [store, taskId])
  const task = store.getTaskById(taskId)
  const backLabel =
    backPage === 'schedules'
      ? t('taskCenter.detail.backSchedules', 'Back to Scheduled Tasks')
      : t('taskCenter.detail.back', 'Back to Tasks')

  if (!task) {
    return (
      <div className="space-y-4">
        <button
          type="button"
          onClick={() => onNavigate({ page: backPage })}
          className="flex items-center gap-1.5 text-sm transition-colors"
          style={{ color: 'var(--cp-accent)' }}
        >
          <ArrowLeft size={16} />
          {backLabel}
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
    task.error ?? (task.status === 'failed' && task.payload?.error
      ? String(task.payload.error)
      : null)

  const cancel = async () => {
    setCanceling(true)
    setCancelError(null)
    try {
      await store.cancelTask(task.taskId)
    } catch (error) {
      setCancelError(error instanceof Error ? error.message : String(error))
    } finally {
      setCanceling(false)
    }
  }

  return (
    <div className="space-y-5">
      {/* Back link */}
      <button
        type="button"
        onClick={() => onNavigate({ page: backPage })}
        className="flex items-center gap-1.5 text-sm transition-colors"
        style={{ color: 'var(--cp-accent)' }}
      >
        <ArrowLeft size={16} />
        {backLabel}
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
        {task.cancellable && (
          <button
            type="button"
            onClick={() => void cancel()}
            disabled={canceling}
            className="mt-3 inline-flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-xs font-medium transition-colors disabled:opacity-60"
            style={{
              background: 'var(--cp-surface)',
              color: 'var(--cp-danger)',
              border: '1px solid color-mix(in srgb, var(--cp-danger) 30%, var(--cp-border))',
            }}
            data-testid="task-cancel"
          >
            <CircleStop size={14} />
            {t('taskCenter.detail.cancel', 'Cancel Task')}
          </button>
        )}
        {task.pendingControlAction === 'Cancel' && task.status !== 'cancelled' && (
          <p className="mt-2 text-xs" style={{ color: 'var(--cp-muted)' }}>
            {t('taskCenter.detail.canceling', 'Canceling…')}
          </p>
        )}
        {cancelError && (
          <p className="mt-2 text-xs" style={{ color: 'var(--cp-danger)' }}>
            {cancelError}
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

      <TaskResults key={task.taskId} task={task} />

      {/* A URL download: where from and where it was saved */}
      {task.download && (
        <section
          className="rounded-2xl p-4"
          style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}
          data-testid="task-download-info"
        >
          <h2
            className="text-xs font-semibold uppercase tracking-wide mb-2"
            style={{ color: 'var(--cp-muted)' }}
          >
            {t('taskCenter.download.section', 'Download')}
          </h2>
          <InfoRow label={t('taskCenter.download.url', 'Link')} value={<span className="break-all">{task.download.url}</span>} />
          {task.download.localPath && (
            <InfoRow
              label={t('taskCenter.download.savedTo', 'Saved to')}
              value={<span className="break-all font-mono text-xs" data-testid="task-download-path">{task.download.localPath}</span>}
            />
          )}
        </section>
      )}

      {/* The task of a System Notify is answered on its MessageHub card */}
      {task.notifyId && (
        <div
          className="flex flex-wrap items-center gap-3 rounded-xl p-3 text-sm"
          style={{
            background: 'color-mix(in srgb, var(--cp-accent-soft) 14%, var(--cp-surface))',
            border: '1px solid color-mix(in srgb, var(--cp-accent) 22%, var(--cp-border))',
            color: 'var(--cp-text)',
          }}
          data-testid="task-notify"
        >
          <span className="min-w-0 flex-1">{t('taskCenter.notify.hint', 'This task is the answer to a notification. Handle it on its card in MessageHub.')}</span>
          <a
            href={notifyCardPath(task.notifyId)}
            className="inline-flex items-center gap-1.5 font-medium"
            style={{ color: 'var(--cp-accent)' }}
            data-testid="task-notify-link"
          >
            <MessageSquare size={15} />
            {t('taskCenter.notify.open', 'Handle in MessageHub')}
          </a>
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

      {task.aiccRequest && Object.keys(task.aiccRequest.params).length > 0 && <section
        className="rounded-2xl p-4 min-w-0"
        style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}
      >
        <div className="flex flex-wrap items-center justify-between gap-2 mb-3">
          <h2 className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.playground.taskRequest')}</h2>
          <button type="button" className="text-sm" style={{ color: 'var(--cp-accent)' }} onClick={async () => {
            try { await navigator.clipboard.writeText(JSON.stringify(task.aiccRequest, null, 2)); setCopyState('common.copied') }
            catch { setCopyState('aiCenter.playground.copyFailed') }
          }}>{t(copyState || 'aiCenter.playground.copyRequest')}</button>
        </div>
        <p className="text-xs mb-3" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.playground.taskRequestHint')}</p>
        <pre className="text-xs overflow-auto max-h-96 whitespace-pre-wrap break-all" style={{ color: 'var(--cp-text)' }}>{JSON.stringify(task.aiccRequest, null, 2)}</pre>
      </section>}

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
            className="text-xs overflow-x-auto whitespace-pre-wrap break-all"
            style={{ color: 'var(--cp-text)' }}
          >
            {JSON.stringify(task.payload, null, 2)}
          </pre>
        </section>
      )}
    </div>
  )
}
