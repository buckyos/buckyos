import type { FormEvent } from 'react'
import { useEffect, useRef, useState } from 'react'

import { fetchSysConfigTree, fetchSystemMetrics, fetchSystemOverview } from '@/api'
import Icon from '../icons'

const formatUptime = (seconds: number) => {
  const safeSeconds = Math.max(0, Math.floor(seconds))
  const days = Math.floor(safeSeconds / 86400)
  const hours = Math.floor((safeSeconds % 86400) / 3600)
  const minutes = Math.floor((safeSeconds % 3600) / 60)
  const parts = []
  if (days) {
    parts.push(`${days}d`)
  }
  if (hours || days) {
    parts.push(`${hours}h`)
  }
  parts.push(`${minutes}m`)
  return parts.join(' ')
}

type SettingsModule = {
  title: string
  description: string
  icon: IconName
  owner: string
  updatedAt: string
  state: 'ready' | 'review' | 'draft'
  controls: string[]
}

const settingsModules: SettingsModule[] = [
  {
    title: 'General',
    description: 'Node naming, locale defaults, and branding baseline.',
    icon: 'settings',
    owner: 'Platform Ops',
    updatedAt: 'Today 09:20',
    state: 'ready',
    controls: ['Host label policy', 'Timezone + locale', 'Control panel banner'],
  },
  {
    title: 'Security',
    description: 'MFA rules, session behavior, and trusted-device posture.',
    icon: 'shield',
    owner: 'Security Team',
    updatedAt: 'Today 08:10',
    state: 'review',
    controls: ['Role-based MFA', 'Session timeout', 'Audit retention window'],
  },
  {
    title: 'Networking',
    description: 'SN endpoints, DNS fallback, gateway forwarding, and ports.',
    icon: 'network',
    owner: 'Network Ops',
    updatedAt: 'Yesterday 21:42',
    state: 'ready',
    controls: ['SN host preference', 'Gateway host routes', 'DNS resolver policy'],
  },
  {
    title: 'Storage',
    description: 'Snapshot cadence, replication plan, and capacity thresholding.',
    icon: 'storage',
    owner: 'Infra Team',
    updatedAt: 'Yesterday 19:30',
    state: 'review',
    controls: ['Snapshot schedule', 'Capacity guardrails', 'Replica consistency checks'],
  },
  {
    title: 'Notifications',
    description: 'Alert channels, severity routing, and incident escalation.',
    icon: 'bell',
    owner: 'SRE Team',
    updatedAt: 'Today 06:54',
    state: 'draft',
    controls: ['Critical paging path', 'Digest cadence', 'Mute windows'],
  },
  {
    title: 'Integrations',
    description: 'Repo hooks, observability exports, and identity providers.',
    icon: 'link',
    owner: 'DevEx Team',
    updatedAt: 'Today 07:35',
    state: 'ready',
    controls: ['Webhook secrets', 'Metrics export endpoint', 'OIDC federation'],
  },
]

const policyBaseline = [
  { key: 'MFA', value: 'Required for Owner/Admin', tone: 'ready' as const },
  { key: 'Session', value: '12h idle timeout', tone: 'ready' as const },
  { key: 'Backups', value: 'Nightly 04:00 validation', tone: 'review' as const },
  { key: 'Audit Logs', value: 'Retain 90 days', tone: 'ready' as const },
  { key: 'Alert Escalation', value: 'P1 -> Pager + Email', tone: 'draft' as const },
]

const integrationChannels = [
  {
    name: 'Observability Export',
    endpoint: 'https://metrics.example.net/v1/push',
    state: 'Active',
  },
  {
    name: 'Webhook Relay',
    endpoint: 'https://hooks.example.net/control-panel',
    state: 'Active',
  },
  {
    name: 'OIDC Provider',
    endpoint: 'https://identity.example.net',
    state: 'Pending',
  },
]

const SettingsPage = () => {
  const mountedRef = useRef(true)
  const [overview, setOverview] = useState<SystemOverview | null>(null)
  const [metrics, setMetrics] = useState<SystemMetrics | null>(null)
  const [configKey, setConfigKey] = useState('boot')
  const [configDepth, setConfigDepth] = useState(2)
  const [configTree, setConfigTree] = useState<SysConfigTreeResponse | null>(null)
  const [overviewError, setOverviewError] = useState<string | null>(null)
  const [metricsError, setMetricsError] = useState<string | null>(null)
  const [configError, setConfigError] = useState<string | null>(null)
  const [loadingOverview, setLoadingOverview] = useState(true)
  const [loadingMetrics, setLoadingMetrics] = useState(true)
  const [loadingConfig, setLoadingConfig] = useState(true)

  const loadSystemOverview = async () => {
    setLoadingOverview(true)
    const { data, error } = await fetchSystemOverview()
    if (!mountedRef.current) {
      return
    }
    if (error) {
      // eslint-disable-next-line no-console
      console.warn('System overview API unavailable', error)
      setOverviewError('System overview is unavailable.')
    } else {
      setOverviewError(null)
    }
    setOverview(data)
    setLoadingOverview(false)
  }

  const loadSystemMetrics = async () => {
    setLoadingMetrics(true)
    const { data, error } = await fetchSystemMetrics()
    if (!mountedRef.current) {
      return
    }
    if (error) {
      // eslint-disable-next-line no-console
      console.warn('System metrics API unavailable', error)
      setMetricsError('System metrics are unavailable.')
    } else {
      setMetricsError(null)
    }
    setMetrics(data)
    setLoadingMetrics(false)
  }

  const loadConfigTree = async (key: string, depth: number) => {
    setLoadingConfig(true)
    const { data, error } = await fetchSysConfigTree(key, depth)
    if (!mountedRef.current) {
      return
    }
    if (error) {
      // eslint-disable-next-line no-console
      console.warn('Sys config tree API unavailable', error)
      setConfigError('System config tree is unavailable.')
    } else {
      setConfigError(null)
    }
    setConfigTree(data)
    setLoadingConfig(false)
  }

  useEffect(() => {
    mountedRef.current = true
    loadSystemOverview()
    loadSystemMetrics()
    loadConfigTree(configKey, configDepth)

    return () => {
      mountedRef.current = false
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const cpuUsage = Math.round(metrics?.cpu?.usagePercent ?? 0)
  const memoryUsage = Math.round(metrics?.memory?.usagePercent ?? 0)
  const diskUsage = Math.round(metrics?.disk?.usagePercent ?? 0)
  const diskTotal = metrics?.disk?.totalGb ?? 0
  const diskUsed = metrics?.disk?.usedGb ?? 0
  const diskItems = metrics?.disk?.disks ?? []
  const uptimeLabel = overview ? formatUptime(overview.uptime_seconds) : '—'

  const handleConfigDepthChange = (value: string) => {
    const next = Number.parseInt(value, 10)
    if (Number.isNaN(next)) {
      setConfigDepth(1)
      return
    }
    const clamped = Math.min(Math.max(next, 1), 6)
    setConfigDepth(clamped)
  }

  const handleConfigSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    const key = configKey.trim()
    const depth = Math.min(Math.max(configDepth, 1), 6)
    setConfigKey(key)
    setConfigDepth(depth)
    await loadConfigTree(key, depth)
  }

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

      <section className="grid gap-6 lg:grid-cols-[1.1fr_0.9fr]">
        <div className="cp-panel p-6">
          <div className="mb-5 flex items-center gap-3 text-lg font-semibold text-[var(--cp-ink)]">
            <span className="inline-flex size-9 items-center justify-center rounded-2xl bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]">
              <Icon name="server" className="size-4" />
            </span>
            <h2>System Overview</h2>
          </div>
          {loadingOverview ? (
            <p className="text-sm text-[var(--cp-muted)]">Loading overview...</p>
          ) : (
            <div className="space-y-4">
              {overviewError ? (
                <div className="rounded-xl border border-amber-200 bg-amber-50 px-4 py-3 text-xs text-amber-700">
                  {overviewError}
                </div>
              ) : null}
              <div className="grid gap-3 sm:grid-cols-2">
                <div className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-3">
                  <p className="text-xs uppercase tracking-wide text-[var(--cp-muted)]">Node</p>
                  <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">
                    {overview?.name ?? '—'}
                  </p>
                </div>
                <div className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-3">
                  <p className="text-xs uppercase tracking-wide text-[var(--cp-muted)]">Model</p>
                  <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">
                    {overview?.model ?? '—'}
                  </p>
                </div>
                <div className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-3">
                  <p className="text-xs uppercase tracking-wide text-[var(--cp-muted)]">OS</p>
                  <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">
                    {overview?.os ?? '—'}
                  </p>
                </div>
                <div className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-3">
                  <p className="text-xs uppercase tracking-wide text-[var(--cp-muted)]">Version</p>
                  <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">
                    {overview?.version ?? '—'}
                  </p>
                </div>
                <div className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-3 sm:col-span-2">
                  <p className="text-xs uppercase tracking-wide text-[var(--cp-muted)]">Uptime</p>
                  <p className="mt-1 text-sm font-semibold text-[var(--cp-ink)]">{uptimeLabel}</p>
                </div>
              </div>
            </div>
          )}
        </div>

        <div className="cp-panel p-6">
          <div className="mb-5 flex items-center gap-3 text-lg font-semibold text-[var(--cp-ink)]">
            <span className="inline-flex size-9 items-center justify-center rounded-2xl bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]">
              <Icon name="activity" className="size-4" />
            </span>
            <h2>Live Metrics</h2>
          </div>
          {loadingMetrics ? (
            <p className="text-sm text-[var(--cp-muted)]">Loading metrics...</p>
          ) : (
            <div className="space-y-4">
              {metricsError ? (
                <div className="rounded-xl border border-amber-200 bg-amber-50 px-4 py-3 text-xs text-amber-700">
                  {metricsError}
                </div>
              ) : null}
              <div className="space-y-3">
                {[
                  { label: 'CPU', value: cpuUsage, color: 'bg-[var(--cp-primary)]' },
                  { label: 'Memory', value: memoryUsage, color: 'bg-[var(--cp-accent)]' },
                  { label: 'Disk', value: diskUsage, color: 'bg-emerald-500' },
                ].map((metric) => (
                  <div key={metric.label} className="rounded-xl border border-[var(--cp-border)] bg-white px-4 py-3">
                    <div className="flex items-center justify-between text-xs text-[var(--cp-muted)]">
                      <span className="font-semibold uppercase tracking-wide">{metric.label}</span>
                      <span className="text-[11px]">{metric.value}%</span>
                    </div>
                    <div className="mt-2 h-2 w-full overflow-hidden rounded-full bg-[var(--cp-border)]">
                      <div
                        className={`h-full ${metric.color}`}
                        style={{ width: `${metric.value}%` }}
                      />
                    </div>
                  </div>
                ))}
              </div>
              <div className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-3 text-xs text-[var(--cp-muted)]">
                <p className="font-semibold uppercase tracking-wide text-[var(--cp-ink)]">
                  Disk Capacity
                </p>
                <p className="mt-1">
                  {diskUsed.toFixed(1)} / {diskTotal.toFixed(1)} GB used
                </p>
              </div>
              {diskItems.length ? (
                <div className="rounded-xl border border-[var(--cp-border)] bg-white px-4 py-3 text-xs text-[var(--cp-muted)]">
                  <p className="font-semibold uppercase tracking-wide text-[var(--cp-ink)]">
                    Active Disks
                  </p>
                  <div className="mt-2 space-y-2">
                    {diskItems.slice(0, 3).map((disk) => (
                      <div key={disk.label} className="flex items-center justify-between">
                        <span className="text-[11px] text-[var(--cp-muted)]">{disk.mount}</span>
                        <span className="text-[11px] text-[var(--cp-ink)]">
                          {disk.usedGb.toFixed(1)} / {disk.totalGb.toFixed(1)} GB
                        </span>
                      </div>
                    ))}
                  </div>
                </div>
              ) : null}
            </div>
          )}
        </div>
      </section>

      <section className="cp-panel p-6">
        <div className="mb-5 flex items-center gap-3 text-lg font-semibold text-[var(--cp-ink)]">
          <span className="inline-flex size-9 items-center justify-center rounded-2xl bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]">
            <Icon name="chart" className="size-4" />
          </span>
          <h2>System Config Tree</h2>
        </div>
        <form
          className="flex flex-wrap items-end gap-3"
          onSubmit={handleConfigSubmit}
        >
          <label className="flex min-w-[220px] flex-1 flex-col gap-1 text-xs text-[var(--cp-muted)]">
            Config Key
            <input
              type="text"
              value={configKey}
              onChange={(event) => setConfigKey(event.target.value)}
              placeholder="boot/config"
              className="h-11 rounded-xl border border-[var(--cp-border)] bg-white px-3 text-sm text-[var(--cp-ink)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--cp-primary)]"
            />
          </label>
          <label className="flex w-28 flex-col gap-1 text-xs text-[var(--cp-muted)]">
            Depth
            <input
              type="number"
              min={1}
              max={6}
              value={configDepth}
              onChange={(event) => handleConfigDepthChange(event.target.value)}
              className="h-11 rounded-xl border border-[var(--cp-border)] bg-white px-3 text-sm text-[var(--cp-ink)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--cp-primary)]"
            />
          </label>
          <button
            type="submit"
            className="h-11 rounded-full bg-[var(--cp-primary)] px-5 text-sm font-semibold text-white shadow-sm transition hover:bg-[var(--cp-primary-strong)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--cp-primary)]"
          >
            Load Tree
          </button>
        </form>
        {configError ? (
          <div className="mt-4 rounded-xl border border-amber-200 bg-amber-50 px-4 py-3 text-xs text-amber-700">
            {configError}
          </div>
        ) : null}
        <div className="mt-4 rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] p-4">
          <pre className="max-h-72 overflow-auto whitespace-pre-wrap break-words text-xs text-[var(--cp-ink)]">
            {loadingConfig
              ? 'Loading system config tree...'
              : JSON.stringify(configTree?.tree ?? {}, null, 2)}
          </pre>
        </div>
        <p className="mt-2 text-xs text-[var(--cp-muted)]">
          Key: {configTree?.key ?? configKey} · Depth: {configTree?.depth ?? configDepth}
        </p>
      </section>

      <section className="grid gap-6 xl:grid-cols-[1.25fr_0.75fr]">
        <div className="cp-panel p-6">
          <div className="mb-5 flex items-center gap-3 text-lg font-semibold text-[var(--cp-ink)]">
            <span className="inline-flex size-9 items-center justify-center rounded-2xl bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]">
              <Icon name="settings" className="size-4" />
            </span>
            <h2>Configuration Modules</h2>
          </div>
          <div className="space-y-3">
            {settingsModules.map((module) => (
              <div
                key={module.title}
                className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-4"
              >
                <div className="flex items-start justify-between gap-3">
                  <div className="flex items-start gap-3">
                    <span className="inline-flex size-9 items-center justify-center rounded-2xl bg-white text-[var(--cp-primary-strong)]">
                      <Icon name={module.icon} className="size-4" />
                    </span>
                    <div>
                      <p className="text-sm font-semibold text-[var(--cp-ink)]">{module.title}</p>
                      <p className="text-xs text-[var(--cp-muted)]">{module.description}</p>
                    </div>
                  </div>
                  <span
                    className={`rounded-full px-2.5 py-1 text-[10px] font-semibold uppercase tracking-wide ${
                      module.state === 'ready'
                        ? 'bg-emerald-100 text-emerald-700'
                        : module.state === 'review'
                          ? 'bg-amber-100 text-amber-700'
                          : 'bg-slate-100 text-slate-700'
                    }`}
                  >
                    {module.state}
                  </span>
                </div>
                <div className="mt-3 flex flex-wrap gap-2">
                  {module.controls.map((control) => (
                    <span
                      key={control}
                      className="rounded-full border border-[var(--cp-border)] bg-white px-2.5 py-1 text-[11px] text-[var(--cp-ink)]"
                    >
                      {control}
                    </span>
                  ))}
                </div>
                <div className="mt-3 flex items-center justify-between text-[11px] text-[var(--cp-muted)]">
                  <span>Owner: {module.owner}</span>
                  <span>Updated: {module.updatedAt}</span>
                </div>
              </div>
            ))}
          </div>
        </div>

        <div className="space-y-6">
          <div className="cp-panel p-6">
            <div className="mb-4 flex items-center gap-3 text-lg font-semibold text-[var(--cp-ink)]">
              <span className="inline-flex size-9 items-center justify-center rounded-2xl bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]">
                <Icon name="shield" className="size-4" />
              </span>
              <h2>Policy Baseline</h2>
            </div>
            <div className="space-y-2">
              {policyBaseline.map((policy) => (
                <div
                  key={policy.key}
                  className="flex items-center justify-between gap-3 rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2"
                >
                  <div>
                    <p className="text-xs font-semibold uppercase tracking-wide text-[var(--cp-muted)]">{policy.key}</p>
                    <p className="text-xs text-[var(--cp-ink)]">{policy.value}</p>
                  </div>
                  <span
                    className={`rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide ${
                      policy.tone === 'ready'
                        ? 'bg-emerald-100 text-emerald-700'
                        : policy.tone === 'review'
                          ? 'bg-amber-100 text-amber-700'
                          : 'bg-slate-100 text-slate-700'
                    }`}
                  >
                    {policy.tone}
                  </span>
                </div>
              ))}
            </div>
          </div>

          <div className="cp-panel p-6">
            <div className="mb-4 flex items-center gap-3 text-lg font-semibold text-[var(--cp-ink)]">
              <span className="inline-flex size-9 items-center justify-center rounded-2xl bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]">
                <Icon name="link" className="size-4" />
              </span>
              <h2>Integration Channels</h2>
            </div>
            <div className="space-y-2">
              {integrationChannels.map((channel) => (
                <div
                  key={channel.name}
                  className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2"
                >
                  <div className="flex items-center justify-between gap-3">
                    <p className="text-sm font-semibold text-[var(--cp-ink)]">{channel.name}</p>
                    <span
                      className={`rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide ${
                        channel.state === 'Active'
                          ? 'bg-emerald-100 text-emerald-700'
                          : 'bg-amber-100 text-amber-700'
                      }`}
                    >
                      {channel.state}
                    </span>
                  </div>
                  <p className="mt-1 break-all text-[11px] text-[var(--cp-muted)]">{channel.endpoint}</p>
                </div>
              ))}
            </div>
          </div>
        </div>
      </section>
    </div>
  )
}

export default SettingsPage
