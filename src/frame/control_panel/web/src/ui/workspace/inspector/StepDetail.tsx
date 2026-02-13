import { useCallback, useEffect, useState } from 'react'

import { fetchWsTasks, fetchWorkLogs } from '@/api/workspace'
import Icon from '../../icons'
import StatusPill from '../components/StatusPill'
import JsonViewer from '../components/JsonViewer'

const formatDuration = (seconds?: number): string => {
  if (seconds == null) return '—'
  if (seconds < 60) return `${seconds}s`
  const m = Math.floor(seconds / 60)
  const s = seconds % 60
  return `${m}m ${s}s`
}

type StepDetailProps = {
  step: WsStep
}

const StepDetail = ({ step }: StepDetailProps) => {
  const [tasks, setTasks] = useState<WsTask[]>([])
  const [logs, setLogs] = useState<WsWorkLog[]>([])

  const loadData = useCallback(async () => {
    // We need the run_id; for simplicity, load using step-level filters
    // In a real implementation, we'd have run_id from context
    const [taskRes, logRes] = await Promise.all([
      fetchWsTasks('run-001', { stepId: step.step_id }),
      fetchWorkLogs('run-001', { stepId: step.step_id }),
    ])
    if (taskRes.data) setTasks(taskRes.data)
    if (logRes.data) setLogs(logRes.data)
  }, [step.step_id])

  useEffect(() => {
    loadData()
  }, [loadData])

  return (
    <div className="space-y-4">
      {/* Summary */}
      <div>
        <div className="flex items-center gap-2">
          <span className="text-sm font-semibold text-[var(--cp-ink)]">
            Step #{step.step_index}
          </span>
          <StatusPill status={step.status} />
        </div>
        {step.title && (
          <p className="mt-1 text-sm text-[var(--cp-ink)]">{step.title}</p>
        )}
      </div>

      {/* Metadata */}
      <div className="grid grid-cols-2 gap-2 text-xs">
        <div className="rounded-lg bg-[var(--cp-surface-muted)] px-3 py-2">
          <p className="text-[var(--cp-muted)]">Duration</p>
          <p className="font-medium text-[var(--cp-ink)]">{formatDuration(step.duration)}</p>
        </div>
        <div className="rounded-lg bg-[var(--cp-surface-muted)] px-3 py-2">
          <p className="text-[var(--cp-muted)]">Tasks</p>
          <p className="font-medium text-[var(--cp-ink)]">{step.task_count}</p>
        </div>
      </div>

      {/* Log counts */}
      <div className="flex flex-wrap gap-1.5">
        {step.log_counts.message > 0 && (
          <span className="cp-pill border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] text-[var(--cp-muted)]">
            <Icon name="message" className="size-3" /> {step.log_counts.message} msgs
          </span>
        )}
        {step.log_counts.function_call > 0 && (
          <span className="cp-pill border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] text-[var(--cp-muted)]">
            <Icon name="function" className="size-3" /> {step.log_counts.function_call} calls
          </span>
        )}
        {step.log_counts.action > 0 && (
          <span className="cp-pill border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] text-[var(--cp-muted)]">
            <Icon name="action" className="size-3" /> {step.log_counts.action} actions
          </span>
        )}
        {step.log_counts.sub_agent > 0 && (
          <span className="cp-pill border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] text-[var(--cp-muted)]">
            <Icon name="branch" className="size-3" /> {step.log_counts.sub_agent} sub-agent
          </span>
        )}
      </div>

      {/* Tasks */}
      {tasks.length > 0 && (
        <div>
          <h4 className="mb-2 text-xs font-semibold uppercase tracking-wide text-[var(--cp-muted)]">
            Tasks
          </h4>
          <div className="space-y-1.5">
            {tasks.map((task) => (
              <div
                key={task.task_id}
                className="rounded-lg border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2 text-xs"
              >
                <div className="flex items-center gap-2">
                  <StatusPill status={task.status} />
                  <span className="text-[var(--cp-muted)]">{task.model}</span>
                  {task.duration != null && (
                    <span className="text-[var(--cp-muted)]">{task.duration}s</span>
                  )}
                </div>
                <p className="mt-1 truncate text-[var(--cp-ink)]">{task.prompt_preview}</p>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Key Logs */}
      {logs.length > 0 && (
        <div>
          <h4 className="mb-2 text-xs font-semibold uppercase tracking-wide text-[var(--cp-muted)]">
            Key Logs
          </h4>
          <div className="space-y-1.5">
            {logs
              .filter((l) => l.status === 'failed' || l.status === 'partial')
              .concat(logs.filter((l) => l.status !== 'failed' && l.status !== 'partial'))
              .slice(0, 5)
              .map((log) => (
                <div
                  key={log.log_id}
                  className={`rounded-lg border px-3 py-2 text-xs ${
                    log.status === 'failed'
                      ? 'border-rose-200 bg-rose-50'
                      : 'border-[var(--cp-border)] bg-[var(--cp-surface-muted)]'
                  }`}
                >
                  <div className="flex items-center gap-2">
                    <StatusPill status={log.status} />
                    <span className="text-[var(--cp-muted)]">{log.type.replace(/_/g, ' ')}</span>
                  </div>
                  <p className="mt-1 truncate text-[var(--cp-ink)]">{log.summary}</p>
                </div>
              ))}
          </div>
        </div>
      )}

      {/* Output snapshot */}
      {step.output_snapshot && (
        <JsonViewer label="Output Snapshot" data={step.output_snapshot} />
      )}

      <div className="text-[10px] text-[var(--cp-muted)]">ID: {step.step_id}</div>
    </div>
  )
}

export default StepDetail
