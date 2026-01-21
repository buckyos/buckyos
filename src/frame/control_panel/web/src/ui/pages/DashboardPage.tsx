import type { CSSProperties } from 'react'
import { useEffect, useState } from 'react'
import { Link } from 'react-router-dom'

import { fetchDashboard, fetchSystemMetrics, mockDashboardData } from '@/api'
import Icon from '../icons'

const toneStyles: Record<EventItem['tone'], string> = {
  success: 'bg-emerald-500',
  warning: 'bg-amber-500',
  info: 'bg-sky-500',
}

const DashboardPage = () => {
  const [dashboardData, setDashboardData] = useState<DashboardState | null>(null)
  const [systemMetrics, setSystemMetrics] = useState<SystemMetrics | null>(null)
  const normalizeTimelineTime = (value: string) =>
    value.length === 5 ? `${value}:00` : value

  const [resourceSeries, setResourceSeries] = useState<ResourcePoint[]>(
    mockDashboardData.resourceTimeline.map((point) => ({
      ...point,
      time: normalizeTimelineTime(point.time),
    })),
  )
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    let cancelled = false
    const loadDashboard = async () => {
      const { data, error } = await fetchDashboard()
      if (!cancelled) {
        setDashboardData(data ?? mockDashboardData)
        if (data?.resourceTimeline?.length) {
          setResourceSeries(
            data.resourceTimeline
              .slice(-6)
              .map((point) => ({
                ...point,
                time: normalizeTimelineTime(point.time),
              })),
          )
        }
        if (error) {
          // eslint-disable-next-line no-console
          console.warn('Dashboard API unavailable, using mock data', error)
        }
        setLoading(false)
      }
    }

    loadDashboard()
    return () => {
      cancelled = true
    }
  }, [])

  useEffect(() => {
    let cancelled = false
    const formatTimelineTime = (date: Date) =>
      `${date.getHours().toString().padStart(2, '0')}:${date
        .getMinutes()
        .toString()
        .padStart(2, '0')}:${date.getSeconds().toString().padStart(2, '0')}`

    const loadMetrics = async () => {
      const { data, error } = await fetchSystemMetrics({ lite: true })
      if (!cancelled) {
        if (error) {
          // eslint-disable-next-line no-console
          console.warn('System metrics API unavailable', error)
          return
        }
        if (data) {
          setSystemMetrics((prev) => ({
            cpu: data.cpu,
            memory: data.memory,
            disk: prev?.disk ?? data.disk,
            network: prev?.network ?? data.network,
          }))
          const cpuValue = Math.round(data.cpu?.usagePercent ?? 0)
          const memoryValue = Math.round(data.memory?.usagePercent ?? 0)
          const time = formatTimelineTime(new Date())
          setResourceSeries((prev) => {
            const trimmed = prev.length >= 6 ? prev.slice(prev.length - 5) : prev
            return [
              ...trimmed,
              {
                time,
                cpu: Math.max(0, Math.min(cpuValue, 100)),
                memory: Math.max(0, Math.min(memoryValue, 100)),
              },
            ]
          })
        }
      }
    }

    loadMetrics()
    const intervalId = window.setInterval(loadMetrics, 5000)
    return () => {
      cancelled = true
      window.clearInterval(intervalId)
    }
  }, [])

  const recentEvents = dashboardData?.recentEvents ?? []
  const quickActions = dashboardData?.quickActions ?? []
  const resourceTimeline = resourceSeries
  const storageSlices = dashboardData?.storageSlices ?? []
  const storageCapacityGb = dashboardData?.storageCapacityGb ?? systemMetrics?.disk?.totalGb ?? 0
  const storageUsedGb = dashboardData?.storageUsedGb ?? systemMetrics?.disk?.usedGb ?? 0
  const devices = dashboardData?.devices ?? []
  const memoryInfo = systemMetrics?.memory ?? dashboardData?.memory
  const cpuInfo = systemMetrics?.cpu ?? dashboardData?.cpu
  const disks = dashboardData?.disks ?? systemMetrics?.disk?.disks ?? []

  const totalMemoryGb = memoryInfo?.totalGb ?? 0
  const usedMemoryGb = memoryInfo?.usedGb ?? 0
  const memoryPercent = Math.round(memoryInfo?.usagePercent ?? 0)
  const cpuPercent = Math.round(cpuInfo?.usagePercent ?? 0)
  const cpuModel = cpuInfo?.model ?? 'Unknown CPU'
  const cpuCores = cpuInfo?.cores ?? 0

  const chartTimeline = resourceTimeline.slice(-6)
  const cpuUsage = chartTimeline.at(-1)?.cpu ?? 0
  const memoryUsage = chartTimeline.at(-1)?.memory ?? 0
  const resourceLinePoints = chartTimeline
    .map((point, index) => {
      const denominator = Math.max(chartTimeline.length - 1, 1)
      const x = (index / denominator) * 100
      const cpuY = 100 - point.cpu
      const memoryY = 100 - point.memory
      return { x, cpuY, memoryY }
    })
    .reduce<{ cpu: string; memory: string }>(
      (acc, { x, cpuY, memoryY }) => ({
        cpu: `${acc.cpu}${acc.cpu ? ' ' : ''}${x},${cpuY}`,
        memory: `${acc.memory}${acc.memory ? ' ' : ''}${x},${memoryY}`,
      }),
      { cpu: '', memory: '' },
    )

  const storageSlicesTotal = storageSlices.reduce((sum, slice) => sum + slice.value, 0) || 1
  const storageBarSegments = storageSlices.map((slice) => ({
    ...slice,
    width: `${(slice.value / storageSlicesTotal) * 100}%`,
  }))

  const storageDonutStyle: CSSProperties = {
    background: `conic-gradient(${storageSlices
      .map((slice, index) => {
        const start =
          storageSlices.slice(0, index).reduce((sum, current) => sum + current.value, 0) /
          storageSlicesTotal
        const end = start + slice.value / storageSlicesTotal
        return `${slice.color} ${start * 360}deg ${end * 360}deg`
      })
      .join(', ')})`,
  }

  if (loading) {
    return (
      <div className="cp-panel flex min-h-[60vh] items-center justify-center px-8 py-12">
        <div className="flex items-center gap-3 text-[var(--cp-muted)]">
          <span
            className="size-3 animate-pulse rounded-full bg-[var(--cp-primary)]"
            aria-hidden
          />
          <span className="text-sm">Loading dashboard...</span>
        </div>
      </div>
    )
  }

  return (
    <div className="space-y-6">
      <header className="cp-panel px-8 py-6">
        <div className="flex flex-wrap items-center justify-between gap-4">
          <div>
            <h1 className="text-2xl font-semibold text-[var(--cp-ink)] sm:text-3xl">
              System Overview
            </h1>
            <p className="text-sm text-[var(--cp-muted)]">
              Monitor your decentralized personal cloud infrastructure
            </p>
          </div>
          <div className="cp-pill bg-emerald-100 text-emerald-700">
            <span className="inline-flex size-2 rounded-full bg-emerald-500" aria-hidden />
            System Online
          </div>
        </div>
      </header>

      <section className="cp-panel p-6">
          <div className="mb-6 flex items-center gap-3 text-lg font-semibold text-[var(--cp-ink)]">
            <span className="inline-flex size-9 items-center justify-center rounded-2xl bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]">
              <Icon name="activity" className="size-4" />
            </span>
            <h2>System Resources</h2>
          </div>
          <div className="grid gap-4 lg:grid-cols-[1fr_auto]">
            <div className="space-y-4">
              <div className="flex flex-wrap gap-3 text-sm text-[var(--cp-muted)]">
                <div className="flex min-w-[180px] flex-1 items-center gap-3 rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-2">
                  <span className="inline-flex size-2 rounded-full bg-[var(--cp-primary)]" aria-hidden />
                  <span className="flex-1 font-medium text-[var(--cp-ink)]">CPU</span>
                  <span className="text-xs uppercase tracking-wide text-[var(--cp-muted)]">
                    {cpuPercent || cpuUsage}%
                  </span>
                </div>
                <div className="flex min-w-[180px] flex-1 items-center gap-3 rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-2">
                  <span className="inline-flex size-2 rounded-full bg-[var(--cp-accent)]" aria-hidden />
                  <span className="flex-1 font-medium text-[var(--cp-ink)]">Memory</span>
                  <span className="text-xs uppercase tracking-wide text-[var(--cp-muted)]">
                    {memoryPercent || memoryUsage}%
                  </span>
                </div>
              </div>
              <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
                <svg viewBox="0 0 100 60" className="h-40 w-full text-[var(--cp-border)]">
                  <rect x="0" y="0" width="100" height="60" fill="transparent" />
                  {[20, 40, 60, 80].map((value) => (
                    <line
                      // eslint-disable-next-line react/no-array-index-key
                      key={value}
                      x1="0"
                      y1={60 - value * 0.6}
                      x2="100"
                      y2={60 - value * 0.6}
                      stroke="currentColor"
                      strokeWidth="0.4"
                      strokeDasharray="2"
                    />
                  ))}
                  <polyline
                    points={resourceLinePoints.cpu}
                    fill="none"
                    stroke="#0f766e"
                    strokeWidth="2"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  />
                  <polyline
                    points={resourceLinePoints.memory}
                    fill="none"
                    stroke="#f59e0b"
                    strokeWidth="2"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  />
                </svg>
                <div className="mt-3 grid grid-cols-6 gap-2 overflow-hidden text-xs text-[var(--cp-muted)]">
                  {chartTimeline.map((point, index) => (
                    <span key={`${point.time}-${index}`} className="truncate text-center">
                      {point.time}
                    </span>
                  ))}
                </div>
              </div>
            </div>
            <div className="flex flex-col justify-between gap-6 text-sm text-[var(--cp-muted)]">
              <div className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-5 text-center">
                <p className="text-xs uppercase tracking-wide text-[var(--cp-muted)]">CPU Usage</p>
                <p className="mt-2 text-3xl font-semibold text-[var(--cp-ink)]">
                  {cpuPercent || cpuUsage}%
                </p>
                <p className="text-xs text-emerald-600">Live</p>
                <p className="mt-1 truncate text-[11px] text-[var(--cp-muted)]">
                  {cpuModel}
                  {cpuCores ? ` - ${cpuCores} cores` : ''}
                </p>
              </div>
              <div className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-5 text-center">
                <p className="text-xs uppercase tracking-wide text-[var(--cp-muted)]">Memory Usage</p>
                <p className="mt-2 text-3xl font-semibold text-[var(--cp-ink)]">
                  {memoryPercent || memoryUsage}%
                </p>
                <p className="text-xs text-[var(--cp-primary)]">Live</p>
                <p className="mt-1 text-[11px] text-[var(--cp-muted)]">
                  {usedMemoryGb.toFixed(1)} / {totalMemoryGb.toFixed(1)} GB
                </p>
              </div>
            </div>
          </div>
      </section>

      <section className="cp-panel p-6">
        <div className="mb-6 flex items-center gap-3 text-lg font-semibold text-[var(--cp-ink)]">
          <span className="inline-flex size-9 items-center justify-center rounded-2xl bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]">
            <Icon name="drive" className="size-4" />
          </span>
          <h2>Storage Overview</h2>
        </div>
        <div className="space-y-6 text-sm text-[var(--cp-muted)]">
          <div className="flex items-center justify-between text-xs uppercase tracking-wide text-[var(--cp-muted)]">
            <span>Used Space</span>
            <span className="text-[var(--cp-ink)]">
              {(storageUsedGb / 1024).toFixed(1)}TB / {(storageCapacityGb / 1024).toFixed(1)}TB
            </span>
          </div>
          <div className="flex h-2 overflow-hidden rounded-full bg-[var(--cp-surface-muted)]">
            {storageBarSegments.map((segment) => (
              <span
                key={segment.label}
                style={{ width: segment.width, backgroundColor: segment.color }}
                className="h-full"
              />
            ))}
          </div>
          <div className="flex flex-col items-center gap-6 lg:flex-row lg:items-center lg:justify-between">
            <div className="flex items-center gap-5">
              <div
                style={storageDonutStyle}
                className="relative flex size-24 items-center justify-center rounded-full"
              >
                <div className="size-10 rounded-full bg-white" />
              </div>
              <div className="space-y-2 text-xs text-[var(--cp-muted)]">
                {storageSlices.map((slice) => (
                  <div key={slice.label} className="flex items-center gap-2">
                    <span
                      className="inline-flex size-2 rounded-full"
                      style={{ backgroundColor: slice.color }}
                    />
                    <span className="w-20 text-[var(--cp-ink)]">{slice.label}</span>
                    <span>{slice.value}%</span>
                  </div>
                ))}
              </div>
            </div>
            <div className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-5 py-4 text-xs text-[var(--cp-muted)]">
              <div className="flex justify-between text-[var(--cp-ink)]">
                <span>Snapshots</span>
                <span>12</span>
              </div>
              <div className="flex justify-between text-[var(--cp-ink)]">
                <span>Replication</span>
                <span>Enabled</span>
              </div>
              <div className="mt-2 rounded-lg bg-[var(--cp-primary-soft)] px-3 py-2 text-[var(--cp-primary-strong)]">
                4 nodes synced
              </div>
            </div>
          </div>
        </div>
      </section>

      <section className="cp-panel p-6">
        <div className="flex items-center justify-between">
          <h2 className="text-lg font-semibold text-[var(--cp-ink)]">Recent Events</h2>
          <span className="text-xs uppercase tracking-wide text-[var(--cp-muted)]">Last 24h</span>
        </div>
        <div className="mt-5 max-h-64 space-y-3 overflow-y-auto pr-1">
          {recentEvents.map((item) => (
            <div
              key={item.title}
              className="flex items-start gap-3 rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-3 text-sm text-[var(--cp-muted)]"
            >
              <span
                className={`mt-1 inline-flex size-2 rounded-full ${toneStyles[item.tone]}`}
                aria-hidden
              />
              <div className="flex-1">
                <p className="font-medium text-[var(--cp-ink)]">{item.title}</p>
                <p className="text-xs text-[var(--cp-muted)]">{item.subtitle}</p>
              </div>
            </div>
          ))}
        </div>
      </section>

      <section className="grid gap-6 lg:grid-cols-2">
        <div className="cp-panel p-6 lg:col-span-2">
          <div className="mb-6 flex items-center gap-3 text-lg font-semibold text-[var(--cp-ink)]">
            <span className="inline-flex size-9 items-center justify-center rounded-2xl bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]">
              <Icon name="network" className="size-4" />
            </span>
            <h2>Network Status</h2>
          </div>
          <div className="space-y-6 text-sm text-[var(--cp-muted)]">
            <div>
              <div className="mb-1 flex justify-between text-xs uppercase tracking-wide text-[var(--cp-muted)]">
                <span>Upload</span>
                <span className="text-[var(--cp-ink)]">12.5 MB/s</span>
              </div>
              <div className="h-2 rounded-full bg-[var(--cp-surface-muted)]">
                <div className="h-full w-3/5 rounded-full bg-[var(--cp-primary)]" />
              </div>
            </div>
            <div>
              <div className="mb-1 flex justify-between text-xs uppercase tracking-wide text-[var(--cp-muted)]">
                <span>Download</span>
                <span className="text-[var(--cp-ink)]">24.8 MB/s</span>
              </div>
              <div className="h-2 rounded-full bg-[var(--cp-surface-muted)]">
                <div className="h-full w-4/5 rounded-full bg-[var(--cp-accent)]" />
              </div>
            </div>
            <div className="grid grid-cols-2 gap-4 text-xs text-[var(--cp-muted)]">
              <div>
                <p className="uppercase tracking-wide">Connected Peers</p>
                <p className="mt-1 text-lg font-semibold text-[var(--cp-ink)]">847</p>
              </div>
              <div>
                <p className="uppercase tracking-wide">Active Connections</p>
                <p className="mt-1 text-lg font-semibold text-[var(--cp-ink)]">23</p>
              </div>
            </div>
          </div>
        </div>

        <div className="cp-panel p-6">
          <div className="mb-6 flex items-center gap-3 text-lg font-semibold text-[var(--cp-ink)]">
            <span className="inline-flex size-9 items-center justify-center rounded-2xl bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]">
              <Icon name="storage" className="size-4" />
            </span>
            <h2>Storage</h2>
          </div>
          <div className="space-y-4 text-sm text-[var(--cp-muted)]">
            <div className="flex items-center justify-between text-xs uppercase tracking-wide text-[var(--cp-muted)]">
              <span>Capacity</span>
              <span className="text-[var(--cp-ink)]">{storageCapacityGb.toFixed(0)} GB</span>
            </div>
            <div className="flex items-center justify-between text-xs uppercase tracking-wide text-[var(--cp-muted)]">
              <span>Used</span>
              <span className="text-[var(--cp-ink)]">{storageUsedGb.toFixed(0)} GB</span>
            </div>
            <div className="flex h-2 overflow-hidden rounded-full bg-[var(--cp-surface-muted)]">
              <span
                className="block h-full rounded-full bg-[var(--cp-primary)]"
                style={{
                  width: `${storageCapacityGb ? Math.min((storageUsedGb / storageCapacityGb) * 100, 100) : 0}%`,
                }}
              />
            </div>
            <div className="space-y-2 text-xs text-[var(--cp-muted)]">
              {disks.map((disk: any) => (
                <div
                  key={`${disk.mount}-${disk.label}`}
                  className="flex items-center justify-between rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2"
                >
                  <div className="text-xs text-[var(--cp-muted)]">
                    <p className="text-[var(--cp-ink)]">{disk.label}</p>
                    <p>{disk.mount}</p>
                  </div>
                  <div className="text-xs text-[var(--cp-muted)]">
                    <span className="text-[var(--cp-ink)]">{disk.usedGb.toFixed(1)} GB</span>
                    <span className="text-[var(--cp-muted)]">
                      {' '}
                      / {disk.totalGb.toFixed(1)} GB
                    </span>
                  </div>
                </div>
              ))}
            </div>
          </div>
        </div>

        <div className="cp-panel p-6">
          <div className="mb-6 flex items-center gap-3 text-lg font-semibold text-[var(--cp-ink)]">
            <span className="inline-flex size-9 items-center justify-center rounded-2xl bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]">
              <Icon name="spark" className="size-4" />
            </span>
            <h2>Quick Actions</h2>
          </div>
          <div className="space-y-3 text-sm text-[var(--cp-ink)]">
            {quickActions.map((action) => (
              <Link
                key={action.to}
                to={action.to}
                className="flex w-full items-center gap-3 rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-3 text-left transition hover:bg-white"
              >
                <span className="inline-flex size-9 items-center justify-center rounded-xl bg-white text-[var(--cp-primary-strong)] shadow-sm">
                  <Icon name={action.icon} className="size-4" />
                </span>
                <span>{action.label}</span>
              </Link>
            ))}
          </div>
        </div>
      </section>

      <section className="cp-panel p-6">
        <div className="mb-2 flex items-center gap-3 text-lg font-semibold text-[var(--cp-ink)]">
          <span className="inline-flex size-9 items-center justify-center rounded-2xl bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]">
            <Icon name="server" className="size-4" />
          </span>
          <h2>Hardware</h2>
        </div>
        <p className="mb-4 text-xs text-[var(--cp-muted)]">
          CPU: {cpuModel}
          {cpuCores ? ` - ${cpuCores} cores` : ''}
        </p>
        <p className="mb-4 text-xs text-[var(--cp-muted)]">
          Memory: {usedMemoryGb.toFixed(1)} / {totalMemoryGb.toFixed(1)} GB
        </p>
        <div className="grid gap-3 text-sm text-[var(--cp-muted)] md:grid-cols-2">
          {(devices as any[]).map((device) => (
            <div
              key={device.name}
              className="flex flex-col gap-2 rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] p-4"
            >
              <div className="flex items-center justify-between">
                <div>
                  <p className="text-[var(--cp-ink)]">{device.name}</p>
                  <p className="text-xs text-[var(--cp-muted)] capitalize">{device.role}</p>
                </div>
                <span
                  className={[
                    'cp-pill uppercase tracking-wide',
                    device.status === 'online'
                      ? 'bg-emerald-100 text-emerald-700'
                      : 'bg-amber-100 text-amber-700',
                  ].join(' ')}
                >
                  {device.status}
                </span>
              </div>
              <div className="flex items-center gap-2 text-xs text-[var(--cp-ink)]">
                <span className="cp-pill bg-white text-[var(--cp-ink)]">
                  CPU {device.cpu ?? cpuPercent ?? cpuUsage}%
                </span>
                <span className="cp-pill bg-white text-[var(--cp-ink)]">
                  Mem {device.memory ?? memoryPercent ?? memoryUsage}%
                </span>
              </div>
              <div className="text-[11px] uppercase tracking-wide text-[var(--cp-muted)]">
                Uptime {device.uptimeHours ?? 0}h
              </div>
            </div>
          ))}
        </div>
      </section>
    </div>
  )
}

export default DashboardPage
