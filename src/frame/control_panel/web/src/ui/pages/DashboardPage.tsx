import type { CSSProperties } from 'react'
import { useEffect, useState } from 'react'
import { Link } from 'react-router-dom'

import { fetchDashboard, mockDashboardData } from '@/api'

const toneStyles: Record<EventItem['tone'], string> = {
  success: 'bg-emerald-500/20 text-emerald-300',
  warning: 'bg-amber-500/20 text-amber-300',
  info: 'bg-sky-500/20 text-sky-300',
}

const DashboardPage = () => {
  const [dashboardData, setDashboardData] = useState<DashboardState | null>(null)
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    let cancelled = false
    const loadDashboard = async () => {
      const { data, error } = await fetchDashboard()
      if (!cancelled) {
        setDashboardData(data ?? mockDashboardData)
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

  const recentEvents = dashboardData?.recentEvents ?? []
  const dapps = dashboardData?.dapps ?? []
  const quickActions = dashboardData?.quickActions ?? []
  const resourceTimeline = dashboardData?.resourceTimeline ?? []
  const storageSlices = dashboardData?.storageSlices ?? []
  const storageCapacityGb = dashboardData?.storageCapacityGb ?? 0
  const storageUsedGb = dashboardData?.storageUsedGb ?? 0
  const devices = dashboardData?.devices ?? []
  const memoryInfo = dashboardData?.memory
  const cpuInfo = dashboardData?.cpu
  const disks = dashboardData?.disks ?? []

  const totalMemoryGb = memoryInfo?.totalGb ?? 0
  const usedMemoryGb = memoryInfo?.usedGb ?? 0
  const memoryPercent = Math.round(memoryInfo?.usagePercent ?? 0)
  const cpuPercent = Math.round(cpuInfo?.usagePercent ?? 0)
  const cpuModel = cpuInfo?.model ?? 'Unknown CPU'
  const cpuCores = cpuInfo?.cores ?? 0

  const cpuUsage = resourceTimeline.at(-1)?.cpu ?? 0
  const memoryUsage = resourceTimeline.at(-1)?.memory ?? 0
  const resourceLinePoints = resourceTimeline
    .map((point, index) => {
      const x = (index / (resourceTimeline.length - 1)) * 100
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
      <div className="flex min-h-[60vh] items-center justify-center rounded-3xl border border-slate-900/60 bg-slate-900/40 px-8 py-12 shadow-lg shadow-black/20 backdrop-blur">
        <div className="flex items-center gap-3 text-slate-300">
          <span className="size-3 animate-pulse rounded-full bg-sky-400" aria-hidden />
          <span className="text-sm">Loading dashboard...</span>
        </div>
      </div>
    )
  }

  return (
    <div className="space-y-6">
      <header className="rounded-3xl border border-slate-900/60 bg-slate-900/40 px-8 py-6 shadow-lg shadow-black/20 backdrop-blur">
        <div className="flex flex-wrap items-center justify-between gap-4">
          <div>
            <h1 className="text-2xl font-semibold text-white sm:text-3xl">System Overview</h1>
            <p className="text-sm text-slate-400">
              Monitor your decentralized personal cloud infrastructure
            </p>
          </div>
          <div className="flex items-center gap-3 text-sm text-slate-300">
            <span className="inline-flex size-2 rounded-full bg-emerald-400" aria-hidden />
            System Online
          </div>
        </div>
      </header>

      <section className="grid gap-6 lg:grid-cols-[2fr_1fr]">
        <div className="rounded-3xl border border-slate-900/60 bg-slate-900/60 p-6 shadow-lg shadow-black/20">
          <div className="mb-6 flex items-center gap-2 text-lg font-semibold text-white">
            <span aria-hidden>⚡</span>
            <h2>System Resources</h2>
          </div>
          <div className="grid gap-4 lg:grid-cols-[1fr_auto]">
            <div className="space-y-4">
              <div className="flex flex-wrap gap-3 text-sm text-slate-300">
                <div className="flex min-w-[180px] flex-1 items-center gap-3 rounded-xl bg-slate-900/70 px-4 py-2 ring-1 ring-slate-800">
                  <span className="inline-flex size-2 rounded-full bg-sky-500" aria-hidden />
                  <span className="flex-1 font-medium text-white">CPU</span>
                  <span className="text-xs uppercase tracking-wide text-slate-400">
                    {cpuPercent || cpuUsage}%
                  </span>
                </div>
                <div className="flex min-w-[180px] flex-1 items-center gap-3 rounded-xl bg-slate-900/70 px-4 py-2 ring-1 ring-slate-800">
                  <span className="inline-flex size-2 rounded-full bg-emerald-500" aria-hidden />
                  <span className="flex-1 font-medium text-white">Memory</span>
                  <span className="text-xs uppercase tracking-wide text-slate-400">
                    {memoryPercent || memoryUsage}%
                  </span>
                </div>
              </div>
              <div className="rounded-2xl border border-slate-900/80 bg-slate-950/40 p-4">
                <svg viewBox="0 0 100 60" className="h-40 w-full text-slate-700">
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
                    stroke="#38bdf8"
                    strokeWidth="2"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  />
                  <polyline
                    points={resourceLinePoints.memory}
                    fill="none"
                    stroke="#22c55e"
                    strokeWidth="2"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  />
                </svg>
                <div className="mt-3 flex justify-between text-xs text-slate-500">
                  {resourceTimeline.map((point) => (
                    <span key={point.time}>{point.time}</span>
                  ))}
                </div>
              </div>
            </div>
            <div className="flex flex-col justify-between gap-6 text-sm text-slate-300">
              <div className="rounded-2xl bg-slate-900/70 px-4 py-5 text-center ring-1 ring-slate-800">
                <p className="text-xs uppercase tracking-wide text-slate-400">CPU Usage</p>
                <p className="mt-2 text-3xl font-semibold text-white">{cpuPercent || cpuUsage}%</p>
                <p className="text-xs text-emerald-400">Live</p>
                <p className="mt-1 truncate text-[11px] text-slate-500">
                  {cpuModel}
                  {cpuCores ? ` · ${cpuCores} cores` : ''}
                </p>
              </div>
              <div className="rounded-2xl bg-slate-900/70 px-4 py-5 text-center ring-1 ring-slate-800">
                <p className="text-xs uppercase tracking-wide text-slate-400">Memory Usage</p>
                <p className="mt-2 text-3xl font-semibold text-white">
                  {memoryPercent || memoryUsage}%
                </p>
                <p className="text-xs text-sky-400">Live</p>
                <p className="mt-1 text-[11px] text-slate-500">
                  {usedMemoryGb.toFixed(1)} / {totalMemoryGb.toFixed(1)} GB
                </p>
              </div>
            </div>
          </div>
        </div>

        <div className="rounded-3xl border border-slate-900/60 bg-slate-900/60 p-6 shadow-lg shadow-black/20">
          <div className="mb-6 flex items-center gap-2 text-lg font-semibold text-white">
            <span aria-hidden>🧠</span>
            <h2>Storage Overview</h2>
          </div>
          <div className="space-y-6 text-sm text-slate-300">
            <div className="flex items-center justify-between text-xs uppercase tracking-wide text-slate-500">
              <span>Used Space</span>
              <span className="text-white">
                {(storageUsedGb / 1024).toFixed(1)}TB / {(storageCapacityGb / 1024).toFixed(1)}TB
              </span>
            </div>
            <div className="flex h-2 overflow-hidden rounded-full bg-slate-800">
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
                  <div className="size-10 rounded-full bg-slate-900" />
                </div>
                <div className="space-y-2 text-xs text-slate-400">
                  {storageSlices.map((slice) => (
                    <div key={slice.label} className="flex items-center gap-2">
                      <span
                        className="inline-flex size-2 rounded-full"
                        style={{ backgroundColor: slice.color }}
                      />
                      <span className="w-20 text-slate-300">{slice.label}</span>
                      <span>{slice.value}%</span>
                    </div>
                  ))}
                </div>
              </div>
              <div className="rounded-2xl bg-slate-900/70 px-5 py-4 text-xs text-slate-400 ring-1 ring-slate-800">
                <div className="flex justify-between text-slate-300">
                  <span>Snapshots</span>
                  <span>12</span>
                </div>
                <div className="flex justify-between text-slate-300">
                  <span>Replication</span>
                  <span>Enabled</span>
                </div>
                <div className="mt-2 rounded-lg bg-slate-950/50 px-3 py-2 text-sky-300">
                  4 nodes synced
                </div>
              </div>
            </div>
          </div>
        </div>
      </section>

      <section className="grid gap-6 lg:grid-cols-[2fr_1fr]">
        <div className="rounded-3xl border border-slate-900/60 bg-slate-900/60 p-6 shadow-lg shadow-black/20">
          <div className="flex items-center justify-between">
            <h2 className="text-lg font-semibold text-white">Recent Events</h2>
            <span className="text-xs uppercase tracking-wide text-slate-500">Last 24h</span>
          </div>
          <div className="mt-5 max-h-64 space-y-3 overflow-y-auto pr-1">
            {recentEvents.map((item) => (
              <div
                key={item.title}
                className="flex items-start gap-3 rounded-2xl bg-slate-900/70 px-4 py-3 text-sm text-slate-300 ring-1 ring-slate-800"
              >
                <span
                  className={`mt-0.5 inline-flex size-6 items-center justify-center rounded-full text-xs font-semibold ${toneStyles[item.tone]}`}
                >
                  ●
                </span>
                <div className="flex-1">
                  <p className="font-medium text-white">{item.title}</p>
                  <p className="text-xs text-slate-500">{item.subtitle}</p>
                </div>
              </div>
            ))}
          </div>
        </div>

        <div className="rounded-3xl border border-slate-900/60 bg-slate-900/60 p-6 shadow-lg shadow-black/20">
          <div className="mb-4 flex items-center gap-2 text-lg font-semibold text-white">
            <span aria-hidden>🧩</span>
            <h2>dApps</h2>
          </div>
          <div className="space-y-3">
            {dapps.map((dapp) => (
              <div
                key={dapp.name}
                className="flex items-center justify-between rounded-2xl bg-slate-900/70 px-4 py-3 text-sm ring-1 ring-slate-800"
              >
                <div className="flex items-center gap-3 text-slate-200">
                  <span className="text-lg" aria-hidden>
                    {dapp.icon}
                  </span>
                  <span>{dapp.name}</span>
                </div>
                <span
                  className={[
                    'rounded-full px-3 py-1 text-xs font-semibold capitalize',
                    dapp.status === 'running'
                      ? 'bg-emerald-500/15 text-emerald-300'
                      : 'bg-slate-700/60 text-slate-400',
                  ].join(' ')}
                >
                  {dapp.status}
                </span>
              </div>
            ))}
          </div>
        </div>
      </section>

      <section className="grid gap-6 lg:grid-cols-[1.5fr_1fr_1fr]">
        <div className="rounded-3xl border border-slate-900/60 bg-slate-900/60 p-6 shadow-lg shadow-black/20">
          <div className="mb-6 flex items-center gap-2 text-lg font-semibold text-white">
            <span aria-hidden>🌐</span>
            <h2>Network Status</h2>
          </div>
          <div className="space-y-6 text-sm text-slate-300">
            <div>
              <div className="mb-1 flex justify-between text-xs uppercase tracking-wide text-slate-500">
                <span>Upload</span>
                <span className="text-white">12.5 MB/s</span>
              </div>
              <div className="h-2 rounded-full bg-slate-800">
                <div className="h-full w-3/5 rounded-full bg-sky-500" />
              </div>
            </div>
            <div>
              <div className="mb-1 flex justify-between text-xs uppercase tracking-wide text-slate-500">
                <span>Download</span>
                <span className="text-white">24.8 MB/s</span>
              </div>
              <div className="h-2 rounded-full bg-slate-800">
                <div className="h-full w-4/5 rounded-full bg-blue-500" />
              </div>
            </div>
            <div className="grid grid-cols-2 gap-4 text-xs text-slate-400">
              <div>
                <p className="uppercase tracking-wide">Connected Peers</p>
                <p className="mt-1 text-lg font-semibold text-white">847</p>
              </div>
              <div>
                <p className="uppercase tracking-wide">Active Connections</p>
                <p className="mt-1 text-lg font-semibold text-white">23</p>
              </div>
            </div>
          </div>
        </div>

        <div className="rounded-3xl border border-slate-900/60 bg-slate-900/60 p-6 shadow-lg shadow-black/20">
          <div className="mb-6 flex items-center gap-2 text-lg font-semibold text-white">
            <span aria-hidden>🧠</span>
            <h2>Storage</h2>
          </div>
          <div className="space-y-4 text-sm text-slate-300">
            <div className="flex items-center justify-between text-xs uppercase tracking-wide text-slate-500">
              <span>Capacity</span>
              <span className="text-white">{storageCapacityGb.toFixed(0)} GB</span>
            </div>
            <div className="flex items-center justify-between text-xs uppercase tracking-wide text-slate-500">
              <span>Used</span>
              <span className="text-white">{storageUsedGb.toFixed(0)} GB</span>
            </div>
            <div className="flex h-2 overflow-hidden rounded-full bg-slate-800">
              <span
                className="block h-full rounded-full bg-sky-500"
                style={{
                  width: `${storageCapacityGb ? Math.min((storageUsedGb / storageCapacityGb) * 100, 100) : 0}%`,
                }}
              />
            </div>
            <div className="space-y-2 text-xs text-slate-400">
              {disks.map((disk: any) => (
                <div
                  key={`${disk.mount}-${disk.label}`}
                  className="flex items-center justify-between rounded-xl bg-slate-900/70 px-3 py-2 ring-1 ring-slate-800"
                >
                  <div className="text-xs text-slate-400">
                    <p className="text-white">{disk.label}</p>
                    <p>{disk.mount}</p>
                  </div>
                  <div className="text-xs text-slate-300">
                    <span className="text-white">{disk.usedGb.toFixed(1)} GB</span>
                    <span className="text-slate-500"> / {disk.totalGb.toFixed(1)} GB</span>
                  </div>
                </div>
              ))}
            </div>
          </div>
        </div>

        <div className="rounded-3xl border border-slate-900/60 bg-slate-900/60 p-6 shadow-lg shadow-black/20">
          <div className="mb-6 flex items-center gap-2 text-lg font-semibold text-white">
            <span aria-hidden>⚡</span>
            <h2>Quick Actions</h2>
          </div>
          <div className="space-y-3 text-sm text-slate-200">
            {quickActions.map((action) => (
              <Link
                key={action.to}
                to={action.to}
                className="flex w-full items-center gap-3 rounded-2xl bg-slate-900/70 px-4 py-3 text-left transition hover:bg-slate-800"
              >
                <span className="text-lg" aria-hidden>
                  {action.icon}
                </span>
                <span>{action.label}</span>
              </Link>
            ))}
          </div>
        </div>
      </section>

      <section className="rounded-3xl border border-slate-900/60 bg-slate-900/60 p-6 shadow-lg shadow-black/20">
          <div className="mb-2 flex items-center gap-2 text-lg font-semibold text-white">
            <span aria-hidden>🖥️</span>
            <h2>Hardware</h2>
          </div>
          <p className="mb-4 text-xs text-slate-400">
            CPU: {cpuModel}
            {cpuCores ? ` · ${cpuCores} cores` : ''}
          </p>
          <p className="mb-4 text-xs text-slate-400">
            Memory: {usedMemoryGb.toFixed(1)} / {totalMemoryGb.toFixed(1)} GB
          </p>
          <div className="grid gap-3 text-sm text-slate-300 md:grid-cols-2">
            {(devices as any[]).map((device) => (
              <div
              key={device.name}
              className="flex flex-col gap-2 rounded-2xl bg-slate-900/70 p-4 ring-1 ring-slate-800"
            >
              <div className="flex items-center justify-between">
                <div>
                  <p className="text-white">{device.name}</p>
                  <p className="text-xs text-slate-500 capitalize">{device.role}</p>
                </div>
                <span
                  className={[
                    'rounded-full px-3 py-1 text-[11px] uppercase tracking-wide',
                    device.status === 'online'
                      ? 'bg-emerald-500/15 text-emerald-300'
                      : 'bg-amber-500/15 text-amber-300',
                  ].join(' ')}
                >
                  {device.status}
                </span>
              </div>
              <div className="flex items-center gap-2 text-xs text-slate-300">
                <span className="rounded-full bg-slate-800 px-3 py-1">
                  CPU {device.cpu ?? cpuPercent ?? cpuUsage}%
                </span>
                <span className="rounded-full bg-slate-800 px-3 py-1">
                  Mem {device.memory ?? memoryPercent ?? memoryUsage}%
                </span>
              </div>
              <div className="text-[11px] uppercase tracking-wide text-slate-500">
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
