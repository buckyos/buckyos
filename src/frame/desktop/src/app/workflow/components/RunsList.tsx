/* ── Workflow Run list. Rows open the run detail page; TaskMgr holds the control actions. ── */

import { ExternalLink } from 'lucide-react'
import type { WorkflowRunSummary } from '../types'
import { formatTime, runStatusStyle } from './run-status'

export function RunsList({
  runs,
  onOpen,
}: {
  runs: WorkflowRunSummary[]
  onOpen: (runId: string) => void
}) {
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
        No runs visible to you for this definition.
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
        className="grid grid-cols-[150px_120px_90px_1fr_140px_60px_70px] gap-3 px-3 py-2 text-[10px] uppercase tracking-wider"
        style={{
          color: 'var(--cp-muted)',
          background: 'var(--cp-surface-2)',
          borderBottom: '1px solid var(--cp-border)',
        }}
      >
        <span>Run ID</span>
        <span>Status</span>
        <span>Revision</span>
        <span>Requested by</span>
        <span>Updated</span>
        <span>Progress</span>
        <span className="text-right">Open</span>
      </div>
      <div className="divide-y" style={{ borderColor: 'var(--cp-border)' }}>
        {runs.map((r) => {
          const sty = runStatusStyle[r.status]
          return (
            <div
              key={r.runId}
              className="grid grid-cols-[150px_120px_90px_1fr_140px_60px_70px] items-center gap-3 px-3 py-2 text-[12px]"
              style={{
                color: 'var(--cp-text)',
                background:
                  r.status === 'waiting_human'
                    ? 'color-mix(in srgb, var(--cp-warning) 8%, transparent)'
                    : 'transparent',
                borderColor: 'var(--cp-border)',
              }}
            >
              <span className="truncate font-mono text-[11px]" style={{ color: 'var(--cp-muted)' }} title={r.runId}>
                {r.runId}
              </span>
              <span className="inline-flex items-center gap-1 text-[11px] font-medium" style={{ color: sty.color }}>
                <span className="h-1.5 w-1.5 rounded-full" style={{ background: sty.color }} />
                {sty.label}
              </span>
              <span className="truncate font-mono text-[10px]" style={{ color: 'var(--cp-muted)' }} title={r.revision}>
                {r.revision.slice(0, 10)}
              </span>
              <span className="truncate text-[11px]" style={{ color: 'var(--cp-muted)' }}>
                {r.requestedBy ?? `${r.owner.userId}/${r.owner.appId}`}
              </span>
              <span className="text-[11px]" style={{ color: 'var(--cp-muted)' }}>
                {formatTime(r.updatedAt)}
              </span>
              <span className="text-[11px]" style={{ color: 'var(--cp-muted)' }}>
                {Math.round(r.progress)}%
              </span>
              <button
                type="button"
                onClick={() => onOpen(r.runId)}
                className="ml-auto inline-flex items-center gap-1 rounded px-2 py-1 text-[11px]"
                style={{
                  color: 'var(--cp-accent)',
                  background: 'var(--cp-surface-2)',
                  border: '1px solid var(--cp-border)',
                }}
              >
                Detail <ExternalLink size={11} />
              </button>
            </div>
          )
        })}
      </div>
    </div>
  )
}
