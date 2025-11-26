import { useEffect, useState } from 'react'
import { NavLink, Outlet } from 'react-router-dom'

import { fetchLayout, mockLayoutData } from '@/api'

const baseNavClasses =
  'flex w-full items-center gap-3 rounded-xl px-3.5 py-2.5 text-left transition'

const RootLayout = () => {
  const [data, setData] = useState<RootLayoutData | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<unknown>(null)

  useEffect(() => {
    const load = async () => {
      const { data, error } = await fetchLayout()
      setData(data ?? mockLayoutData)
      setError(error)
      setLoading(false)
    }

    load()
  }, [])

  if (loading) {
    return (
      <div className="flex min-h-screen items-center justify-center bg-slate-950 text-slate-100">
        <div className="rounded-xl bg-slate-900/80 px-6 py-4 text-sm text-slate-300 shadow-lg shadow-slate-900/40">
          Loading layout...
        </div>
      </div>
    )
  }

  if (!data) {
    return (
      <div className="flex min-h-screen items-center justify-center bg-slate-950 text-slate-100">
        <div className="rounded-xl bg-rose-500/20 px-6 py-4 text-sm text-rose-100 shadow-lg shadow-rose-900/40">
          Failed to load layout data.
        </div>
      </div>
    )
  }

  const { primaryNav, secondaryNav, profile, systemStatus } = data
  const errorMessage =
    error instanceof Error
      ? error.message
      : typeof error === 'string'
        ? error
        : 'Failed to load layout data'

  return (
    <div className="relative min-h-screen bg-slate-950 text-slate-100">
      {error ? (
        <div className="pointer-events-none absolute inset-x-0 top-4 flex justify-center px-4">
          <div
            role="alert"
            className="pointer-events-auto flex max-w-xl items-start gap-3 rounded-xl border border-amber-500/40 bg-amber-500/15 px-4 py-3 text-sm text-amber-100 shadow-lg shadow-amber-900/30 backdrop-blur"
          >
            <span aria-hidden>⚠️</span>
            <div className="flex-1">
              <p className="font-medium text-amber-50">Layout request failed</p>
              <p className="text-xs text-amber-100/80">Using mock data. {errorMessage}</p>
            </div>
          </div>
        </div>
      ) : null}
      <div className="mx-auto flex min-h-screen w-full gap-8 px-6 py-10 lg:px-10">
        <aside className="fixed left-6 top-10 z-20 flex h-[calc(100vh-5rem)] w-64 flex-col rounded-3xl bg-slate-900/60 p-6 backdrop-blur">
          <div className="mb-10 flex items-center gap-3 text-lg font-semibold tracking-tight">
            <span className="inline-flex size-10 items-center justify-center rounded-2xl bg-sky-500 text-2xl">
              B
            </span>
            <div className="flex flex-col leading-tight">
              <span>BuckyOS</span>
              <span className="text-xs font-normal text-slate-400">Control Panel</span>
            </div>
          </div>

          <nav className="space-y-8 text-sm">
            <div className="space-y-1.5">
              {primaryNav.map((item) => (
                <NavLink
                  key={item.path}
                  to={item.path}
                  end={item.path === '/'}
                  className={({ isActive }) =>
                    [
                      baseNavClasses,
                      isActive
                        ? 'bg-sky-500 text-white shadow-lg shadow-sky-500/30'
                        : 'text-slate-300 hover:bg-slate-800/80',
                    ].join(' ')
                  }
                >
                  <span aria-hidden>{item.icon}</span>
                  <span className="flex-1">{item.label}</span>
                </NavLink>
              ))}
            </div>

            <div className="space-y-1.5">
              {secondaryNav.map((item) => (
                <NavLink
                  key={item.path}
                  to={item.path}
                  className={({ isActive }) =>
                    [
                      baseNavClasses,
                      isActive
                        ? 'bg-slate-800/90 text-white'
                        : 'text-slate-300 hover:bg-slate-800/80',
                    ].join(' ')
                  }
                >
                  <span aria-hidden>{item.icon}</span>
                  <span className="flex-1">{item.label}</span>
                  {item.badge ? (
                    <span className="inline-flex min-w-6 items-center justify-center rounded-full bg-rose-500 px-1.5 text-xs font-semibold text-white">
                      {item.badge}
                    </span>
                  ) : null}
                </NavLink>
              ))}
            </div>
          </nav>

          <div className="mt-auto space-y-4 rounded-2xl bg-slate-900/80 p-4 text-sm text-slate-300">
            <div className="flex items-center gap-3">
              <img
                src={profile.avatar}
                alt={`${profile.name} avatar`}
                className="size-10 rounded-full border border-slate-700 object-cover"
              />
              <div className="leading-tight">
                <p className="font-medium text-white">{profile.name}</p>
                <p className="text-xs text-slate-400">{profile.email}</p>
              </div>
            </div>
            <div className="rounded-xl border border-slate-800 bg-slate-900/60 px-4 py-3 text-xs leading-5 text-slate-400">
              <div className="mb-1 flex items-center gap-2 text-slate-200">
                <span className="size-2 rounded-full bg-emerald-400" aria-hidden />
                {systemStatus.label}
              </div>
              <div className="flex justify-between">
                <span>Network</span>
                <span className="text-white">{systemStatus.networkPeers} peers</span>
              </div>
              <div className="flex justify-between">
                <span>Active Sessions</span>
                <span className="text-white">{systemStatus.activeSessions}</span>
              </div>
            </div>
          </div>
        </aside>

        <main className="ml-[18rem] flex-1 pb-10 pr-2">
          <Outlet />
        </main>
      </div>
    </div>
  )
}

export default RootLayout
