const poolCategories: PoolCategory[] = [
  { label: 'Applications', percent: 28, sizeGb: 450, color: '#2563eb' },
  { label: 'System Files', percent: 15, sizeGb: 240, color: '#f97316' },
  { label: 'User Data', percent: 24, sizeGb: 380, color: '#22c55e' },
  { label: 'Media Files', percent: 20, sizeGb: 320, color: '#facc15' },
  { label: 'Cache & Logs', percent: 11, sizeGb: 180, color: '#a855f7' },
  { label: 'Other', percent: 2, sizeGb: 30, color: '#94a3b8' },
]

const totalCapacityTb = 16
const usedCapacityTb = 7.85

const readWriteSpeed: ActivityPoint[] = [
  { time: '00:00', read: 38, write: 26 },
  { time: '00:05', read: 44, write: 30 },
  { time: '00:10', read: 48, write: 33 },
  { time: '00:15', read: 52, write: 35 },
  { time: '00:20', read: 55, write: 37 },
  { time: '00:25', read: 58, write: 41 },
]

const readWriteQps: ActivityPoint[] = [
  { time: '00:00', read: 1180, write: 820 },
  { time: '00:05', read: 1320, write: 910 },
  { time: '00:10', read: 1410, write: 970 },
  { time: '00:15', read: 1360, write: 930 },
  { time: '00:20', read: 1480, write: 980 },
  { time: '00:25', read: 1580, write: 1020 },
]

const storageNodes: DeviceNode[] = [
  {
    name: 'Primary Storage Node',
    role: 'Server',
    totalTb: 4,
    usedTb: 2.4,
    status: 'healthy',
    disks: [
      { label: 'NVMe SSD 1', sizeTb: 2, usagePercent: 65, status: 'healthy' },
      { label: 'NVMe SSD 2', sizeTb: 2, usagePercent: 55, status: 'healthy' },
    ],
  },
  {
    name: 'Secondary Node',
    role: 'Workstation',
    totalTb: 3,
    usedTb: 1.8,
    status: 'healthy',
    disks: [
      { label: 'SATA SSD', sizeTb: 1, usagePercent: 70, status: 'healthy' },
      { label: 'HDD Storage', sizeTb: 2, usagePercent: 55, status: 'warning' },
    ],
  },
  {
    name: 'Archive Node',
    role: 'Cold Storage',
    totalTb: 5,
    usedTb: 1.9,
    status: 'degraded',
    disks: [
      { label: 'HDD Array 1', sizeTb: 3, usagePercent: 40, status: 'healthy' },
      { label: 'HDD Array 2', sizeTb: 2, usagePercent: 36, status: 'healthy' },
    ],
  },
  {
    name: 'Analytics Node',
    role: 'Compute',
    totalTb: 4,
    usedTb: 3.2,
    status: 'healthy',
    disks: [
      { label: 'NVMe Scratch', sizeTb: 1, usagePercent: 82, status: 'warning' },
      { label: 'NVMe Cache', sizeTb: 1.5, usagePercent: 74, status: 'healthy' },
      { label: 'Bulk SSD', sizeTb: 1.5, usagePercent: 68, status: 'healthy' },
    ],
  },
]

const StoragePage = () => {
  const categoriesBar = poolCategories.map((category) => ({
    ...category,
    width: `${category.percent}%`,
  }))

  const speedPoints = readWriteSpeed.reduce<{ read: string; write: string }>(
    (acc, point, index) => {
      const x = (index / (readWriteSpeed.length - 1)) * 100
      return {
        read: `${acc.read}${acc.read ? ' ' : ''}${x},${100 - point.read}`,
        write: `${acc.write}${acc.write ? ' ' : ''}${x},${100 - point.write}`,
      }
    },
    { read: '', write: '' },
  )

  const qpsPoints = readWriteQps.reduce<{ read: string; write: string }>(
    (acc, point, index) => {
      const x = (index / (readWriteQps.length - 1)) * 100
      return {
        read: `${acc.read}${acc.read ? ' ' : ''}${x},${100 - point.read / 20}`,
        write: `${acc.write}${acc.write ? ' ' : ''}${x},${100 - point.write / 20}`,
      }
    },
    { read: '', write: '' },
  )

  return (
    <div className="space-y-6">
      <header className="rounded-3xl border border-slate-900/60 bg-slate-900/40 px-8 py-6 shadow-lg shadow-black/20 backdrop-blur">
        <div className="flex flex-wrap items-center justify-between gap-4">
          <div>
            <h1 className="text-2xl font-semibold text-white sm:text-3xl">Storage Management</h1>
            <p className="text-sm text-slate-400">
              Monitor and manage your distributed storage infrastructure
            </p>
          </div>
          <div className="flex items-center gap-3 text-sm text-slate-300">
            <span className="inline-flex size-2 rounded-full bg-emerald-400" aria-hidden />
            System Online
          </div>
        </div>
      </header>

      <section className="rounded-3xl border border-slate-900/60 bg-slate-900/60 p-6 shadow-lg shadow-black/20">
        <div className="mb-6 flex items-center gap-2 text-lg font-semibold text-white">
          <span aria-hidden>🧊</span>
          <h2>Storage Pool Overview</h2>
        </div>
        <div className="space-y-6 text-sm text-slate-300">
          <div className="flex items-center justify-between text-xs uppercase tracking-wide text-slate-500">
            <span>Total Pool Usage</span>
            <span className="text-white">
              {usedCapacityTb.toFixed(2)} TB / {totalCapacityTb.toFixed(1)} TB
            </span>
          </div>
          <div className="flex h-2 overflow-hidden rounded-full bg-slate-800">
            {categoriesBar.map((segment) => (
              <span
                key={segment.label}
                style={{ width: segment.width, backgroundColor: segment.color }}
                className="h-full"
              />
            ))}
          </div>
          <div className="grid gap-6 sm:grid-cols-2 lg:grid-cols-3">
            {poolCategories.map((category) => (
              <div
                key={category.label}
                className="flex items-center gap-3 rounded-2xl bg-slate-900/70 p-4 ring-1 ring-slate-800"
              >
                <span
                  className="inline-flex size-3 rounded-full"
                  style={{ backgroundColor: category.color }}
                />
                <div className="flex-1 text-xs text-slate-400">
                  <p className="font-semibold text-slate-200">{category.label}</p>
                  <p>
                    {category.percent}% • {category.sizeGb} GB
                  </p>
                </div>
              </div>
            ))}
          </div>
          <div className="rounded-2xl bg-slate-900/70 px-4 py-3 text-xs text-slate-400 ring-1 ring-slate-800">
            49% of total pool capacity used — maintain at least 25% free space to ensure optimal
            redundancy and burst allocation.
          </div>
        </div>
      </section>

      <section className="grid gap-6 lg:grid-cols-2">
        <div className="rounded-3xl border border-slate-900/60 bg-slate-900/60 p-6 shadow-lg shadow-black/20">
          <div className="mb-6 flex items-center gap-2 text-lg font-semibold text-white">
            <span aria-hidden>⚙️</span>
            <h2>Real-Time R/W Speed (MB/s)</h2>
          </div>
          <div className="rounded-2xl border border-slate-900/80 bg-slate-950/40 p-4">
            <svg viewBox="0 0 100 60" className="h-48 w-full text-slate-700">
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
                points={speedPoints.read}
                fill="none"
                stroke="#2563eb"
                strokeWidth="3"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
              <polyline
                points={speedPoints.write}
                fill="none"
                stroke="#22c55e"
                strokeWidth="3"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
            <div className="mt-3 flex justify-between text-xs text-slate-500">
              {readWriteSpeed.map((point) => (
                <span key={point.time}>{point.time}</span>
              ))}
            </div>
            <div className="mt-4 flex items-center justify-end gap-4 text-xs text-slate-400">
              <div className="flex items-center gap-2">
                <span className="inline-flex size-2 rounded-full bg-blue-500" />
                Read: 58 MB/s
              </div>
              <div className="flex items-center gap-2">
                <span className="inline-flex size-2 rounded-full bg-emerald-500" />
                Write: 41 MB/s
              </div>
            </div>
          </div>
        </div>

        <div className="rounded-3xl border border-slate-900/60 bg-slate-900/60 p-6 shadow-lg shadow-black/20">
          <div className="mb-6 flex items-center gap-2 text-lg font-semibold text-white">
            <span aria-hidden>📈</span>
            <h2>Real-Time R/W QPS</h2>
          </div>
          <div className="rounded-2xl border border-slate-900/80 bg-slate-950/40 p-4">
            <svg viewBox="0 0 100 60" className="h-48 w-full text-slate-700">
              <rect x="0" y="0" width="100" height="60" fill="transparent" />
              {[400, 800, 1200, 1600].map((value) => (
                <line
                  // eslint-disable-next-line react/no-array-index-key
                  key={value}
                  x1="0"
                  y1={60 - value / 20}
                  x2="100"
                  y2={60 - value / 20}
                  stroke="currentColor"
                  strokeWidth="0.4"
                  strokeDasharray="2"
                />
              ))}
              <polyline
                points={qpsPoints.read}
                fill="none"
                stroke="#2563eb"
                strokeWidth="3"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
              <polyline
                points={qpsPoints.write}
                fill="none"
                stroke="#22c55e"
                strokeWidth="3"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
            <div className="mt-3 flex justify-between text-xs text-slate-500">
              {readWriteQps.map((point) => (
                <span key={point.time}>{point.time}</span>
              ))}
            </div>
            <div className="mt-4 flex items-center justify-end gap-4 text-xs text-slate-400">
              <div className="flex items-center gap-2">
                <span className="inline-flex size-2 rounded-full bg-blue-500" />
                Read: 1,420 QPS
              </div>
              <div className="flex items-center gap-2">
                <span className="inline-flex size-2 rounded-full bg-emerald-500" />
                Write: 1,020 QPS
              </div>
            </div>
          </div>
        </div>
      </section>

      <section className="rounded-3xl border border-slate-900/60 bg-slate-900/60 p-6 shadow-lg shadow-black/20">
        <div className="mb-6 flex items-center gap-2 text-lg font-semibold text-white">
          <span aria-hidden>🖥️</span>
          <h2>Storage Pool Devices</h2>
        </div>
        <div className="space-y-6">
          {storageNodes.map((node) => {
            const usagePercent = Math.round((node.usedTb / node.totalTb) * 100)
            const statusStyles: Record<DeviceNode['status'], string> = {
              healthy: 'bg-emerald-500/15 text-emerald-300',
              degraded: 'bg-amber-500/15 text-amber-300',
              offline: 'bg-rose-500/15 text-rose-300',
            }
            const diskStatusStyles: Record<DeviceDisk['status'], string> = {
              healthy: 'text-emerald-400',
              warning: 'text-amber-300',
              offline: 'text-rose-300',
            }
            return (
              <div
                key={node.name}
                className="rounded-3xl bg-slate-900/70 p-6 text-sm text-slate-300 ring-1 ring-slate-800"
              >
                <div className="flex flex-wrap items-start justify-between gap-4">
                  <div>
                    <div className="flex items-center gap-2 text-white">
                      <span className="text-lg" aria-hidden>
                        🗄️
                      </span>
                      <p className="text-lg font-semibold">{node.name}</p>
                    </div>
                    <p className="text-xs uppercase tracking-wide text-slate-500">{node.role}</p>
                  </div>
                  <span className={`inline-flex items-center gap-2 rounded-full px-3 py-1 text-xs font-semibold ${statusStyles[node.status]}`}>
                    <span aria-hidden>✔</span>
                    {node.status}
                  </span>
                </div>

                <div className="mt-6 space-y-4 text-xs text-slate-400">
                  <div className="flex items-center justify-between text-slate-300">
                    <span>Storage Contribution</span>
                    <span>
                      {node.usedTb.toFixed(1)} TB / {node.totalTb.toFixed(1)} TB
                    </span>
                  </div>
                  <div className="h-2 overflow-hidden rounded-full bg-slate-800">
                    <div
                      className="h-full rounded-full bg-blue-500"
                      style={{ width: `${usagePercent}%` }}
                    />
                  </div>

                  <div className="rounded-2xl border border-slate-800 bg-slate-950/40 p-4">
                    <div className="mb-3 flex items-center gap-2 text-slate-300">
                      <span aria-hidden>💽</span>
                      <span className="font-medium">Physical Disks</span>
                    </div>
                    <div className="space-y-3">
                      {node.disks.map((disk) => (
                        <div key={disk.label} className="rounded-2xl bg-slate-900/60 p-4">
                          <div className="flex items-center justify-between text-sm text-white">
                            <span>{disk.label}</span>
                            <span>{disk.usagePercent}%</span>
                          </div>
                          <div className="mt-1 flex items-center justify-between text-xs text-slate-400">
                            <span>{disk.sizeTb} TB</span>
                            <span className={diskStatusStyles[disk.status]}>
                              {disk.status === 'healthy'
                                ? '● healthy'
                                : disk.status === 'warning'
                                  ? '● monitor'
                                  : '● offline'}
                            </span>
                          </div>
                          <div className="mt-2 h-2 overflow-hidden rounded-full bg-slate-800">
                            <div
                              className="h-full rounded-full bg-emerald-500"
                              style={{ width: `${disk.usagePercent}%` }}
                            />
                          </div>
                        </div>
                      ))}
                    </div>
                  </div>
                </div>

                <div className="mt-6 flex flex-wrap gap-3 text-xs">
                  <button
                    type="button"
                    className="flex items-center gap-2 rounded-xl bg-slate-900/80 px-4 py-2 font-medium text-slate-300 ring-1 ring-slate-800 transition hover:bg-slate-800 hover:text-white"
                  >
                    ⚡ Performance
                  </button>
                  <button
                    type="button"
                    className="flex items-center gap-2 rounded-xl bg-slate-900/80 px-4 py-2 font-medium text-slate-300 ring-1 ring-slate-800 transition hover:bg-slate-800 hover:text-white"
                  >
                    🩺 Health Check
                  </button>
                </div>
              </div>
            )
          })}
        </div>
      </section>
    </div>
  )
}

export default StoragePage
