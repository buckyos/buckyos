import { useEffect, useMemo, useState } from 'react'

import { fetchDashboard, mockDashboardData } from '@/api'
import Icon from '../icons'

type EventFilter = 'all' | EventItem['tone']

const toneStyles: Record<EventItem['tone'], string> = {
  success: 'bg-emerald-500',
  warning: 'bg-amber-500',
  info: 'bg-sky-500',
}

const tonePills: Record<EventItem['tone'], string> = {
  success: 'bg-emerald-100 text-emerald-700',
  warning: 'bg-amber-100 text-amber-700',
  info: 'bg-sky-100 text-sky-700',
}

const filterOptions: { value: EventFilter; label: string }[] = [
  { value: 'all', label: 'All' },
  { value: 'success', label: 'Success' },
  { value: 'warning', label: 'Warning' },
  { value: 'info', label: 'Info' },
]

const RecentEventsPage = () => {
  const [events, setEvents] = useState<EventItem[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<unknown>(null)
  const [filter, setFilter] = useState<EventFilter>('all')
  const [reloadKey, setReloadKey] = useState(0)

  useEffect(() => {
    let cancelled = false
    const loadEvents = async () => {
      setLoading(true)
      const { data, error } = await fetchDashboard()
      if (cancelled) {
        return
      }
      setEvents(data?.recentEvents ?? mockDashboardData.recentEvents)
      setError(error)
      if (error) {
        // eslint-disable-next-line no-console
        console.warn('Events API unavailable, using mock data', error)
      }
      setLoading(false)
    }

    loadEvents()
    return () => {
      cancelled = true
    }
  }, [reloadKey])

  const filteredEvents = useMemo(() => {
    if (filter === 'all') {
      return events
    }
    return events.filter((event) => event.tone === filter)
  }, [events, filter])

  const totals = useMemo(
    () => ({
      total: events.length,
      success: events.filter((event) => event.tone === 'success').length,
      warning: events.filter((event) => event.tone === 'warning').length,
      info: events.filter((event) => event.tone === 'info').length,
    }),
    [events],
  )

  const handleRefresh = () => setReloadKey((prev) => prev + 1)

  return (
    <div className="space-y-6">
      <header className="cp-panel px-8 py-6">
        <div className="flex flex-wrap items-center justify-between gap-4">
          <div>
            <div className="flex items-center gap-3 text-lg font-semibold text-[var(--cp-ink)]">
              <span className="inline-flex size-9 items-center justify-center rounded-2xl bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]">
                <Icon name="bell" className="size-4" />
              </span>
              <h1 className="text-2xl font-semibold text-[var(--cp-ink)] sm:text-3xl">
                Recent Events
              </h1>
            </div>
            <p className="mt-2 text-sm text-[var(--cp-muted)]">
              Review system alerts, operations, and automation activity in one place.
            </p>
          </div>
          <button
            type="button"
            onClick={handleRefresh}
            className="inline-flex items-center gap-2 rounded-full bg-[var(--cp-primary)] px-5 py-2 text-sm font-semibold text-white shadow transition hover:bg-[var(--cp-primary-strong)]"
          >
            Refresh
          </button>
        </div>
        <div className="mt-6 grid gap-3 sm:grid-cols-3">
          <div className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-3">
            <p className="text-xs uppercase tracking-wide text-[var(--cp-muted)]">Total Events</p>
            <p className="mt-2 text-2xl font-semibold text-[var(--cp-ink)]">{totals.total}</p>
          </div>
          <div className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-3">
            <p className="text-xs uppercase tracking-wide text-[var(--cp-muted)]">Warnings</p>
            <p className="mt-2 text-2xl font-semibold text-[var(--cp-ink)]">{totals.warning}</p>
          </div>
          <div className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-3">
            <p className="text-xs uppercase tracking-wide text-[var(--cp-muted)]">Successes</p>
            <p className="mt-2 text-2xl font-semibold text-[var(--cp-ink)]">{totals.success}</p>
          </div>
        </div>
      </header>

      <section className="cp-panel p-6">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="text-sm font-semibold text-[var(--cp-ink)]">Event Timeline</div>
          <div className="flex flex-wrap gap-2">
            {filterOptions.map((option) => (
              <button
                key={option.value}
                type="button"
                onClick={() => setFilter(option.value)}
                className={`cp-pill border border-[var(--cp-border)] text-[var(--cp-muted)] transition ${
                  filter === option.value
                    ? 'border-transparent bg-[var(--cp-primary)] text-white shadow'
                    : 'bg-[var(--cp-surface-muted)]'
                }`}
              >
                {option.label}
              </button>
            ))}
          </div>
        </div>

        {error ? (
          <div className="mt-4 rounded-xl border border-amber-200 bg-amber-50 px-4 py-3 text-xs text-amber-700">
            Unable to reach the events API. Showing cached data.
          </div>
        ) : null}

        <div className="mt-5 space-y-3">
          {loading ? (
            Array.from({ length: 4 }).map((_, index) => (
              <div
                // eslint-disable-next-line react/no-array-index-key
                key={`event-skeleton-${index}`}
                className="animate-pulse rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-4"
              >
                <div className="h-3 w-32 rounded-full bg-white" />
                <div className="mt-3 h-3 w-2/3 rounded-full bg-white" />
              </div>
            ))
          ) : filteredEvents.length ? (
            filteredEvents.map((event) => (
              <div
                key={`${event.title}-${event.subtitle}`}
                className="flex flex-wrap items-start justify-between gap-3 rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-4 text-sm text-[var(--cp-muted)]"
              >
                <div className="flex items-start gap-3">
                  <span
                    className={`mt-1 inline-flex size-2 rounded-full ${toneStyles[event.tone]}`}
                    aria-hidden
                  />
                  <div>
                    <p className="font-medium text-[var(--cp-ink)]">{event.title}</p>
                    <p className="text-xs text-[var(--cp-muted)]">{event.subtitle}</p>
                  </div>
                </div>
                <span className={`cp-pill uppercase tracking-wide ${tonePills[event.tone]}`}>
                  {event.tone}
                </span>
              </div>
            ))
          ) : (
            <div className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-6 text-center text-sm text-[var(--cp-muted)]">
              <p className="text-[var(--cp-ink)]">No recent events for this filter.</p>
              <p className="mt-1">Try switching the filter or refresh for new activity.</p>
            </div>
          )}
        </div>
      </section>
    </div>
  )
}

export default RecentEventsPage
