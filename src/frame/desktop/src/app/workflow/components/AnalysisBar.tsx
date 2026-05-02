/* ── Static analysis summary bar ── */

import { useState } from 'react'
import { AlertTriangle, ChevronDown, ChevronUp, Info } from 'lucide-react'
import type { AnalysisReport, AnalysisIssue } from '../mock/types'

function severityColor(s: AnalysisIssue['severity']) {
  if (s === 'error') return 'var(--cp-danger)'
  if (s === 'warn') return 'var(--cp-warning)'
  return 'var(--cp-muted)'
}

export function AnalysisBar({
  report,
  onLocateNode,
}: {
  report: AnalysisReport
  onLocateNode?: (nodeId: string) => void
}) {
  const [open, setOpen] = useState(report.errorCount > 0)
  const total = report.errorCount + report.warnCount + report.infoCount
  return (
    <div
      className="rounded-lg"
      style={{
        background: 'var(--cp-surface)',
        border:
          '1px solid ' +
          (report.errorCount > 0
            ? 'var(--cp-danger)'
            : report.warnCount > 0
              ? 'var(--cp-warning)'
              : 'var(--cp-border)'),
      }}
    >
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center gap-3 px-3 py-2 text-left"
      >
        <span
          className="flex items-center gap-1 text-xs"
          style={{ color: 'var(--cp-danger)' }}
        >
          <AlertTriangle size={13} /> {report.errorCount} errors
        </span>
        <span
          className="flex items-center gap-1 text-xs"
          style={{ color: 'var(--cp-warning)' }}
        >
          <AlertTriangle size={13} /> {report.warnCount} warns
        </span>
        <span
          className="flex items-center gap-1 text-xs"
          style={{ color: 'var(--cp-muted)' }}
        >
          <Info size={13} /> {report.infoCount} info
        </span>
        <span
          className="ml-auto text-[10px] uppercase tracking-wider"
          style={{ color: 'var(--cp-muted)' }}
        >
          Static analysis
        </span>
        {total > 0 &&
          (open ? <ChevronUp size={14} /> : <ChevronDown size={14} />)}
      </button>
      {open && total > 0 && (
        <div
          className="space-y-1 px-3 pb-3 pt-1"
          style={{ borderTop: '1px solid var(--cp-border)' }}
        >
          {report.issues.map((iss, i) => (
            <button
              key={i}
              type="button"
              onClick={() => iss.nodeId && onLocateNode?.(iss.nodeId)}
              disabled={!iss.nodeId}
              className="flex w-full items-start gap-2 rounded px-2 py-1 text-left text-[11px]"
              style={{ color: 'var(--cp-text)' }}
            >
              <span
                className="rounded px-1 text-[9px] font-semibold uppercase"
                style={{
                  background: 'var(--cp-surface-2)',
                  color: severityColor(iss.severity),
                  border: '1px solid var(--cp-border)',
                }}
              >
                {iss.severity}
              </span>
              <span className="font-mono text-[10px]" style={{ color: 'var(--cp-muted)' }}>
                {iss.code}
              </span>
              <span className="flex-1">{iss.message}</span>
              {iss.nodeId && (
                <span style={{ color: 'var(--cp-accent)' }}>@{iss.nodeId}</span>
              )}
            </button>
          ))}
        </div>
      )}
    </div>
  )
}
