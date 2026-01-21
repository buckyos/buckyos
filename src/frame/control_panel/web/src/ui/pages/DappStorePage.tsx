import { useEffect, useState } from 'react'

import { fetchAppsList, mockDappStoreData } from '@/api'
import Icon from '../icons'

const formatSettings = (settings: DappCard['settings']) => {
  if (settings === undefined) {
    return '—'
  }
  if (settings === null) {
    return 'null'
  }
  if (typeof settings === 'string') {
    return settings
  }
  try {
    return JSON.stringify(settings, null, 2)
  } catch {
    return String(settings)
  }
}

const DappStorePage = () => {
  const [apps, setApps] = useState<DappCard[]>(mockDappStoreData)
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    let cancelled = false
    const loadApps = async () => {
      const { data, error } = await fetchAppsList()
      if (!cancelled) {
        setApps(data ?? mockDappStoreData)
        if (error) {
          // eslint-disable-next-line no-console
          console.warn('Apps API unavailable, using mock data', error)
        }
        setLoading(false)
      }
    }
    loadApps()
    return () => {
      cancelled = true
    }
  }, [])

  if (loading) {
    return (
      <div className="cp-panel flex min-h-[60vh] items-center justify-center px-8 py-12">
        <div className="flex items-center gap-3 text-[var(--cp-muted)]">
          <span className="size-3 animate-pulse rounded-full bg-[var(--cp-primary)]" aria-hidden />
          <span className="text-sm">Loading applications...</span>
        </div>
      </div>
    )
  }

  return (
    <div className="space-y-6">
      <header className="cp-panel px-9 py-7">
        <div className="flex flex-wrap items-center justify-between gap-4">
          <div>
            <h1 className="text-2xl font-semibold text-[var(--cp-ink)] sm:text-3xl">dApp Store</h1>
            <p className="text-sm text-[var(--cp-muted)]">
              Discover, install, and update decentralized applications for your BuckyOS node.
            </p>
          </div>
          <button
            type="button"
            className="inline-flex items-center gap-2 rounded-full bg-[var(--cp-primary)] px-4 py-2 text-sm font-medium text-white shadow transition hover:bg-[var(--cp-primary-strong)]"
          >
            Browse Catalog
          </button>
        </div>
      </header>

      <section className="cp-panel p-7">
        <div className="mb-6 flex items-center justify-between text-[var(--cp-ink)]">
          <div className="flex items-center gap-3 text-lg font-semibold">
            <span className="inline-flex size-9 items-center justify-center rounded-2xl bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]">
              <Icon name="apps" className="size-4" />
            </span>
            <span>Applications</span>
          </div>
          <div className="text-xs text-[var(--cp-muted)]">Installed / Available / Updates</div>
        </div>
        <div className="grid gap-5 md:grid-cols-2">
          {apps.map((app) => (
            <div
              key={app.name}
              className="flex items-start gap-5 rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface)] p-5 text-sm text-[var(--cp-muted)]"
            >
              <div className="inline-flex size-10 shrink-0 items-center justify-center rounded-2xl bg-[var(--cp-surface-muted)] text-[var(--cp-primary-strong)]">
                <Icon name={app.icon} className="size-4" />
              </div>
              <div className="min-w-0 flex-1">
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0">
                    <p
                      className="truncate text-base font-semibold text-[var(--cp-ink)]"
                      title={app.name}
                    >
                      {app.name}
                    </p>
                    <p className="text-xs text-[var(--cp-muted)]">{app.category}</p>
                  </div>
                  <span
                    className={`rounded-full px-3 py-1 text-[11px] uppercase tracking-wide ${
                      app.status === 'installed'
                        ? 'bg-emerald-100 text-emerald-700'
                        : 'bg-[var(--cp-surface-muted)] text-[var(--cp-muted)]'
                    }`}
                  >
                    {app.status}
                  </span>
                </div>
                <div className="mt-2 flex items-center justify-between text-xs text-[var(--cp-muted)]">
                  <span>v{app.version}</span>
                  <button
                    type="button"
                    className="rounded-full border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-1 text-xs text-[var(--cp-ink)] transition hover:border-[var(--cp-primary)] hover:text-[var(--cp-primary-strong)]"
                  >
                    {app.status === 'installed' ? 'Open' : 'Install'}
                  </button>
                </div>
                <div className="mt-3 rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2">
                  <p className="text-[10px] uppercase tracking-wide text-[var(--cp-muted)]">
                    Settings
                  </p>
                  <pre className="mt-1 max-h-28 overflow-auto whitespace-pre-wrap break-words text-[11px] text-[var(--cp-ink)]">
                    {formatSettings(app.settings)}
                  </pre>
                </div>
              </div>
            </div>
          ))}
        </div>
      </section>
    </div>
  )
}

export default DappStorePage
