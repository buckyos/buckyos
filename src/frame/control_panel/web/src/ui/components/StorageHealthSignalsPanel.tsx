type StorageHealthSignalsPanelProps = {
  warnings?: SystemWarning[]
  disks?: DiskInfo[]
  loading?: boolean
  compact?: boolean
}

const getDiskUsagePercent = (disk: DiskInfo) => {
  if (typeof disk.usagePercent === 'number') {
    return Math.max(0, Math.min(100, Math.round(disk.usagePercent)))
  }

  if (!disk.totalGb) {
    return 0
  }

  return Math.max(0, Math.min(100, Math.round((disk.usedGb / disk.totalGb) * 100)))
}

const StorageHealthSignalsPanel = ({
  warnings = [],
  disks = [],
  loading = false,
  compact = false,
}: StorageHealthSignalsPanelProps) => {
  const storageWarnings = warnings.filter((warning) => warning.label === 'Storage')
  const hotDisks = disks
    .map((disk) => ({
      ...disk,
      usagePercent: getDiskUsagePercent(disk),
    }))
    .filter((disk) => disk.usagePercent >= 85)
    .sort((a, b) => b.usagePercent - a.usagePercent)
    .slice(0, compact ? 3 : 6)

  if (loading) {
    return (
      <div className="space-y-2">
        <div className="h-12 animate-pulse rounded-xl bg-[var(--cp-surface-muted)]" />
        <div className="h-12 animate-pulse rounded-xl bg-[var(--cp-surface-muted)]" />
      </div>
    )
  }

  return (
    <div className="space-y-2">
      {storageWarnings.map((warning) => (
        <div
          key={`${warning.label}-${warning.message}`}
          className="rounded-xl border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-800"
        >
          <p className="font-semibold">{warning.label}</p>
          <p>{warning.message}</p>
        </div>
      ))}

      {hotDisks.map((disk) => (
        <div
          key={`${disk.mount}-${disk.label}`}
          className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2 text-xs"
        >
          <p className="font-semibold text-[var(--cp-ink)]">{disk.label}</p>
          <p className="text-[var(--cp-muted)]">
            {disk.mount} at {disk.usagePercent}%
          </p>
        </div>
      ))}

      {!storageWarnings.length && !hotDisks.length ? (
        <div className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-3 text-xs text-[var(--cp-muted)]">
          No storage warnings right now.
        </div>
      ) : null}
    </div>
  )
}

export default StorageHealthSignalsPanel
