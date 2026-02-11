type StorageDiskStatusPanelProps = {
  disk: SystemMetricsDisk | null | undefined
  loading?: boolean
  errorMessage?: string | null
  compact?: boolean
  maxItems?: number
}

type DiskHealthTone = {
  label: 'healthy' | 'warning' | 'critical'
  pillClass: string
  barClass: string
  textClass: string
}

const getDiskUsagePercent = (diskItem: DiskInfo) => {
  if (typeof diskItem.usagePercent === 'number') {
    return Math.max(0, Math.min(100, Math.round(diskItem.usagePercent)))
  }

  if (!diskItem.totalGb) {
    return 0
  }

  return Math.max(0, Math.min(100, Math.round((diskItem.usedGb / diskItem.totalGb) * 100)))
}

const getHealthTone = (usagePercent: number): DiskHealthTone => {
  if (usagePercent >= 95) {
    return {
      label: 'critical',
      pillClass: 'bg-rose-100 text-rose-700',
      barClass: 'bg-rose-500',
      textClass: 'text-rose-700',
    }
  }

  if (usagePercent >= 85) {
    return {
      label: 'warning',
      pillClass: 'bg-amber-100 text-amber-700',
      barClass: 'bg-amber-500',
      textClass: 'text-amber-700',
    }
  }

  return {
    label: 'healthy',
    pillClass: 'bg-emerald-100 text-emerald-700',
    barClass: 'bg-emerald-500',
    textClass: 'text-emerald-700',
  }
}

const formatGb = (value: number) => `${value.toFixed(value >= 100 ? 0 : 1)} GB`

const StorageDiskStatusPanel = ({
  disk,
  loading = false,
  errorMessage,
  compact = false,
  maxItems,
}: StorageDiskStatusPanelProps) => {
  const totalGb = disk?.totalGb ?? 0
  const usedGb = disk?.usedGb ?? 0
  const usagePercent = totalGb > 0 ? Math.max(0, Math.min(100, Math.round((usedGb / totalGb) * 100))) : 0
  const freeGb = Math.max(0, totalGb - usedGb)
  const summaryTone = getHealthTone(usagePercent)

  const diskItems = (disk?.disks ?? []).map((diskItem) => {
    const itemUsage = getDiskUsagePercent(diskItem)
    return {
      ...diskItem,
      usagePercent: itemUsage,
      health: getHealthTone(itemUsage),
    }
  })

  const visibleDisks = typeof maxItems === 'number' ? diskItems.slice(0, maxItems) : diskItems
  const hiddenCount = Math.max(0, diskItems.length - visibleDisks.length)

  const healthCounts = diskItems.reduce(
    (acc, item) => {
      acc[item.health.label] += 1
      return acc
    },
    { healthy: 0, warning: 0, critical: 0 },
  )

  if (loading) {
    return (
      <div className="space-y-3">
        <div className="h-16 animate-pulse rounded-2xl bg-[var(--cp-surface-muted)]" />
        <div className="h-20 animate-pulse rounded-2xl bg-[var(--cp-surface-muted)]" />
        <div className="h-20 animate-pulse rounded-2xl bg-[var(--cp-surface-muted)]" />
      </div>
    )
  }

  return (
    <div className="space-y-3">
      <div className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] p-4">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div>
            <p className="text-xs uppercase tracking-wide text-[var(--cp-muted)]">Pool capacity</p>
            <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">
              {formatGb(usedGb)} / {formatGb(totalGb)}
            </p>
          </div>
          <span className={`rounded-full px-2.5 py-1 text-[10px] font-semibold uppercase tracking-wide ${summaryTone.pillClass}`}>
            {summaryTone.label}
          </span>
        </div>
        <div className="mt-2 h-2 overflow-hidden rounded-full bg-white">
          <div className={`h-full rounded-full ${summaryTone.barClass}`} style={{ width: `${usagePercent}%` }} />
        </div>
        <div className="mt-2 flex items-center justify-between text-[11px] text-[var(--cp-muted)]">
          <span>{usagePercent}% used</span>
          <span>{formatGb(freeGb)} free</span>
        </div>
      </div>

      {!compact ? (
        <div className="grid gap-2 sm:grid-cols-3">
          <div className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2 text-xs">
            <p className="uppercase tracking-wide text-[var(--cp-muted)]">Healthy</p>
            <p className="mt-1 text-lg font-semibold text-emerald-700">{healthCounts.healthy}</p>
          </div>
          <div className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2 text-xs">
            <p className="uppercase tracking-wide text-[var(--cp-muted)]">Warning</p>
            <p className="mt-1 text-lg font-semibold text-amber-700">{healthCounts.warning}</p>
          </div>
          <div className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2 text-xs">
            <p className="uppercase tracking-wide text-[var(--cp-muted)]">Critical</p>
            <p className="mt-1 text-lg font-semibold text-rose-700">{healthCounts.critical}</p>
          </div>
        </div>
      ) : null}

      <div className="space-y-2">
        {visibleDisks.map((diskItem) => (
          <div
            key={`${diskItem.mount}-${diskItem.label}`}
            className="rounded-2xl border border-[var(--cp-border)] bg-white px-3 py-3"
          >
            <div className="flex items-start justify-between gap-3">
              <div className="min-w-0">
                <p className="truncate text-sm font-semibold text-[var(--cp-ink)]">{diskItem.label}</p>
                <p className="truncate text-xs text-[var(--cp-muted)]">
                  {diskItem.mount} - {diskItem.fs ?? 'unknown'}
                </p>
              </div>
              <span className={`text-xs font-semibold ${diskItem.health.textClass}`}>{diskItem.usagePercent}%</span>
            </div>
            <div className="mt-2 h-2 overflow-hidden rounded-full bg-[var(--cp-surface-muted)]">
              <div
                className={`h-full rounded-full ${diskItem.health.barClass}`}
                style={{ width: `${diskItem.usagePercent}%` }}
              />
            </div>
            <div className="mt-2 flex items-center justify-between text-[11px] text-[var(--cp-muted)]">
              <span>
                {formatGb(diskItem.usedGb)} / {formatGb(diskItem.totalGb)}
              </span>
              {!compact ? (
                <span className={`rounded-full px-2 py-0.5 font-semibold uppercase tracking-wide ${diskItem.health.pillClass}`}>
                  {diskItem.health.label}
                </span>
              ) : null}
            </div>
          </div>
        ))}
      </div>

      {!visibleDisks.length ? (
        <div className="rounded-2xl border border-dashed border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-6 text-sm text-[var(--cp-muted)]">
          No disk details available yet.
        </div>
      ) : null}

      {hiddenCount > 0 ? (
        <p className="text-xs text-[var(--cp-muted)]">+ {hiddenCount} more disks in full storage view.</p>
      ) : null}

      {errorMessage ? (
        <div className="rounded-2xl border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-800">
          {errorMessage}
        </div>
      ) : null}
    </div>
  )
}

export default StorageDiskStatusPanel
