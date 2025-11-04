const dapps: DappCard[] = [
  { name: 'FileSync', icon: '🗂️', category: 'Storage', status: 'installed', version: '1.4.2' },
  { name: 'SecureChat', icon: '💬', category: 'Communication', status: 'available', version: '2.1.0' },
  { name: 'CloudBridge', icon: '🌉', category: 'Networking', status: 'available', version: '0.9.5' },
  { name: 'PhotoVault', icon: '📷', category: 'Media', status: 'installed', version: '1.2.0' },
  { name: 'DataAnalyzer', icon: '📊', category: 'Analytics', status: 'installed', version: '3.0.1' },
  { name: 'WebPortal', icon: '🌐', category: 'Web', status: 'available', version: '1.0.0' },
]

const DappStorePage = () => {
  return (
    <div className="space-y-6">
      <header className="rounded-3xl border border-slate-900/60 bg-slate-900/50 px-8 py-6 shadow-lg shadow-black/20 backdrop-blur">
        <div className="flex flex-wrap items-center justify-between gap-4">
          <div>
            <h1 className="text-2xl font-semibold text-white sm:text-3xl">dApp Store</h1>
            <p className="text-sm text-slate-400">
              Discover, install, and update decentralized applications for your BuckyOS node.
            </p>
          </div>
          <button
            type="button"
            className="inline-flex items-center gap-2 rounded-full bg-sky-500 px-4 py-2 text-sm font-medium text-white shadow hover:bg-sky-400"
          >
            Browse Catalog
          </button>
        </div>
      </header>

      <section className="rounded-3xl border border-slate-900/60 bg-slate-900/60 p-6 shadow-lg shadow-black/20">
        <div className="mb-6 flex items-center justify-between text-white">
          <div className="flex items-center gap-2 text-lg font-semibold">
            <span aria-hidden>🛒</span>
            <span>Applications</span>
          </div>
          <div className="text-xs text-slate-400">Installed • Available • Updates</div>
        </div>
        <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
          {dapps.map((app) => (
            <div
              key={app.name}
              className="flex items-start gap-4 rounded-2xl border border-slate-900/80 bg-slate-900/60 p-4 text-sm text-slate-300"
            >
              <div className="inline-flex size-10 items-center justify-center rounded-2xl bg-slate-800 text-lg">
                {app.icon}
              </div>
              <div className="flex-1">
                <div className="flex items-center justify-between">
                  <div>
                    <p className="text-base font-semibold text-white">{app.name}</p>
                    <p className="text-xs text-slate-400">{app.category}</p>
                  </div>
                  <span
                    className={`rounded-full px-3 py-1 text-[11px] uppercase tracking-wide ${
                      app.status === 'installed'
                        ? 'bg-emerald-500/15 text-emerald-300'
                        : 'bg-slate-800 text-slate-200'
                    }`}
                  >
                    {app.status}
                  </span>
                </div>
                <div className="mt-2 flex items-center justify-between text-xs text-slate-400">
                  <span>v{app.version}</span>
                  <button
                    type="button"
                    className="rounded-full bg-slate-800 px-3 py-1 text-xs text-slate-200 transition hover:bg-slate-700"
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
