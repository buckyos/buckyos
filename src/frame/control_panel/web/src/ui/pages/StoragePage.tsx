import { useCallback, useEffect, useMemo, useState } from 'react'
import { Link } from 'react-router-dom'

import { fetchSystemMetrics, fetchSystemStatus } from '@/api'
import { useI18n } from '@/i18n'
import StorageDiskStatusPanel from '../components/StorageDiskStatusPanel'
import StorageHealthSignalsPanel from '../components/StorageHealthSignalsPanel'
import Icon from '../icons'

const POLL_INTERVAL_MS = 6000

const formatGb = (value: number) => `${value.toFixed(value >= 100 ? 0 : 1)} GB`

const toErrorText = (value: unknown, fallback: string) => {
  if (value instanceof Error) {
    return value.message
  }

  if (typeof value === 'string') {
    return value
  }

  return fallback
}

const StoragePage = () => {
  const { t } = useI18n()
  const [metrics, setMetrics] = useState<SystemMetrics | null>(null)
  const [status, setStatus] = useState<SystemStatusResponse | null>(null)
  const [loading, setLoading] = useState(true)
  const [refreshing, setRefreshing] = useState(false)
  const [errorMessage, setErrorMessage] = useState<string | null>(null)
  const [lastUpdatedAt, setLastUpdatedAt] = useState<Date | null>(null)

  const loadStorageData = useCallback(async (silent: boolean) => {
    if (silent) {
      setRefreshing(true)
    } else {
      setLoading(true)
    }

    const [metricsResult, statusResult] = await Promise.all([
      fetchSystemMetrics(),
      fetchSystemStatus(),
    ])

    if (metricsResult.data) {
      setMetrics(metricsResult.data)
    }

    if (statusResult.data) {
      setStatus(statusResult.data)
    }

    const errors = [metricsResult.error, statusResult.error].filter(Boolean)
    if (errors.length > 0) {
      setErrorMessage(toErrorText(errors[0], t('storage.refreshFailed', 'Failed to refresh storage telemetry.')))
    } else {
      setErrorMessage(null)
    }

    setLastUpdatedAt(new Date())
    setLoading(false)
    setRefreshing(false)
  }, [t])

  useEffect(() => {
    let disposed = false

    const safeLoad = async (silent: boolean) => {
      if (disposed) {
        return
      }
      await loadStorageData(silent)
    }

    void safeLoad(false)
    const intervalId = window.setInterval(() => {
      void safeLoad(true)
    }, POLL_INTERVAL_MS)

    return () => {
      disposed = true
      window.clearInterval(intervalId)
    }
  }, [loadStorageData])

  const storageDisk = metrics?.disk ?? null

  const storageStats = useMemo(() => {
    const totalGb = storageDisk?.totalGb ?? 0
    const usedGb = storageDisk?.usedGb ?? 0
    const freeGb = Math.max(0, totalGb - usedGb)
    const usagePercent = totalGb > 0 ? Math.round((usedGb / totalGb) * 100) : 0
    const diskCount = storageDisk?.disks?.length ?? 0

    const hottestDisk = (storageDisk?.disks ?? [])
      .map((diskItem) => {
        const usagePercentFromValue =
          typeof diskItem.usagePercent === 'number'
            ? diskItem.usagePercent
            : diskItem.totalGb > 0
              ? (diskItem.usedGb / diskItem.totalGb) * 100
              : 0

        return {
          ...diskItem,
          usagePercent: Math.round(usagePercentFromValue),
        }
      })
      .sort((a, b) => b.usagePercent - a.usagePercent)[0]

    return {
      totalGb,
      usedGb,
      freeGb,
      usagePercent,
      diskCount,
      hottestDisk,
    }
  }, [storageDisk])

  const stateTone = status?.state ?? 'online'
  const stateClass =
    stateTone === 'critical'
      ? 'bg-rose-100 text-rose-700'
      : stateTone === 'warning'
        ? 'bg-amber-100 text-amber-700'
        : 'bg-emerald-100 text-emerald-700'

  return (
    <div className="space-y-6">
      <header className="cp-panel px-8 py-6">
        <div className="flex flex-wrap items-center justify-between gap-4">
          <div>
            <h1 className="text-2xl font-semibold text-[var(--cp-ink)] sm:text-3xl">{t('storage.pageTitle', 'Storage Center')}</h1>
            <p className="text-sm text-[var(--cp-muted)]">{t('storage.pageDescription', 'Unified disk health and capacity telemetry for this node.')}</p>
          </div>
          <div className="flex items-center gap-2">
            <div className={`cp-pill uppercase tracking-wide ${stateClass}`}>{stateTone}</div>
            {refreshing ? (
              <div className="cp-pill bg-[var(--cp-surface-muted)] text-[var(--cp-muted)]">{t('storage.refreshing', 'Refreshing')}</div>
            ) : null}
          </div>
        </div>
      </header>

      <section className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
        <div className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface)] px-4 py-4 shadow-sm">
          <p className="text-xs uppercase tracking-wide text-[var(--cp-muted)]">{t('storage.totalCapacity', 'Total Capacity')}</p>
          <p className="mt-1 text-2xl font-semibold text-[var(--cp-ink)]">{formatGb(storageStats.totalGb)}</p>
        </div>
        <div className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface)] px-4 py-4 shadow-sm">
          <p className="text-xs uppercase tracking-wide text-[var(--cp-muted)]">{t('storage.used', 'Used')}</p>
          <p className="mt-1 text-2xl font-semibold text-[var(--cp-ink)]">{formatGb(storageStats.usedGb)}</p>
          <p className="mt-1 text-xs text-[var(--cp-muted)]">{t('storage.percentOfPool', '{value}% of pool', { value: storageStats.usagePercent })}</p>
        </div>
        <div className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface)] px-4 py-4 shadow-sm">
          <p className="text-xs uppercase tracking-wide text-[var(--cp-muted)]">{t('storage.freeSpace', 'Free Space')}</p>
          <p className="mt-1 text-2xl font-semibold text-[var(--cp-ink)]">{formatGb(storageStats.freeGb)}</p>
          <p className="mt-1 text-xs text-[var(--cp-muted)]">{t('storage.acrossDisks', 'Across {count} disks', { count: storageStats.diskCount })}</p>
        </div>
        <div className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface)] px-4 py-4 shadow-sm">
          <p className="text-xs uppercase tracking-wide text-[var(--cp-muted)]">{t('storage.hottestDisk', 'Hottest Disk')}</p>
          <p className="mt-1 truncate text-base font-semibold text-[var(--cp-ink)]">
            {storageStats.hottestDisk?.label ?? t('settings.notAvailable', 'N/A')}
          </p>
          <p className="mt-1 text-xs text-[var(--cp-muted)]">
            {storageStats.hottestDisk
              ? t('storage.percentUsed', '{value}% used', { value: storageStats.hottestDisk.usagePercent })
              : t('storage.noDiskData', 'No disk data yet')}
          </p>
        </div>
      </section>

      <section className="grid gap-6 xl:grid-cols-[1.35fr_0.65fr]">
        <div className="cp-panel p-6">
          <div className="mb-4 flex items-center gap-3 text-lg font-semibold text-[var(--cp-ink)]">
            <span className="inline-flex size-9 items-center justify-center rounded-2xl bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]">
              <Icon name="drive" className="size-4" />
            </span>
            <h2>{t('storage.diskHealth', 'Disk Health')}</h2>
          </div>
          <StorageDiskStatusPanel
            disk={storageDisk}
            loading={loading}
            errorMessage={errorMessage}
          />
        </div>

        <div className="space-y-6">
          <div className="cp-panel p-6">
            <div className="mb-4 flex items-center gap-3 text-lg font-semibold text-[var(--cp-ink)]">
              <span className="inline-flex size-9 items-center justify-center rounded-2xl bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]">
                <Icon name="alert" className="size-4" />
              </span>
              <h2>{t('storage.healthSignals', 'Health Signals')}</h2>
            </div>
            <StorageHealthSignalsPanel
              warnings={status?.warnings}
              disks={storageDisk?.disks}
              loading={loading}
            />
          </div>

          <div className="cp-panel p-6">
            <div className="mb-4 flex items-center gap-3 text-lg font-semibold text-[var(--cp-ink)]">
              <span className="inline-flex size-9 items-center justify-center rounded-2xl bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]">
                <Icon name="apps" className="size-4" />
              </span>
              <h2>{t('storage.fileManager', 'File Manager')}</h2>
            </div>
            <p className="text-sm text-[var(--cp-muted)]">
              {t('storage.fileManagerDescription', 'File browsing and sharing live in the standalone Files module, while Storage focuses on capacity, disks, and backup posture.')}
            </p>
            <div className="mt-4 flex flex-wrap items-center gap-2">
              <Link
                to="/"
                className="inline-flex items-center gap-2 rounded-xl bg-[var(--cp-primary)] px-3 py-2 text-xs font-semibold text-white transition hover:bg-[var(--cp-primary-strong)]"
              >
                <Icon name="drive" className="size-4" />
                {t('storage.openDesktopFiles', 'Open desktop files')}
              </Link>
              <Link
                to="/"
                className="inline-flex items-center gap-2 rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface)] px-3 py-2 text-xs font-semibold text-[var(--cp-ink)] transition hover:border-[var(--cp-primary)] hover:text-[var(--cp-primary-strong)]"
              >
                <Icon name="link" className="size-4" />
                {t('storage.openFromDesktop', 'Open from desktop')}
              </Link>
            </div>
          </div>
        </div>
      </section>

      <p className="text-xs text-[var(--cp-muted)]">
        {t('storage.lastUpdated', 'Last updated: {value}', {
          value: lastUpdatedAt ? lastUpdatedAt.toLocaleTimeString() : t('storage.waitingFirstSample', 'waiting for first sample'),
        })}
      </p>
    </div>
  )
}

export default StoragePage
