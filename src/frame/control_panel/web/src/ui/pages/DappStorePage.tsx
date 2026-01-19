import Icon from '../icons'

const dapps: DappCard[] = [
  { name: 'FileSync', icon: 'package', category: 'Storage', status: 'installed', version: '1.4.2' },
  { name: 'SecureChat', icon: 'package', category: 'Communication', status: 'available', version: '2.1.0' },
  { name: 'CloudBridge', icon: 'package', category: 'Networking', status: 'available', version: '0.9.5' },
  { name: 'PhotoVault', icon: 'package', category: 'Media', status: 'installed', version: '1.2.0' },
  { name: 'DataAnalyzer', icon: 'package', category: 'Analytics', status: 'installed', version: '3.0.1' },
  { name: 'WebPortal', icon: 'package', category: 'Web', status: 'available', version: '1.0.0' },
]

const DappStorePage = () => {
  return (
    <div className="space-y-6">
      <header className="cp-panel px-8 py-6">
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

      <section className="cp-panel p-6">
        <div className="mb-6 flex items-center justify-between text-[var(--cp-ink)]">
          <div className="flex items-center gap-3 text-lg font-semibold">
            <span className="inline-flex size-9 items-center justify-center rounded-2xl bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]">
              <Icon name="apps" className="size-4" />
            </span>
            <span>Applications</span>
          </div>
          <div className="text-xs text-[var(--cp-muted)]">Installed / Available / Updates</div>
        </div>
        <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
          {dapps.map((app) => (
            <div
              key={app.name}
              className="flex items-start gap-4 rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface)] p-4 text-sm text-[var(--cp-muted)]"
            >
              <div className="inline-flex size-10 items-center justify-center rounded-2xl bg-[var(--cp-surface-muted)] text-[var(--cp-primary-strong)]">
                <Icon name={app.icon} className="size-4" />
              </div>
              <div className="flex-1">
                <div className="flex items-center justify-between">
                  <div>
                    <p className="text-base font-semibold text-[var(--cp-ink)]">{app.name}</p>
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
              </div>
            </div>
          ))}
        </div>
      </section>
    </div>
  )
}

export default DappStorePage
