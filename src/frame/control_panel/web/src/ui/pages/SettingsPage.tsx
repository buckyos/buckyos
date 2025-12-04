const settingsBlocks: SettingBlock[] = [
  {
    title: 'General',
    description: 'Node name, locale, and branding for your control panel.',
    actions: ['Edit'],
    icon: '⚙️',
  },
  {
    title: 'Security',
    description: 'MFA, session policies, device trust, and audit retention.',
    actions: ['Configure'],
    icon: '🛡️',
  },
  {
    title: 'Networking',
    description: 'Ports, gateways, SN settings, and zero-trust policies.',
    actions: ['Open'],
    icon: '🛰️',
  },
  {
    title: 'Storage',
    description: 'Replication, snapshots, and tiering preferences.',
    actions: ['Review'],
    icon: '🧊',
  },
  {
    title: 'Notifications',
    description: 'Alert channels, thresholds, and escalations.',
    actions: ['Tune'],
    icon: '🔔',
  },
  {
    title: 'Integrations',
    description: 'Connect CI, observability, and external identity providers.',
    actions: ['Manage'],
    icon: '🔗',
  },
]

const SettingsPage = () => {
  return (
    <div className="space-y-6">
      <header className="rounded-3xl border border-slate-900/60 bg-slate-900/50 px-8 py-6 shadow-lg shadow-black/20 backdrop-blur">
        <div className="flex flex-wrap items-center justify-between gap-4">
          <div>
            <h1 className="text-2xl font-semibold text-white sm:text-3xl">Settings</h1>
            <p className="text-sm text-slate-400">
              Adjust system preferences, security posture, and integrations.
            </p>
          </div>
          <button
            type="button"
            className="inline-flex items-center gap-2 rounded-full bg-sky-500 px-4 py-2 text-sm font-medium text-white shadow hover:bg-sky-400"
          >
            Save Profile
          </button>
        </div>
      </header>

      <section className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
        {settingsBlocks.map((block) => (
          <div
            key={block.title}
            className="flex flex-col gap-3 rounded-2xl border border-slate-900/80 bg-slate-900/60 p-5 text-sm text-slate-300 shadow"
          >
            <div className="flex items-center gap-2 text-white">
              <span aria-hidden>{block.icon}</span>
              <p className="text-base font-semibold">{block.title}</p>
            </div>
            <p className="text-xs text-slate-400">{block.description}</p>
            <div className="flex flex-wrap gap-2">
              {block.actions.map((action) => (
                <button
                  key={action}
                  type="button"
                  className="rounded-full bg-slate-800 px-3 py-1 text-xs text-slate-200 transition hover:bg-slate-700"
                >
                  {action}
                </button>
              ))}
            </div>
          </div>
        ))}
      </section>
    </div>
  )
}

export default SettingsPage
