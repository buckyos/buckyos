import LineAreaChart from '../charts/LineAreaChart'

type NetworkOverviewPanelProps = {
  overview: NetworkOverview | null
  loading?: boolean
  errorMessage?: string | null
  compact?: boolean
}

const formatBytes = (value: number) => {
  if (!Number.isFinite(value) || value <= 0) {
    return '0 B'
  }
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  const index = Math.min(Math.floor(Math.log(value) / Math.log(1024)), units.length - 1)
  const scaled = value / 1024 ** index
  return `${scaled.toFixed(scaled >= 100 || index === 0 ? 0 : 1)} ${units[index]}`
}

const formatRate = (value: number) => `${formatBytes(value)}/s`

const getTimelineTicks = (points: NetworkPoint[], maxTicks: number) => {
  if (!points.length) {
    return [] as string[]
  }
  if (points.length <= maxTicks) {
    return points.map((point) => point.time)
  }

  const step = Math.max(1, Math.floor(points.length / Math.max(1, maxTicks - 1)))
  const values: string[] = []
  for (let index = 0; index < points.length; index += step) {
    values.push(points[index].time)
  }

  const last = points.at(-1)?.time
  if (last && values[values.length - 1] !== last) {
    values.push(last)
  }

  return values
}

const NetworkOverviewPanel = ({
  overview,
  loading = false,
  errorMessage,
  compact = false,
}: NetworkOverviewPanelProps) => {
  if (loading) {
    return (
      <div className="space-y-3">
        <div className="h-20 animate-pulse rounded-2xl bg-[var(--cp-surface-muted)]" />
        <div className="h-36 animate-pulse rounded-2xl bg-[var(--cp-surface-muted)]" />
      </div>
    )
  }

  const summary = overview?.summary
  const timeline = (overview?.timeline ?? []).slice(-(compact ? 16 : 24))
  const timelineTicks = getTimelineTicks(timeline, compact ? 4 : 6)
  const perInterface = compact
    ? (overview?.perInterface ?? []).slice(0, 6)
    : overview?.perInterface ?? []
  const hiddenCount = Math.max(0, (overview?.perInterface?.length ?? 0) - perInterface.length)

  const throughputData = [
      {
        id: 'Download',
        data: timeline.map((point) => ({ x: point.time, y: point.rx / 1024 / 1024 })),
      },
      {
        id: 'Upload',
        data: timeline.map((point) => ({ x: point.time, y: point.tx / 1024 / 1024 })),
      },
    ]

  const healthData = [
      {
        id: 'Errors',
        data: timeline.map((point) => ({ x: point.time, y: point.errors ?? 0 })),
      },
      {
        id: 'Drops',
        data: timeline.map((point) => ({ x: point.time, y: point.drops ?? 0 })),
      },
    ]

  const maxHealth = Math.max(
    1,
    ...timeline.map((point) => Math.max(point.errors ?? 0, point.drops ?? 0)),
  )

  return (
    <div className="space-y-4">
      <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-4">
        <div className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2">
          <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Download</p>
          <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">{formatRate(summary?.rxPerSec ?? 0)}</p>
          <p className="text-[11px] text-[var(--cp-muted)]">Total {formatBytes(summary?.rxBytes ?? 0)}</p>
        </div>
        <div className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2">
          <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Upload</p>
          <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">{formatRate(summary?.txPerSec ?? 0)}</p>
          <p className="text-[11px] text-[var(--cp-muted)]">Total {formatBytes(summary?.txBytes ?? 0)}</p>
        </div>
        <div className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2">
          <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Errors / s</p>
          <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">
            {Math.round((timeline.at(-1)?.errors ?? 0) || 0)}
          </p>
          <p className="text-[11px] text-[var(--cp-muted)]">Total {summary?.rxErrors ?? 0} / {summary?.txErrors ?? 0}</p>
        </div>
        <div className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2">
          <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Drops / s</p>
          <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">
            {Math.round((timeline.at(-1)?.drops ?? 0) || 0)}
          </p>
          <p className="text-[11px] text-[var(--cp-muted)]">Total {summary?.rxDrops ?? 0} / {summary?.txDrops ?? 0}</p>
        </div>
      </div>

      <div className="grid gap-4 lg:grid-cols-2">
        <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-3">
          <p className="mb-2 text-sm font-semibold text-[var(--cp-ink)]">Network throughput (MB/s)</p>
          <LineAreaChart
            data={throughputData}
            height={compact ? 170 : 210}
            colors={['var(--cp-primary)', 'var(--cp-accent)']}
            axisBottom={{ tickSize: 0, tickPadding: 8, tickValues: timelineTicks }}
            axisLeft={{ tickSize: 0, tickPadding: 8 }}
            yScaleMin={0}
            yScaleMax="auto"
            valueFormatter={(value) => `${value.toFixed(2)} MB/s`}
          />
        </div>
        <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-3">
          <p className="mb-2 text-sm font-semibold text-[var(--cp-ink)]">Errors and drops (/s)</p>
          <LineAreaChart
            data={healthData}
            height={compact ? 170 : 210}
            colors={['#ef4444', '#f59e0b']}
            axisBottom={{ tickSize: 0, tickPadding: 8, tickValues: timelineTicks }}
            axisLeft={{ tickSize: 0, tickPadding: 8, tickValues: [0, Math.ceil(maxHealth / 2), maxHealth] }}
            yScaleMin={0}
            yScaleMax={maxHealth}
            valueFormatter={(value) => `${Math.round(value)}`}
          />
        </div>
      </div>

      <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-3">
        <div className="mb-2 flex items-center justify-between">
          <p className="text-sm font-semibold text-[var(--cp-ink)]">Per-interface</p>
          <p className="text-[11px] text-[var(--cp-muted)]">{summary?.interfaceCount ?? 0} interfaces</p>
        </div>
        <div className="max-h-64 overflow-auto">
          <table className="w-full text-xs">
            <thead className="sticky top-0 bg-white text-[var(--cp-muted)]">
              <tr className="border-b border-[var(--cp-border)]">
                <th className="py-2 text-left font-medium">Interface</th>
                <th className="py-2 text-right font-medium">Down</th>
                <th className="py-2 text-right font-medium">Up</th>
                <th className="py-2 text-right font-medium">Errors</th>
                <th className="py-2 text-right font-medium">Drops</th>
              </tr>
            </thead>
            <tbody>
              {perInterface.map((iface) => {
                const errorTotal = (iface.rxErrors ?? 0) + (iface.txErrors ?? 0)
                const dropTotal = (iface.rxDrops ?? 0) + (iface.txDrops ?? 0)
                return (
                  <tr key={iface.name} className="border-b border-[var(--cp-border)]/70 text-[var(--cp-ink)]">
                    <td className="py-2 font-medium">{iface.name}</td>
                    <td className="py-2 text-right">{formatRate(iface.rxPerSec ?? 0)}</td>
                    <td className="py-2 text-right">{formatRate(iface.txPerSec ?? 0)}</td>
                    <td className={`py-2 text-right ${errorTotal > 0 ? 'text-rose-700' : 'text-[var(--cp-muted)]'}`}>
                      {errorTotal}
                    </td>
                    <td className={`py-2 text-right ${dropTotal > 0 ? 'text-amber-700' : 'text-[var(--cp-muted)]'}`}>
                      {dropTotal}
                    </td>
                  </tr>
                )
              })}
            </tbody>
          </table>
        </div>
        {hiddenCount > 0 ? (
          <p className="mt-2 text-[11px] text-[var(--cp-muted)]">+ {hiddenCount} more interfaces in full view.</p>
        ) : null}
      </div>

      {errorMessage ? (
        <div className="rounded-xl border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-800">
          {errorMessage}
        </div>
      ) : null}
    </div>
  )
}

export default NetworkOverviewPanel
