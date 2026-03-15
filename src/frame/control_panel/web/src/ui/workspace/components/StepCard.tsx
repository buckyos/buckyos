import { useState } from 'react'

import Icon from '../../icons'
import { useWorkspace } from '../WorkspaceContext'
import StatusPill from './StatusPill'
import CountBadge from './CountBadge'

type StepCardProps = {
  step: WsStep
  isCurrent: boolean
}

const formatDuration = (seconds?: number): string => {
  if (seconds == null) return '—'
  if (seconds < 60) return `${seconds}s`
  const m = Math.floor(seconds / 60)
  const s = seconds % 60
  return `${m}m ${s}s`
}

const StepCard = ({ step, isCurrent }: StepCardProps) => {
  const { openInspector, navigateToWorkLog } = useWorkspace()
  const [expanded, setExpanded] = useState(isCurrent)

  return (
    <div
      className={`rounded-2xl border transition ${
        isCurrent
          ? 'border-[var(--cp-primary)] bg-[var(--cp-primary-soft)]/30 shadow-md shadow-emerald-100'
          : 'border-[var(--cp-border)] bg-white'
      }`}
    >
      {/* Header - always visible */}
      <button
        type="button"
        onClick={() => setExpanded((p) => !p)}
        className="flex w-full items-center gap-3 px-4 py-3 text-left"
      >
        {/* Timeline dot */}
        <div className="flex flex-col items-center self-stretch">
          <span
            className={`mt-1 inline-flex size-3 flex-none rounded-full ${
              step.status === 'running'
                ? 'animate-pulse bg-[var(--cp-primary)]'
                : step.status === 'success'
                  ? 'bg-[var(--cp-success)]'
                  : step.status === 'failed'
                    ? 'bg-[var(--cp-danger)]'
                    : 'bg-slate-300'
            }`}
          />
          <span className="mt-1 flex-1 w-px bg-[var(--cp-border)]" />
        </div>

        {/* Content */}
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-xs font-semibold text-[var(--cp-muted)]">
              Step {step.step_index}
            </span>
            {step.title && (
              <span className="text-sm font-medium text-[var(--cp-ink)]">{step.title}</span>
            )}
            <StatusPill status={step.status} />
            {step.duration != null && (
              <span className="text-xs text-[var(--cp-muted)]">{formatDuration(step.duration)}</span>
            )}
          </div>

          {/* Count badges */}
          <div className="mt-2 flex flex-wrap gap-1">
            {step.task_count > 0 && (
              <CountBadge icon="spark" label="Tasks" count={step.task_count} />
            )}
            {step.log_counts.message > 0 && (
              <CountBadge
                icon="message"
                label="Messages"
                count={step.log_counts.message}
                onClick={() =>
                  navigateToWorkLog({ stepId: step.step_id, type: 'message_sent' })
                }
              />
            )}
            {step.log_counts.function_call > 0 && (
              <CountBadge
                icon="function"
                label="Function Calls"
                count={step.log_counts.function_call}
                onClick={() =>
                  navigateToWorkLog({ stepId: step.step_id, type: 'function_call' })
                }
              />
            )}
            {step.log_counts.action > 0 && (
              <CountBadge
                icon="action"
                label="Actions"
                count={step.log_counts.action}
                onClick={() =>
                  navigateToWorkLog({ stepId: step.step_id, type: 'action' })
                }
              />
            )}
            {step.log_counts.sub_agent > 0 && (
              <CountBadge
                icon="branch"
                label="Sub-Agent Events"
                count={step.log_counts.sub_agent}
              />
            )}
          </div>
        </div>

        {/* Expand arrow */}
        <Icon
          name={expanded ? 'chevron-down' : 'chevron-right'}
          className="size-4 flex-none text-[var(--cp-muted)]"
        />
      </button>

      {/* Expanded content */}
      {expanded && (
        <div className="border-t border-[var(--cp-border)] px-4 py-3">
          <div className="flex items-center justify-between">
            <p className="text-xs text-[var(--cp-muted)]">
              {step.started_at
                ? `Started ${new Date(step.started_at).toLocaleTimeString()}`
                : 'Not started'}
              {step.ended_at && ` · Ended ${new Date(step.ended_at).toLocaleTimeString()}`}
            </p>
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation()
                openInspector({ kind: 'step', data: step })
              }}
              className="rounded-lg px-2 py-1 text-xs font-medium text-[var(--cp-primary)] transition hover:bg-[var(--cp-primary-soft)]"
            >
              View Details
            </button>
          </div>
        </div>
      )}
    </div>
  )
}

export default StepCard
