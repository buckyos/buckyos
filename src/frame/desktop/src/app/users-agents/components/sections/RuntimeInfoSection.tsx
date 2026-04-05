/* ── Agent runtime info section ── */

import { Activity } from 'lucide-react'
import { MetricCard } from '../../../../components/AppPanelPrimitives'

interface RuntimeInfoSectionProps {
  runtime: {
    uptime: string
    memoryUsage: string
    cpuUsage: string
    lastActive: string
    uiSessions: number
    workSessions: number
    workspaces: number
  }
  status: 'running' | 'stopped' | 'error'
}

export function RuntimeInfoSection({ runtime, status }: RuntimeInfoSectionProps) {
  return (
    <div
      className="rounded-[22px] px-5 py-4"
      style={{
        background: 'color-mix(in srgb, var(--cp-surface-2) 40%, var(--cp-surface))',
        border: '1px solid color-mix(in srgb, var(--cp-border) 50%, transparent)',
      }}
    >
      <div className="flex items-center gap-2 mb-3">
        <Activity size={16} style={{ color: 'var(--cp-accent)' }} />
        <h3
          className="font-display text-sm font-semibold"
          style={{ color: 'var(--cp-text)' }}
        >
          Runtime & Work
        </h3>
      </div>

      <div className="grid gap-2 grid-cols-2 sm:grid-cols-3">
        <MetricCard
          label="Status"
          tone={status === 'running' ? 'success' : status === 'error' ? 'warning' : 'neutral'}
          value={status}
        />
        <MetricCard label="Uptime" tone="accent" value={runtime.uptime} />
        <MetricCard label="CPU" tone="neutral" value={runtime.cpuUsage} />
        <MetricCard label="Memory" tone="neutral" value={runtime.memoryUsage} />
        <MetricCard label="UI Sessions" tone="accent" value={String(runtime.uiSessions)} />
        <MetricCard label="Workspaces" tone="accent" value={String(runtime.workspaces)} />
      </div>

      <div className="mt-3 space-y-1.5">
        <div className="flex items-baseline gap-3">
          <span className="text-[12px] font-medium w-28 shrink-0" style={{ color: 'var(--cp-muted)' }}>
            Work sessions
          </span>
          <span className="text-sm" style={{ color: 'var(--cp-text)' }}>{runtime.workSessions}</span>
        </div>
        <div className="flex items-baseline gap-3">
          <span className="text-[12px] font-medium w-28 shrink-0" style={{ color: 'var(--cp-muted)' }}>
            Last active
          </span>
          <span className="text-sm" style={{ color: 'var(--cp-text)' }}>
            {new Date(runtime.lastActive).toLocaleString()}
          </span>
        </div>
      </div>
    </div>
  )
}
