type ContainerOverviewPanelProps = {
  overview: ContainerOverview | null
  loading?: boolean
  errorMessage?: string | null
  compact?: boolean
  actionLoadingId?: string | null
  onContainerAction?: (id: string, action: 'start' | 'stop' | 'restart') => void
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

const badgeClassByState = (state: string) => {
  const normalized = state.toLowerCase()
  if (normalized === 'running') {
    return 'bg-emerald-100 text-emerald-700'
  }
  if (normalized === 'paused') {
    return 'bg-amber-100 text-amber-700'
  }
  if (normalized === 'restarting') {
    return 'bg-sky-100 text-sky-700'
  }
  if (normalized === 'dead') {
    return 'bg-rose-100 text-rose-700'
  }
  return 'bg-slate-100 text-slate-700'
}

const ContainerOverviewPanel = ({
  overview,
  loading = false,
  errorMessage,
  compact = false,
  actionLoadingId,
  onContainerAction,
}: ContainerOverviewPanelProps) => {
  if (loading) {
    return (
      <div className="space-y-3">
        <div className="h-16 animate-pulse rounded-2xl bg-[var(--cp-surface-muted)]" />
        <div className="h-28 animate-pulse rounded-2xl bg-[var(--cp-surface-muted)]" />
      </div>
    )
  }

  if (!overview?.available) {
    return (
      <div className="rounded-2xl border border-amber-200 bg-amber-50 px-4 py-3 text-sm text-amber-800">
        Docker is not available on this node.
        {errorMessage ? <p className="mt-1 text-xs">{errorMessage}</p> : null}
      </div>
    )
  }

  const containers = compact ? (overview.containers ?? []).slice(0, 6) : overview.containers ?? []
  const hiddenCount = Math.max(0, (overview.containers?.length ?? 0) - containers.length)

  return (
    <div className="space-y-3">
      <div className="grid gap-2 sm:grid-cols-3 lg:grid-cols-6">
        <div className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2">
          <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Total</p>
          <p className="mt-1 text-lg font-semibold text-[var(--cp-ink)]">{overview.summary?.total ?? 0}</p>
        </div>
        <div className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2">
          <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Running</p>
          <p className="mt-1 text-lg font-semibold text-emerald-700">{overview.summary?.running ?? 0}</p>
        </div>
        <div className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2">
          <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Exited</p>
          <p className="mt-1 text-lg font-semibold text-slate-700">{overview.summary?.exited ?? 0}</p>
        </div>
        <div className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2">
          <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Paused</p>
          <p className="mt-1 text-lg font-semibold text-amber-700">{overview.summary?.paused ?? 0}</p>
        </div>
        <div className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2">
          <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">CPU Cores</p>
          <p className="mt-1 text-lg font-semibold text-[var(--cp-ink)]">{overview.server?.cpuCount ?? 0}</p>
        </div>
        <div className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2">
          <p className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">Memory</p>
          <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">{formatBytes(overview.server?.memTotalBytes ?? 0)}</p>
        </div>
      </div>

      <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-3">
        <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
          <p className="text-sm font-semibold text-[var(--cp-ink)]">Docker Engine</p>
          <span className="rounded-full bg-slate-100 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-slate-700">
            v{overview.server?.version || 'unknown'}
          </span>
        </div>
        <p className="text-xs text-[var(--cp-muted)]">
          {overview.server?.os || '-'} · {overview.server?.kernel || '-'} · Driver {overview.server?.driver || '-'}
        </p>
      </div>

      <div className="space-y-2">
        {containers.map((item) => (
          <div
            key={`${item.id}-${item.name}`}
            className="rounded-xl border border-[var(--cp-border)] bg-white px-3 py-2"
          >
            <div className="flex flex-wrap items-center justify-between gap-2">
              <div className="min-w-0">
                <p className="truncate text-sm font-semibold text-[var(--cp-ink)]">{item.name || item.id}</p>
                <p className="truncate text-xs text-[var(--cp-muted)]">{item.image || '-'}</p>
              </div>
              <span className={`rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide ${badgeClassByState(item.state || '')}`}>
                {item.state || 'unknown'}
              </span>
            </div>
            <div className="mt-1 text-[11px] text-[var(--cp-muted)]">
              <p className="break-all">{item.status || '-'}</p>
              <p className="break-all">Ports: {item.ports || '-'}</p>
              <p className="break-all">Networks: {item.networks || '-'}</p>
            </div>
            {!compact && onContainerAction ? (
              <div className="mt-2 flex flex-wrap gap-2">
                <button
                  type="button"
                  disabled={actionLoadingId === item.id}
                  onClick={() => onContainerAction(item.id, 'start')}
                  className="rounded-full border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-2.5 py-1 text-[10px] font-semibold uppercase tracking-wide text-[var(--cp-ink)] transition hover:border-emerald-300 hover:text-emerald-700 disabled:cursor-not-allowed disabled:opacity-50"
                >
                  Start
                </button>
                <button
                  type="button"
                  disabled={actionLoadingId === item.id}
                  onClick={() => onContainerAction(item.id, 'stop')}
                  className="rounded-full border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-2.5 py-1 text-[10px] font-semibold uppercase tracking-wide text-[var(--cp-ink)] transition hover:border-amber-300 hover:text-amber-700 disabled:cursor-not-allowed disabled:opacity-50"
                >
                  Stop
                </button>
                <button
                  type="button"
                  disabled={actionLoadingId === item.id}
                  onClick={() => onContainerAction(item.id, 'restart')}
                  className="rounded-full border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-2.5 py-1 text-[10px] font-semibold uppercase tracking-wide text-[var(--cp-ink)] transition hover:border-sky-300 hover:text-sky-700 disabled:cursor-not-allowed disabled:opacity-50"
                >
                  {actionLoadingId === item.id ? 'Working...' : 'Restart'}
                </button>
              </div>
            ) : null}
          </div>
        ))}
      </div>

      {hiddenCount > 0 ? (
        <p className="text-xs text-[var(--cp-muted)]">+ {hiddenCount} more containers in full view.</p>
      ) : null}

      {errorMessage ? (
        <div className="rounded-xl border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-800">
          {errorMessage}
        </div>
      ) : null}
    </div>
  )
}

export default ContainerOverviewPanel
