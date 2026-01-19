import { useEffect, useState } from 'react'
import { NavLink, Outlet } from 'react-router-dom'

import { fetchLayout, mockLayoutData } from '@/api'
import Icon from './icons'

const baseNavClasses =
  'cp-nav-link text-[var(--cp-muted)] hover:bg-[var(--cp-surface-muted)] hover:text-[var(--cp-primary-strong)]'

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
      <div className="flex min-h-screen items-center justify-center bg-transparent text-[var(--cp-ink)]">
        <div className="cp-panel px-6 py-4 text-sm text-[var(--cp-muted)]">
          Loading layout...
        </div>
      </div>
    )
  }

  if (!data) {
    return (
      <div className="flex min-h-screen items-center justify-center bg-transparent text-[var(--cp-ink)]">
        <div className="cp-panel flex items-center gap-3 px-6 py-4 text-sm text-[var(--cp-danger)]">
          <Icon name="alert" className="size-4" />
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
    <div className="relative min-h-screen text-[var(--cp-ink)]">
      {error ? (
        <div className="pointer-events-none absolute inset-x-0 top-4 flex justify-center px-4">
          <div
            role="alert"
            className="pointer-events-auto flex max-w-xl items-start gap-3 rounded-xl border border-amber-200 bg-amber-50 px-4 py-3 text-sm text-amber-800 shadow-lg shadow-amber-100/60"
          >
            <Icon name="alert" className="mt-0.5 size-4" />
            <div className="flex-1">
              <p className="font-medium text-amber-900">Layout request failed</p>
              <p className="text-xs text-amber-700">Using mock data. {errorMessage}</p>
            </div>
          </div>
        </div>
      ) : null}
      <div className="cp-shell grid min-h-screen gap-8 lg:grid-cols-[260px_1fr]">
        <aside className="sticky top-10 flex h-fit flex-col rounded-3xl border border-[var(--cp-border)] bg-white/85 p-6 shadow-xl shadow-slate-200/60 backdrop-blur">
          <div className="mb-8 flex items-center gap-3 text-lg font-semibold tracking-tight text-[var(--cp-ink)]">
            <span className="inline-flex size-10 items-center justify-center rounded-2xl bg-[var(--cp-primary)] text-lg text-white shadow-lg shadow-emerald-200">
              B
            </span>
            <div className="flex flex-col leading-tight">
              <span className="font-semibold">BuckyOS</span>
              <span className="text-xs font-medium text-[var(--cp-muted)]">Control Panel</span>
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
                        ? 'bg-[var(--cp-primary)] text-white shadow-lg shadow-emerald-200'
                        : '',
                    ].join(' ')
                  }
                >
                  <Icon name={item.icon} className="size-4" />
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
                        ? 'bg-[var(--cp-surface-muted)] text-[var(--cp-ink)]'
                        : '',
                    ].join(' ')
                  }
                >
                  <Icon name={item.icon} className="size-4" />
                  <span className="flex-1">{item.label}</span>
                  {item.badge ? (
                    <span className="inline-flex min-w-6 items-center justify-center rounded-full bg-[var(--cp-danger)] px-1.5 text-xs font-semibold text-white">
                      {item.badge}
                    </span>
                  ) : null}
                </NavLink>
              ))}
            </div>
          </nav>

          <div className="mt-8 space-y-4 rounded-2xl bg-[var(--cp-surface-muted)] p-4 text-sm text-[var(--cp-muted)]">
            <div className="flex items-center gap-3">
              <img
                src={profile.avatar}
                alt={`${profile.name} avatar`}
                className="size-10 rounded-full border border-[var(--cp-border)] object-cover"
              />
              <div className="leading-tight">
                <p className="font-medium text-[var(--cp-ink)]">{profile.name}</p>
                <p className="text-xs text-[var(--cp-muted)]">{profile.email}</p>
              </div>
            </div>
            <div className="rounded-xl border border-[var(--cp-border)] bg-white px-4 py-3 text-xs leading-5 text-[var(--cp-muted)]">
              <div className="mb-2 flex items-center gap-2 text-[var(--cp-ink)]">
                <span className="size-2 rounded-full bg-[var(--cp-success)]" aria-hidden />
                {systemStatus.label}
              </div>
              <div className="flex justify-between">
                <span>Network</span>
                <span className="text-[var(--cp-ink)]">{systemStatus.networkPeers} peers</span>
              </div>
              <div className="flex justify-between">
                <span>Active Sessions</span>
                <span className="text-[var(--cp-ink)]">{systemStatus.activeSessions}</span>
              </div>
            </div>
          </div>
        </aside>

        <main className="min-w-0 pb-10">
          <Outlet />
        </main>
      </div>
    </div>
  )
}

export default RootLayout
