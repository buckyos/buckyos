/* ── TaskCenter task detail: the side panel on a desktop, its own page on a phone ── */

import { useEffect, useMemo, useState } from 'react'
import { ArrowLeft, CircleStop, MessageSquare, X } from 'lucide-react'
import { useI18n } from '../../../i18n/provider'
import { useTaskCenterStore } from '../hooks/use-task-center-store'
import { notifyCardPath, type Task } from '../../../api/task_mgr'
import { ContentPreview } from '../../../components/ContentPreview'
import { formatBytes } from '../../../sysdlg/shareFormat'
import { NAV_LABELS, type TaskCenterNav, type TaskCenterPage } from '../components/layout/navigation'
import { Meter, StatusDot } from '../components/taskView'
import { taskTone, useTaskText, type TaskText } from '../components/taskModel'

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
    <section className="tc-section min-w-0" data-testid="task-results">
      <h3 className="tc-heading">{t('taskCenter.detail.results', 'Results')}</h3>
      <div className="space-y-2">
        {artifacts.map((artifact, index) => (
          <div key={`${artifact.source.objectId}:${index}`} className="flex min-w-0 items-center gap-3">
            <div className="min-w-0 flex-1">
              <p className="break-all text-sm">{artifact.name}</p>
              <p className="tc-muted break-all text-xs">
                {[artifact.mime, artifact.size === undefined ? undefined : formatBytes(artifact.size)].filter(Boolean).join(' · ')}
              </p>
            </div>
            <button
              type="button"
              className="tc-btn"
              aria-pressed={activeObjectId === artifact.source.objectId}
              onClick={() => setActiveObjectId(artifact.source.objectId)}
              data-testid="task-result-preview"
            >
              {t('taskCenter.detail.previewResult', 'Preview')}
            </button>
          </div>
        ))}
      </div>
      {active && (
        <div className="mt-3 overflow-hidden rounded-lg" style={{ height: 'clamp(280px, 55vh, 560px)' }}>
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

/** The run: one progress readout above the subtasks, drawn as dots on a line. */
function TaskRun({ task, text, onOpen }: { task: Task; text: TaskText; onOpen: (taskId: string) => void }) {
  const total = task.children.length
  if (task.progress == null && total === 0) return null
  const done = task.children.filter((child) => child.status === 'completed').length
  const tone = taskTone(task)

  return (
    <section className="tc-section" data-testid="task-run">
      {task.progress != null && (
        <div className={total > 0 ? 'mb-4' : undefined}>
          <div className="mb-2 flex items-baseline justify-between gap-3">
            <span className="tc-num text-[28px] font-medium leading-8">
              {task.progress}
              <span className="tc-muted text-base">%</span>
            </span>
            {total > 0 && (
              <span className="tc-muted text-xs">
                {text.t('taskCenter.detail.subtasksDone', '{{done}} of {{total}} subtasks done', { done, total })}
              </span>
            )}
          </div>
          <Meter value={task.progress} tone={tone} size="lg" />
        </div>
      )}
      {total > 0 && (
        <>
          <h3 className="tc-heading mb-1">
            {text.t('taskCenter.detail.subtasks', 'Subtasks')}
            <span className="tc-num">{total}</span>
          </h3>
          <div className="-mx-2">
            {task.children.map((child) => (
              <button
                key={child.taskId}
                type="button"
                className="tc-step"
                onClick={() => onOpen(child.taskId)}
                data-testid="task-step"
              >
                <span className="tc-row-dot">
                  <StatusDot tone={taskTone(child)} />
                </span>
                <span className="tc-row-body">
                  <span className="tc-row-title">{child.title}</span>
                  <span className="tc-row-meta">{text.meta(child)}</span>
                </span>
                <span className="tc-row-aside">
                  <span>
                    {child.progress != null ? `${child.progress}%` : text.listTime(child.updatedAt)}
                  </span>
                </span>
              </button>
            ))}
          </div>
        </>
      )}
    </section>
  )
}

interface TaskDetailPageProps {
  taskId: string
  /** The page the task was opened from. */
  page: TaskCenterPage
  variant: 'panel' | 'page'
  onNavigate: (nav: TaskCenterNav) => void
  onClose: () => void
}

export function TaskDetailPage({ taskId, page, variant, onNavigate, onClose }: TaskDetailPageProps) {
  const store = useTaskCenterStore()
  const text = useTaskText()
  const { t } = text
  const [copyState, setCopyState] = useState('')
  const [canceling, setCanceling] = useState(false)
  const [cancelError, setCancelError] = useState<string | null>(null)
  // Lists carry metadata only; the payload/progress of a finished task is
  // fetched when its detail opens.
  useEffect(() => {
    void store.loadTaskDetail(taskId)
  }, [store, taskId])
  const task = store.getTaskById(taskId)
  const parent = task?.parentTaskId ? store.getTaskById(task.parentTaskId) : null
  const open = (id: string) => onNavigate({ page, taskId: id })

  const head = variant === 'panel' ? (
    <div className="tc-panel-head">
      {task && <StatusDot tone={taskTone(task)} />}
      <span className="tc-muted min-w-0 flex-1 truncate text-[13px]">{task ? text.status(task) : ''}</span>
      <button
        type="button"
        className="tc-icon-btn"
        data-size="sm"
        onClick={onClose}
        aria-label={t('taskCenter.detail.close', 'Close details')}
        title={t('taskCenter.detail.close', 'Close details')}
      >
        <X size={16} strokeWidth={1.75} />
      </button>
    </div>
  ) : (
    <button
      type="button"
      onClick={onClose}
      className="-ml-2 mb-3 inline-flex h-9 items-center gap-2 rounded-lg px-2 text-sm"
    >
      <ArrowLeft size={16} strokeWidth={1.75} />
      {t(NAV_LABELS[page].key, NAV_LABELS[page].fallback)}
    </button>
  )

  const frame = (body: React.ReactNode) =>
    variant === 'panel' ? (
      <>
        {head}
        <div className="desktop-scrollbar min-h-0 flex-1 overflow-y-auto px-5 pb-8 pt-4" data-testid="task-detail">
          {body}
        </div>
      </>
    ) : (
      <div data-testid="task-detail">
        {head}
        {body}
      </div>
    )

  if (!task) {
    return frame(
      <div className="tc-empty">
        <p>{t('taskCenter.detail.notFound', 'Task not found')}</p>
        <p className="tc-mono mt-1 text-xs">{taskId}</p>
      </div>,
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

  return frame(
    <>
      <header className="tc-section">
        {parent && (
          <button
            type="button"
            onClick={() => open(parent.taskId)}
            className="tc-muted mb-1 flex max-w-full items-baseline gap-1 text-xs hover:text-[color:var(--cp-text)]"
            data-testid="task-parent"
          >
            <span className="shrink-0">{t('taskCenter.detail.partOf', 'Part of')}</span>
            <span className="truncate underline decoration-[color:var(--cp-border)] underline-offset-2">{parent.title}</span>
          </button>
        )}
        <h2 className="break-words text-[17px] font-medium leading-6">{task.title}</h2>
        {variant === 'page' && (
          <p className="tc-muted mt-1 flex items-center gap-2 text-[13px]">
            <StatusDot tone={taskTone(task)} />
            {text.status(task)}
          </p>
        )}
        {task.summary && <p className="tc-muted mt-1 text-sm">{task.summary}</p>}
        {task.cancellable && (
          <button
            type="button"
            onClick={() => void cancel()}
            disabled={canceling}
            className="tc-btn mt-3"
            data-variant="danger"
            data-testid="task-cancel"
          >
            <CircleStop size={14} strokeWidth={1.75} />
            {t('taskCenter.detail.cancel', 'Cancel task')}
          </button>
        )}
        {task.pendingControlAction === 'Cancel' && task.status !== 'cancelled' && (
          <p className="tc-muted mt-2 text-xs">{t('taskCenter.detail.canceling', 'Canceling…')}</p>
        )}
        {cancelError && <p className="tc-danger mt-2 text-xs">{cancelError}</p>}
      </header>

      <TaskRun task={task} text={text} onOpen={open} />

      <TaskResults key={task.taskId} task={task} />

      {task.download && (
        <section className="tc-section" data-testid="task-download-info">
          <h3 className="tc-heading">{t('taskCenter.download.section', 'Download')}</h3>
          <dl className="tc-dl">
            <dt>{t('taskCenter.download.url', 'Link')}</dt>
            <dd>{task.download.url}</dd>
            {task.download.localPath && (
              <>
                <dt>{t('taskCenter.download.savedTo', 'Saved to')}</dt>
                <dd className="tc-mono" data-testid="task-download-path">{task.download.localPath}</dd>
              </>
            )}
          </dl>
        </section>
      )}

      {/* The task of a System Notify is answered on its MessageHub card */}
      {task.notifyId && (
        <div className="tc-section tc-note" data-testid="task-notify">
          <p>{t('taskCenter.notify.hint', 'This task answers a notification. Handle it on its card in MessageHub.')}</p>
          <a href={notifyCardPath(task.notifyId)} className="tc-link mt-2 text-[13px]" data-testid="task-notify-link">
            <MessageSquare size={14} strokeWidth={1.75} />
            {t('taskCenter.notify.open', 'Handle in MessageHub')}
          </a>
        </div>
      )}

      {errorMsg && (
        <div className="tc-section tc-note" data-tone="failed">
          {errorMsg}
        </div>
      )}

      <section className="tc-section">
        <h3 className="tc-heading">{t('taskCenter.detail.info', 'Details')}</h3>
        <dl className="tc-dl">
          <dt>{t('taskCenter.detail.taskId', 'Task ID')}</dt>
          <dd className="tc-mono">{task.taskId}</dd>
          {task.rootTaskId !== task.taskId && (
            <>
              <dt>{t('taskCenter.detail.rootTaskId', 'Root task')}</dt>
              <dd className="tc-mono">{task.rootTaskId}</dd>
            </>
          )}
          <dt>{t('taskCenter.detail.type', 'Type')}</dt>
          <dd>{text.type(task.type)}</dd>
          <dt>{t('taskCenter.detail.source', 'Source')}</dt>
          <dd>{text.source(task.source)}</dd>
          <dt>{t('taskCenter.detail.created', 'Created')}</dt>
          <dd className="tc-num">{text.fullTime(task.createdAt)}</dd>
          <dt>{t('taskCenter.detail.started', 'Started')}</dt>
          <dd className="tc-num">{text.fullTime(task.startedAt)}</dd>
          <dt>{t('taskCenter.detail.ended', 'Ended')}</dt>
          <dd className="tc-num">{text.fullTime(task.endedAt)}</dd>
          <dt>{t('taskCenter.detail.updated', 'Updated')}</dt>
          <dd className="tc-num">{text.fullTime(task.updatedAt)}</dd>
          {task.schemaType && (
            <>
              <dt>{t('taskCenter.detail.schema', 'Schema')}</dt>
              <dd className="tc-mono">{task.schemaType}</dd>
            </>
          )}
        </dl>
      </section>

      {task.aiccRequest && Object.keys(task.aiccRequest.params).length > 0 && (
        <section className="tc-section min-w-0">
          <div className="mb-2 flex items-center justify-between gap-2">
            <h3 className="tc-heading">{t('aiCenter.playground.taskRequest')}</h3>
            <button
              type="button"
              className="tc-link text-[13px]"
              onClick={async () => {
                try {
                  await navigator.clipboard.writeText(JSON.stringify(task.aiccRequest, null, 2))
                  setCopyState('common.copied')
                } catch {
                  setCopyState('aiCenter.playground.copyFailed')
                }
              }}
            >
              {t(copyState || 'aiCenter.playground.copyRequest')}
            </button>
          </div>
          <p className="tc-muted mb-2 text-xs">{t('aiCenter.playground.taskRequestHint')}</p>
          <pre className="tc-code">{JSON.stringify(task.aiccRequest, null, 2)}</pre>
        </section>
      )}

      {!task.aiccRequest && task.aiccRequestSummary && (
        <section className="tc-section min-w-0">
          <h3 className="tc-heading">{t('aiCenter.playground.taskRequestSummary')}</h3>
          <p className="tc-muted mb-2 text-xs">
            {t('aiCenter.playground.taskRequestTooLarge', undefined, {
              size: formatBytes(task.aiccRequestSummary.bytes),
              traceId: task.aiccRequestSummary.traceId ?? '-',
            })}
          </p>
          <pre className="tc-code">{JSON.stringify(task.aiccRequestSummary, null, 2)}</pre>
        </section>
      )}

      {Object.keys(task.payload).length > 0 && (
        <section className="tc-section min-w-0">
          <h3 className="tc-heading">{t('taskCenter.detail.payload', 'Extended data')}</h3>
          <pre className="tc-code">{JSON.stringify(task.payload, null, 2)}</pre>
        </section>
      )}
    </>,
  )
}
