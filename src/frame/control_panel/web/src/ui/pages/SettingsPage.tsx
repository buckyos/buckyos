import Icon from '../icons'

const settingsBlocks: SettingBlock[] = [
  {
    title: 'General',
    description: 'Node name, locale, and branding for your control panel.',
    actions: ['Edit'],
    icon: 'settings',
  },
  {
    title: 'Security',
    description: 'MFA, session policies, device trust, and audit retention.',
    actions: ['Configure'],
    icon: 'shield',
  },
  {
    title: 'Networking',
    description: 'Ports, gateways, SN settings, and zero-trust policies.',
    actions: ['Open'],
    icon: 'network',
  },
  {
    title: 'Storage',
    description: 'Replication, snapshots, and tiering preferences.',
    actions: ['Review'],
    icon: 'storage',
  },
  {
    title: 'Notifications',
    description: 'Alert channels, thresholds, and escalations.',
    actions: ['Tune'],
    icon: 'bell',
  },
  {
    title: 'Integrations',
    description: 'Connect CI, observability, and external identity providers.',
    actions: ['Manage'],
    icon: 'link',
  },
]

const SettingsPage = () => {
  return (
    <div className="space-y-6">
      <header className="cp-panel px-8 py-6">
        <div className="flex flex-wrap items-center justify-between gap-4">
          <div>
            <h1 className="text-2xl font-semibold text-[var(--cp-ink)] sm:text-3xl">Settings</h1>
            <p className="text-sm text-[var(--cp-muted)]">
              Adjust system preferences, security posture, and integrations.
            </p>
          </div>
          <button
            type="button"
            className="inline-flex items-center gap-2 rounded-full bg-[var(--cp-primary)] px-4 py-2 text-sm font-medium text-white shadow transition hover:bg-[var(--cp-primary-strong)]"
          >
            Save Profile
          </button>
        </div>
      </header>

      <section className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
        {settingsBlocks.map((block) => (
          <div
            key={block.title}
            className="flex flex-col gap-3 rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface)] p-5 text-sm text-[var(--cp-muted)] shadow-sm"
          >
            <div className="flex items-center gap-2 text-[var(--cp-ink)]">
              <span className="inline-flex size-9 items-center justify-center rounded-2xl bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]">
                <Icon name={block.icon} className="size-4" />
              </span>
              <p className="text-base font-semibold">{block.title}</p>
            </div>
            <p className="text-xs text-[var(--cp-muted)]">{block.description}</p>
            <div className="flex flex-wrap gap-2">
              {block.actions.map((action) => (
                <button
                  key={action}
                  type="button"
                  className="rounded-full border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-1 text-xs text-[var(--cp-ink)] transition hover:border-[var(--cp-primary)] hover:text-[var(--cp-primary-strong)]"
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
