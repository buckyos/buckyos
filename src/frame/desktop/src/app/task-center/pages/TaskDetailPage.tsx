/* ── TaskCenter Task Detail Page ── */

import { useState } from 'react'
import {
  ArrowLeft,
  Play,
  CheckCircle2,
  XCircle,
  Clock,
  Pause,
  AlertTriangle,
  MessageSquareText,
  Send,
} from 'lucide-react'
import { useI18n } from '../../../i18n/provider'
import { useTaskCenterStore } from '../hooks/use-task-center-store'
import type { Task } from '../../../api/task_mgr'
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

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}

function asRecord(value: unknown): Record<string, unknown> {
  return isRecord(value) ? value : {}
}

function asString(value: unknown): string | null {
  return typeof value === 'string' && value.trim() ? value : null
}

function formatHumanActionTime(value: unknown) {
  if (typeof value === 'number' && Number.isFinite(value)) {
    return formatTime(new Date(value * 1000).toISOString())
  }
  if (typeof value === 'string' && value.trim()) {
    const date = new Date(value)
    if (!Number.isNaN(date.getTime())) return formatTime(date.toISOString())
  }
  return null
}

type TaskInteractionSchema =
  | {
      kind: 'approval'
      title: string
      summary: string
      approveLabel: string
      rejectLabel: string
    }
  | {
      kind: 'comment'
      title: string
      summary: string
      placeholder: string
      submitLabel: string
      outputKey: string
    }

function taskInteractionRecord(task: Task): Record<string, unknown> {
  return asRecord(task.payload.interaction ?? task.payload.human_interaction ?? task.payload.schema)
}

function getTaskInteractionSchema(task: Task): TaskInteractionSchema | null {
  const interaction = taskInteractionRecord(task)
  const schemaType = (task.schemaType ?? asString(task.payload.schema_type) ?? asString(task.payload.schemaType) ?? '')
    .toLowerCase()
  const interactionKind = (
    asString(interaction.kind) ??
    asString(task.payload.interaction_kind) ??
    schemaType
  ).toLowerCase()
  const title = asString(interaction.title) ?? asString(task.payload.prompt_title) ?? task.title
  const summary =
    asString(interaction.summary) ??
    asString(interaction.description) ??
    asString(task.payload.prompt) ??
    task.summary

  const approvalSchema =
    task.payload.backendStatus === 'WaitingForApproval' ||
    interactionKind.includes('approval') ||
    interactionKind.includes('approve') ||
    schemaType === 'human/approval' ||
    schemaType === 'task/approval'

  if (approvalSchema) {
    return {
      kind: 'approval',
      title,
      summary,
      approveLabel: asString(interaction.approveLabel) ?? asString(interaction.approve_label) ?? 'Approve',
      rejectLabel: asString(interaction.rejectLabel) ?? asString(interaction.reject_label) ?? 'Reject',
    }
  }

  const commentSchema =
    interactionKind.includes('comment') ||
    interactionKind.includes('suggest') ||
    interactionKind.includes('feedback') ||
    schemaType === 'submit_output' ||
    schemaType === 'human/comment' ||
    schemaType === 'human/suggestion'

  if (commentSchema) {
    return {
      kind: 'comment',
      title,
      summary,
      placeholder:
        asString(interaction.placeholder) ??
        asString(task.payload.placeholder) ??
        'Write a response for this task...',
      submitLabel: asString(interaction.submitLabel) ?? asString(interaction.submit_label) ?? 'Submit response',
      outputKey:
        asString(interaction.outputKey) ??
        asString(interaction.output_key) ??
        (interactionKind.includes('suggest') ? 'suggestion' : 'comment'),
    }
  }

  return null
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

function TaskInteractionPanel({
  task,
  schema,
  onSubmit,
  t,
}: {
  task: Task
  schema: TaskInteractionSchema
  onSubmit: (action: string, payload?: Record<string, unknown>) => Promise<void>
  t: (key: string, fallback: string) => string
}) {
  const [comment, setComment] = useState('')
  const [isSubmitting, setIsSubmitting] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const humanAction = asRecord(task.payload.human_action)
  const submittedKind = asString(humanAction.kind)
  const submittedAt =
    formatHumanActionTime(humanAction.submitted_at) ??
    formatHumanActionTime(humanAction.acted_at)

  const submit = async (action: string, payload?: Record<string, unknown>) => {
    setIsSubmitting(true)
    setError(null)
    try {
      await onSubmit(action, payload)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setIsSubmitting(false)
    }
  }

  return (
    <section
      className="rounded-2xl p-4"
      style={{
        background: 'color-mix(in srgb, var(--cp-accent) 6%, var(--cp-surface))',
        border: '1px solid color-mix(in srgb, var(--cp-accent) 22%, var(--cp-border))',
      }}
    >
      <div className="flex items-start gap-3">
        <div
          className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl"
          style={{
            background: 'color-mix(in srgb, var(--cp-accent) 14%, transparent)',
            color: 'var(--cp-accent)',
          }}
        >
          <MessageSquareText size={17} />
        </div>
        <div className="min-w-0 flex-1">
          <h2 className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>
            {schema.title}
          </h2>
          {schema.summary && (
            <p className="mt-1 text-xs" style={{ color: 'var(--cp-muted)' }}>
              {schema.summary}
            </p>
          )}

          {submittedKind && (
            <div
              className="mt-3 rounded-xl px-3 py-2 text-xs"
              style={{
                background: 'color-mix(in srgb, var(--cp-success) 10%, var(--cp-surface))',
                border: '1px solid color-mix(in srgb, var(--cp-success) 26%, var(--cp-border))',
                color: 'var(--cp-success)',
              }}
            >
              {t('taskCenter.detail.interactionSubmitted', 'Submitted')}
              {submittedAt ? ` · ${submittedAt}` : ''}
            </div>
          )}

          {schema.kind === 'approval' && (
            <div className="mt-4 flex flex-wrap gap-2">
              <button
                type="button"
                disabled={isSubmitting || Boolean(submittedKind)}
                onClick={() => submit('approve')}
                className="rounded-lg px-3 py-2 text-xs font-medium transition-colors disabled:opacity-50"
                style={{
                  background: 'var(--cp-accent)',
                  color: 'white',
                  border: '1px solid var(--cp-accent)',
                }}
              >
                {schema.approveLabel}
              </button>
              <button
                type="button"
                disabled={isSubmitting || Boolean(submittedKind)}
                onClick={() => submit('reject')}
                className="rounded-lg px-3 py-2 text-xs font-medium transition-colors disabled:opacity-50"
                style={{
                  background: 'var(--cp-surface)',
                  color: 'var(--cp-text)',
                  border: '1px solid var(--cp-border)',
                }}
              >
                {schema.rejectLabel}
              </button>
            </div>
          )}

          {schema.kind === 'comment' && (
            <form
              className="mt-4 space-y-3"
              onSubmit={(event) => {
                event.preventDefault()
                const value = comment.trim()
                if (!value) return
                void submit('submit_output', { [schema.outputKey]: value })
              }}
            >
              <textarea
                aria-label={t('taskCenter.detail.response', 'Response')}
                value={comment}
                onChange={(event) => setComment(event.currentTarget.value)}
                disabled={isSubmitting || Boolean(submittedKind)}
                placeholder={schema.placeholder}
                rows={4}
                className="w-full resize-none rounded-xl px-3 py-2 text-sm outline-none disabled:opacity-50"
                style={{
                  background: 'var(--cp-surface)',
                  border: '1px solid var(--cp-border)',
                  color: 'var(--cp-text)',
                }}
              />
              <button
                type="submit"
                disabled={isSubmitting || Boolean(submittedKind) || !comment.trim()}
                className="inline-flex items-center gap-2 rounded-lg px-3 py-2 text-xs font-medium transition-colors disabled:opacity-50"
                style={{
                  background: 'var(--cp-accent)',
                  color: 'white',
                  border: '1px solid var(--cp-accent)',
                }}
              >
                <Send size={13} />
                {schema.submitLabel}
              </button>
            </form>
          )}

          {error && (
            <div className="mt-3 text-xs" style={{ color: 'var(--cp-danger)' }}>
              {error}
            </div>
          )}
        </div>
      </div>
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
    task.status === 'failed' && task.payload?.error
      ? String(task.payload.error)
      : null
  const interactionSchema = getTaskInteractionSchema(task)

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

      {interactionSchema && (
        <TaskInteractionPanel
          task={task}
          schema={interactionSchema}
          t={t}
          onSubmit={(action, payload) => store.submitTaskHumanAction(task.taskId, action, payload)}
        />
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
