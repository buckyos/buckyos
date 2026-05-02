/* ── Workflow Run list (mount-point view only). Each row deep-links to TaskMgr UI. ── */

import { ExternalLink, UserCheck } from 'lucide-react'
import type { WorkflowRunSummary } from '../mock/types'

const statusStyle: Record<
  WorkflowRunSummary['status'],
  { label: string; color: string }
> = {
  created: { label: 'Created', color: 'var(--cp-muted)' },
  running: { label: 'Running', color: 'var(--cp-accent)' },
  waiting_human: { label: 'Waiting Human', color: 'var(--cp-warning)' },
  completed: { label: 'Completed', color: 'var(--cp-success)' },
  failed: { label: 'Failed', color: 'var(--cp-danger)' },
  paused: { label: 'Paused', color: 'var(--cp-warning)' },
  aborted: { label: 'Aborted', color: 'var(--cp-muted)' },
  budget_exhausted: { label: 'Budget Exhausted', color: 'var(--cp-danger)' },
}

function formatDuration(ms: number | undefined) {
  if (ms == null) return '—'
  if (ms < 1000) return `${ms}ms`
  if (ms < 60_000) return `${Math.round(ms / 1000)}s`
  if (ms < 3_600_000) return `${Math.round(ms / 60_000)}m`
  return `${(ms / 3_600_000).toFixed(1)}h`
}

function formatTime(iso: string) {
  return new Date(iso).toLocaleString(undefined, {
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  })
}

export function RunsList({ runs }: { runs: WorkflowRunSummary[] }) {
  if (runs.length === 0) {
    return (
      <div
        className="rounded-lg p-6 text-center text-xs"
        style={{
          background: 'var(--cp-surface)',
          border: '1px dashed var(--cp-border)',
          color: 'var(--cp-muted)',
        }}
      >
        No recent runs for this mount point.
      </div>
    )
  }

  return (
    <div
      className="overflow-hidden rounded-lg"
      style={{
        background: 'var(--cp-surface)',
        border: '1px solid var(--cp-border)',
      }}
    >
      <div
        className="grid grid-cols-[110px_120px_90px_140px_80px_1fr_80px] gap-3 px-3 py-2 text-[10px] uppercase tracking-wider"
        style={{
          color: 'var(--cp-muted)',
          background: 'var(--cp-surface-2)',
          borderBottom: '1px solid var(--cp-border)',
        }}
      >
        <span>Run ID</span>
        <span>Status</span>
        <span>Trigger</span>
        <span>Started</span>
        <span>Duration</span>
        <span>Notes</span>
        <span className="text-right">Open</span>
      </div>
      <div className="divide-y" style={{ borderColor: 'var(--cp-border)' }}>
        {runs.map((r) => {
          const sty = statusStyle[r.status]
          const waiting = r.humanWaitingNodes.length > 0
          return (
            <div
              key={r.runId}
              className="grid grid-cols-[110px_120px_90px_140px_80px_1fr_80px] items-center gap-3 px-3 py-2 text-[12px]"
              style={{
                color: 'var(--cp-text)',
                background: waiting
                  ? 'color-mix(in srgb, var(--cp-warning) 8%, transparent)'
                  : 'transparent',
                borderColor: 'var(--cp-border)',
              }}
            >
              <span
                className="truncate font-mono text-[11px]"
                style={{ color: 'var(--cp-muted)' }}
                title={r.runId}
              >
                {r.runId}
              </span>
              <span
                className="inline-flex items-center gap-1 text-[11px] font-medium"
                style={{ color: sty.color }}
              >
                <span
                  className="h-1.5 w-1.5 rounded-full"
                  style={{ background: sty.color }}
                />
                {sty.label}
                {r.planVersion > 1 && (
                  <span
                    className="ml-1 rounded px-1 text-[9px]"
                    style={{
                      color: 'var(--cp-warning)',
                      background: 'var(--cp-surface-2)',
                      border: '1px solid var(--cp-border)',
                    }}
                    title="Plan version after Amendment"
                  >
                    pv{r.planVersion}
                  </span>
                )}
              </span>
              <span className="text-[11px]" style={{ color: 'var(--cp-muted)' }}>
                {r.triggerSource}
              </span>
              <span className="text-[11px]" style={{ color: 'var(--cp-muted)' }}>
                {formatTime(r.startedAt)}
              </span>
              <span className="text-[11px]" style={{ color: 'var(--cp-muted)' }}>
                {formatDuration(r.durationMs)}
              </span>
              <span className="truncate text-[11px]" style={{ color: 'var(--cp-muted)' }}>
                {waiting && (
                  <span
                    className="mr-2 inline-flex items-center gap-1"
                    style={{ color: 'var(--cp-warning)' }}
                  >
                    <UserCheck size={11} /> {r.humanWaitingNodes.join(', ')}
                  </span>
                )}
                {r.errorSummary && (
                  <span style={{ color: 'var(--cp-danger)' }}>
                    {r.errorSummary}
                  </span>
                )}
              </span>
              <a
                href={`/${r.taskmgrUrl}`}
                target="_blank"
                rel="noreferrer"
                className="ml-auto inline-flex items-center gap-1 rounded px-2 py-1 text-[11px]"
                style={{
                  color: 'var(--cp-accent)',
                  background: 'var(--cp-surface-2)',
                  border: '1px solid var(--cp-border)',
                }}
              >
                TaskMgr <ExternalLink size={11} />
              </a>
            </div>
          )
        })}
      </div>
    </div>
  )
}
