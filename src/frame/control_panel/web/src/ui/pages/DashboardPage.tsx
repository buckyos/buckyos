import { useEffect, useState } from 'react'
import { Link } from 'react-router-dom'

import {
  fetchDashboard,
  fetchSystemMetrics,
  fetchSystemStatus,
  mockDashboardData,
  mockSystemMetrics,
  mockSystemStatus,
} from '@/api'
import Icon from '../icons'
import { NetworkTrendChart, ResourceTrendChart } from '../components/MonitorTrendCharts'
import DonutChart from '../charts/DonutChart'
import HorizontalBarChart from '../charts/HorizontalBarChart'

const DashboardPage = () => {
  const normalizeTimelineTime = (value: string) =>
    value.length === 5 ? `${value}:00` : value

  const formatTimelineTime = (date: Date) =>
    `${date.getHours().toString().padStart(2, '0')}:${date
      .getMinutes()
      .toString()
      .padStart(2, '0')}:${date.getSeconds().toString().padStart(2, '0')}`

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

  const formatBytes = (value: number) => {
    if (value <= 0) {
      return '0 B'
    }
    const units = ['B', 'KB', 'MB', 'GB', 'TB']
    const index = Math.min(Math.floor(Math.log(value) / Math.log(1024)), units.length - 1)
    const scaled = value / 1024 ** index
    return `${scaled.toFixed(scaled >= 100 || index === 0 ? 0 : 1)} ${units[index]}`
  }

  const formatRate = (value: number) => `${formatBytes(value)}/s`

  const [dashboardData, setDashboardData] = useState<DashboardState | null>(null)
  const [systemMetrics, setSystemMetrics] = useState<SystemMetrics | null>(mockSystemMetrics)
  const [systemStatus, setSystemStatus] = useState<SystemStatusResponse | null>(mockSystemStatus)
  const [statusError, setStatusError] = useState<unknown>(null)

  const [resourceSeries, setResourceSeries] = useState<ResourcePoint[]>(
    mockDashboardData.resourceTimeline.map((point) => ({
      ...point,
      time: normalizeTimelineTime(point.time),
    })),
  )
  const [networkSeries, setNetworkSeries] = useState<NetworkPoint[]>(
    mockDashboardData.resourceTimeline.map((point) => ({
      time: normalizeTimelineTime(point.time),
      rx: 0,
      tx: 0,
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
    const loadMetrics = async () => {
      const { data, error } = await fetchSystemMetrics({ lite: true })
      if (!cancelled) {
        if (error) {
          // eslint-disable-next-line no-console
          console.warn('System metrics API unavailable', error)
        }
        if (!data) {
          return
        }
        setSystemMetrics((prev) => ({
          ...prev,
          ...data,
          cpu: data.cpu ?? prev?.cpu ?? mockSystemMetrics.cpu,
          memory: data.memory ?? prev?.memory ?? mockSystemMetrics.memory,
          disk: data.disk ?? prev?.disk ?? mockSystemMetrics.disk,
          network: data.network ?? prev?.network ?? mockSystemMetrics.network,
        }))
        if (data.resourceTimeline?.length) {
          setResourceSeries(
            data.resourceTimeline.slice(-6).map((point) => ({
              time: normalizeTimelineTime(point.time),
              cpu: Math.max(0, Math.min(Math.round(point.cpu), 100)),
              memory: Math.max(0, Math.min(Math.round(point.memory), 100)),
            })),
          )
        } else {
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

        if (data.networkTimeline?.length) {
          setNetworkSeries(
            data.networkTimeline.slice(-6).map((point) => ({
              time: normalizeTimelineTime(point.time),
              rx: Math.max(0, Math.round(point.rx)),
              tx: Math.max(0, Math.round(point.tx)),
            })),
          )
        } else {
          const time = formatTimelineTime(new Date())
          const rxPerSec = data.network?.rxPerSec ?? 0
          const txPerSec = data.network?.txPerSec ?? 0
          setNetworkSeries((prev) => {
            const trimmed = prev.length >= 6 ? prev.slice(prev.length - 5) : prev
            return [
              ...trimmed,
              {
                time,
                rx: Math.round(rxPerSec),
                tx: Math.round(txPerSec),
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

  useEffect(() => {
    let cancelled = false
    const loadStatus = async () => {
      const { data, error } = await fetchSystemStatus()
      if (!cancelled) {
        if (error) {
          // eslint-disable-next-line no-console
          console.warn('System status API unavailable', error)
          setStatusError(error)
        } else {
          setStatusError(null)
        }
        if (data) {
          setSystemStatus(data)
        }
      }
    }

    loadStatus()
    const intervalId = window.setInterval(loadStatus, 15000)
    return () => {
      cancelled = true
      window.clearInterval(intervalId)
    }
  }, [])

  const quickActions = dashboardData?.quickActions ?? []
  const resourceTimeline = resourceSeries
  const rawStorageSlices = dashboardData?.storageSlices ?? []
  const storageCapacityGb = dashboardData?.storageCapacityGb ?? systemMetrics?.disk?.totalGb ?? 0
  const storageUsedGb = dashboardData?.storageUsedGb ?? systemMetrics?.disk?.usedGb ?? 0
  const storageUsedPercent = storageCapacityGb
    ? Math.min((storageUsedGb / storageCapacityGb) * 100, 100)
    : 0
  const storageSlices = rawStorageSlices.length
    ? rawStorageSlices
    : [
        { label: 'Used', value: Math.round(storageUsedPercent), color: 'var(--cp-primary)' },
        {
          label: 'Free',
          value: Math.max(0, Math.round(100 - storageUsedPercent)),
          color: 'var(--cp-border)',
        },
      ]
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
  const swapInfo = systemMetrics?.swap
  const swapPercent = Math.round(swapInfo?.usagePercent ?? 0)
  const swapUsedGb = swapInfo?.usedGb ?? 0
  const swapTotalGb = swapInfo?.totalGb ?? 0
  const loadAverage = systemMetrics?.loadAverage
  const processCount = systemMetrics?.processCount ?? 0
  const uptimeSeconds = systemMetrics?.uptimeSeconds ?? 0
  const statusState = systemStatus?.state ?? 'online'
  const statusWarnings = systemStatus?.warnings ?? []
  const statusServices = systemStatus?.services ?? []
  const statusLabels: Record<SystemStatusResponse['state'], string> = {
    online: 'System Online',
    warning: 'Attention Needed',
    critical: 'Critical Alerts',
  }
  const statusPillStyles: Record<SystemStatusResponse['state'], string> = {
    online: 'bg-emerald-100 text-emerald-700',
    warning: 'bg-amber-100 text-amber-700',
    critical: 'bg-rose-100 text-rose-700',
  }

  const chartTimeline = resourceTimeline.slice(-6)
  const cpuUsage = chartTimeline.at(-1)?.cpu ?? 0
  const memoryUsage = chartTimeline.at(-1)?.memory ?? 0
  const networkTimeline = networkSeries.slice(-6)
  const latestNetworkPoint = networkTimeline.at(-1)
  const rxRate = latestNetworkPoint?.rx ?? 0
  const txRate = latestNetworkPoint?.tx ?? 0
  const totalRxBytes = systemMetrics?.network?.rxBytes ?? 0
  const totalTxBytes = systemMetrics?.network?.txBytes ?? 0

  const storageSlicesTotal = storageSlices.reduce((sum, slice) => sum + slice.value, 0) || 1
  const storageBarSegments = storageSlices.map((slice) => ({
    ...slice,
    width: `${(slice.value / storageSlicesTotal) * 100}%`,
  }))
  const storageDonutData = storageSlices.map((slice) => ({
    id: slice.label,
    label: slice.label,
    value: slice.value,
    color: slice.color,
  }))
  const diskUsageData = disks.length
    ? disks.map((disk) => ({
        label: disk.mount ? `${disk.label} (${disk.mount})` : disk.label,
        usage: Math.round(
          disk.usagePercent ?? (disk.totalGb ? (disk.usedGb / disk.totalGb) * 100 : 0),
        ),
      }))
    : [{ label: 'No disks', usage: 0 }]

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
          <div className={`cp-pill ${statusPillStyles[statusState]}`}>
            <span
              className={`inline-flex size-2 rounded-full ${
                statusState === 'critical'
                  ? 'bg-rose-500'
                  : statusState === 'warning'
                    ? 'bg-amber-500'
                    : 'bg-emerald-500'
              }`}
              aria-hidden
            />
            {statusLabels[statusState]}
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
        <div className="grid gap-6 lg:grid-cols-[2fr_1fr]">
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
              <ResourceTrendChart timeline={chartTimeline} height={220} />
            </div>
          </div>
          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-1">
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
            <div className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-5 text-center">
              <p className="text-xs uppercase tracking-wide text-[var(--cp-muted)]">Swap Usage</p>
              <p className="mt-2 text-3xl font-semibold text-[var(--cp-ink)]">
                {swapPercent}%
              </p>
              <p className="text-xs text-amber-600">Tracked</p>
              <p className="mt-1 text-[11px] text-[var(--cp-muted)]">
                {swapUsedGb.toFixed(1)} / {swapTotalGb.toFixed(1)} GB
              </p>
            </div>
            <div className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-5 text-center">
              <p className="text-xs uppercase tracking-wide text-[var(--cp-muted)]">Load Average</p>
              <p className="mt-2 text-3xl font-semibold text-[var(--cp-ink)]">
                {loadAverage?.one?.toFixed(2) ?? '0.00'}
              </p>
              <p className="text-xs text-[var(--cp-primary)]">1m / 5m / 15m</p>
              <p className="mt-1 text-[11px] text-[var(--cp-muted)]">
                {loadAverage?.five?.toFixed(2) ?? '0.00'} / {loadAverage?.fifteen?.toFixed(2) ?? '0.00'}
              </p>
            </div>
          </div>
        </div>
      </section>

      <section className="grid gap-6 lg:grid-cols-[1.2fr_0.8fr]">
        <div className="cp-panel p-6">
          <div className="mb-5 flex items-center gap-3 text-lg font-semibold text-[var(--cp-ink)]">
            <span className="inline-flex size-9 items-center justify-center rounded-2xl bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]">
              <Icon name="shield" className="size-4" />
            </span>
            <h2>System Health</h2>
          </div>
          <div className="flex flex-wrap items-center justify-between gap-4">
            <div>
              <p className="text-xs uppercase tracking-wide text-[var(--cp-muted)]">Overall Status</p>
              <p className="mt-1 text-lg font-semibold text-[var(--cp-ink)]">
                {statusLabels[statusState]}
              </p>
            </div>
            <div className={`cp-pill ${statusPillStyles[statusState]}`}>
              <span
                className={`inline-flex size-2 rounded-full ${
                  statusState === 'critical'
                    ? 'bg-rose-500'
                    : statusState === 'warning'
                      ? 'bg-amber-500'
                      : 'bg-emerald-500'
                }`}
                aria-hidden
              />
              {statusState.toUpperCase()}
            </div>
          </div>
          {statusError ? (
            <div className="mt-4 rounded-xl border border-amber-200 bg-amber-50 px-4 py-3 text-xs text-amber-700">
              System status data is unavailable. Showing last known values.
            </div>
          ) : null}
          <div className="mt-4 space-y-3">
            {statusWarnings.length ? (
              statusWarnings.map((warning) => (
                <div
                  key={`${warning.label}-${warning.message}`}
                  className="flex items-start justify-between gap-3 rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-3 text-sm"
                >
                  <div>
                    <p className="font-medium text-[var(--cp-ink)]">{warning.label}</p>
                    <p className="text-xs text-[var(--cp-muted)]">{warning.message}</p>
                  </div>
                  <span
                    className={`cp-pill uppercase tracking-wide ${
                      warning.severity === 'critical'
                        ? 'bg-rose-100 text-rose-700'
                        : 'bg-amber-100 text-amber-700'
                    }`}
                  >
                    {warning.severity}
                  </span>
                </div>
              ))
            ) : (
              <div className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-3 text-sm text-[var(--cp-muted)]">
                No active alerts detected.
              </div>
            )}
          </div>
          <div className="mt-6 grid gap-4 sm:grid-cols-2">
            <div className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-3">
              <p className="text-xs uppercase tracking-wide text-[var(--cp-muted)]">Uptime</p>
              <p className="mt-2 text-lg font-semibold text-[var(--cp-ink)]">
                {uptimeSeconds >= 0 ? formatUptime(uptimeSeconds) : '—'}
              </p>
            </div>
            <div className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-3">
              <p className="text-xs uppercase tracking-wide text-[var(--cp-muted)]">Processes</p>
              <p className="mt-2 text-lg font-semibold text-[var(--cp-ink)]">{processCount}</p>
            </div>
          </div>
        </div>
        <div className="cp-panel p-6">
          <div className="mb-5 flex items-center gap-3 text-lg font-semibold text-[var(--cp-ink)]">
            <span className="inline-flex size-9 items-center justify-center rounded-2xl bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]">
              <Icon name="server" className="size-4" />
            </span>
            <h2>Core Services</h2>
          </div>
          <div className="space-y-3 text-sm">
            {statusServices.length ? (
              statusServices.slice(0, 6).map((service) => (
                <div
                  key={service.name}
                  className="flex items-center justify-between rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-3"
                >
                  <span className="font-medium text-[var(--cp-ink)]">{service.name}</span>
                  <span
                    className={`cp-pill uppercase tracking-wide ${
                      service.status === 'running'
                        ? 'bg-emerald-100 text-emerald-700'
                        : service.status === 'stopped'
                          ? 'bg-rose-100 text-rose-700'
                          : 'bg-slate-100 text-slate-600'
                    }`}
                  >
                    {service.status}
                  </span>
                </div>
              ))
            ) : (
              <div className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-3 text-sm text-[var(--cp-muted)]">
                No service status data available yet.
              </div>
            )}
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
          <div className="grid gap-6 lg:grid-cols-[1.1fr_0.9fr]">
            <div className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] p-4">
              <div className="flex flex-col items-center gap-6 sm:flex-row sm:items-center">
                <div className="w-44">
                  <DonutChart data={storageDonutData} height={180} />
                </div>
                <div className="space-y-2 text-xs text-[var(--cp-muted)]">
                  {storageSlices.map((slice) => (
                    <div key={slice.label} className="flex items-center gap-2">
                      <span
                        className="inline-flex size-2 rounded-full"
                        style={{ backgroundColor: slice.color }}
                      />
                      <span className="w-24 text-[var(--cp-ink)]">{slice.label}</span>
                      <span>{slice.value}%</span>
                    </div>
                  ))}
                </div>
              </div>
              <div className="mt-4 rounded-xl border border-[var(--cp-border)] bg-white px-4 py-3 text-xs text-[var(--cp-muted)]">
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
            <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
              <div className="mb-3 flex items-center justify-between text-xs uppercase tracking-wide text-[var(--cp-muted)]">
                <span>Disk Utilization</span>
                <span className="text-[var(--cp-ink)]">{storageUsedPercent.toFixed(1)}% used</span>
              </div>
              <HorizontalBarChart
                data={diskUsageData}
                keys={['usage']}
                indexBy="label"
                height={220}
                maxValue={100}
                colors={['var(--cp-primary)']}
                axisBottom={{ tickSize: 0, tickPadding: 8, tickValues: [0, 25, 50, 75, 100] }}
                axisLeft={{ tickSize: 0, tickPadding: 8 }}
              />
              <div className="mt-2 flex items-center justify-between text-xs text-[var(--cp-muted)]">
                <span>{disks.length} disks tracked</span>
                <span className="text-[var(--cp-ink)]">{storageUsedGb.toFixed(0)} GB used</span>
              </div>
            </div>
          </div>
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
          <div className="grid gap-6 lg:grid-cols-[1.6fr_1fr]">
            <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
              <NetworkTrendChart timeline={networkTimeline} height={200} />
            </div>
            <div className="space-y-4 text-sm text-[var(--cp-muted)]">
              <div className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-3">
                <div className="flex items-center justify-between text-xs uppercase tracking-wide text-[var(--cp-muted)]">
                  <span>Download</span>
                  <span className="text-[var(--cp-ink)]">{formatRate(rxRate)}</span>
                </div>
                <div className="mt-2 flex items-center justify-between text-xs">
                  <span>Total received</span>
                  <span className="text-[var(--cp-ink)]">{formatBytes(totalRxBytes)}</span>
                </div>
              </div>
              <div className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-3">
                <div className="flex items-center justify-between text-xs uppercase tracking-wide text-[var(--cp-muted)]">
                  <span>Upload</span>
                  <span className="text-[var(--cp-ink)]">{formatRate(txRate)}</span>
                </div>
                <div className="mt-2 flex items-center justify-between text-xs">
                  <span>Total sent</span>
                  <span className="text-[var(--cp-ink)]">{formatBytes(totalTxBytes)}</span>
                </div>
              </div>
              <div className="grid grid-cols-2 gap-4 text-xs text-[var(--cp-muted)]">
                <div className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2">
                  <p className="uppercase tracking-wide">Current RX</p>
                  <p className="mt-1 text-lg font-semibold text-[var(--cp-ink)]">
                    {formatRate(rxRate)}
                  </p>
                </div>
                <div className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2">
                  <p className="uppercase tracking-wide">Current TX</p>
                  <p className="mt-1 text-lg font-semibold text-[var(--cp-ink)]">
                    {formatRate(txRate)}
                  </p>
                </div>
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
              {disks.map((disk) => {
                const usagePercent =
                  disk.usagePercent ??
                  (disk.totalGb ? Math.round((disk.usedGb / disk.totalGb) * 100) : 0)
                return (
                  <div
                    key={`${disk.mount}-${disk.label}`}
                    className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-3"
                  >
                    <div className="flex items-start justify-between gap-3">
                      <div className="min-w-0 flex-1">
                        <p className="truncate text-[var(--cp-ink)]" title={disk.label}>
                          {disk.label}
                        </p>
                        <p className="truncate text-[11px] text-[var(--cp-muted)]" title={disk.mount}>
                          {disk.mount}
                        </p>
                      </div>
                      <span className="text-xs font-semibold text-[var(--cp-ink)]">
                        {usagePercent}%
                      </span>
                    </div>
                    <div className="mt-2 h-2 overflow-hidden rounded-full bg-white">
                      <div
                        className="h-full rounded-full bg-[var(--cp-primary)]"
                        style={{ width: `${Math.min(usagePercent, 100)}%` }}
                      />
                    </div>
                    <div className="mt-2 flex items-center justify-between text-[11px] text-[var(--cp-muted)]">
                      <span>
                        {disk.usedGb.toFixed(1)} / {disk.totalGb.toFixed(1)} GB
                      </span>
                      <span>{disk.fs ?? 'unknown'}</span>
                    </div>
                  </div>
                )
              })}
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
          {devices.map((device) => (
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
