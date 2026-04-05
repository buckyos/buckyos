/* ── TaskCenter System Events Page ── */

import { useState, useMemo } from 'react'
import {
  Search,
  Filter,
  Play,
  CheckCircle2,
  XCircle,
  Ban,
  Milestone,
  Bell,
  BellOff,
  ChevronRight,
} from 'lucide-react'
import { useI18n } from '../../../i18n/provider'
import { useTaskCenterStore } from '../hooks/use-task-center-store'
import type { SystemEvent, SystemEventType } from '../mock/types'
import type { TaskCenterNav } from '../components/layout/navigation'

function eventIcon(eventType: SystemEventType) {
  switch (eventType) {
    case 'task_created':
      return <Play size={14} />
    case 'task_completed':
      return <CheckCircle2 size={14} />
    case 'task_failed':
      return <XCircle size={14} />
    case 'task_cancelled':
      return <Ban size={14} />
    case 'task_milestone':
      return <Milestone size={14} />
    case 'notification_created':
      return <Bell size={14} />
    case 'notification_handled':
      return <BellOff size={14} />
    default:
      return <Play size={14} />
  }
}

function eventColor(eventType: SystemEventType) {
  switch (eventType) {
    case 'task_created':
      return 'var(--cp-accent)'
    case 'task_completed':
      return 'var(--cp-success)'
    case 'task_failed':
      return 'var(--cp-danger)'
    case 'task_cancelled':
      return 'var(--cp-muted)'
    case 'task_milestone':
      return 'var(--cp-warning)'
    case 'notification_created':
      return 'var(--cp-accent)'
    case 'notification_handled':
      return 'var(--cp-success)'
    default:
      return 'var(--cp-muted)'
  }
}

function eventTypeLabel(eventType: SystemEventType) {
  switch (eventType) {
    case 'task_created':
      return 'Created'
    case 'task_completed':
      return 'Completed'
    case 'task_failed':
      return 'Failed'
    case 'task_cancelled':
      return 'Cancelled'
    case 'task_milestone':
      return 'Milestone'
    case 'notification_created':
      return 'Notification'
    case 'notification_handled':
      return 'Handled'
    default:
      return eventType
  }
}

function formatTime(iso: string) {
  const d = new Date(iso)
  return d.toLocaleString(undefined, {
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  })
}

function formatDate(iso: string) {
  const d = new Date(iso)
  return d.toLocaleDateString(undefined, { year: 'numeric', month: 'long', day: 'numeric' })
}

const eventTypeOptions: (SystemEventType | '')[] = [
  '',
  'task_created',
  'task_completed',
  'task_failed',
  'task_cancelled',
  'task_milestone',
  'notification_created',
  'notification_handled',
]

interface SystemEventsPageProps {
  onNavigate: (nav: TaskCenterNav) => void
}

export function SystemEventsPage({ onNavigate }: SystemEventsPageProps) {
  const store = useTaskCenterStore()
  const { t } = useI18n()
  const [search, setSearch] = useState('')
  const [filterType, setFilterType] = useState<SystemEventType | ''>('')
  const [showFilters, setShowFilters] = useState(false)

  const events = store.getEvents()

  const filtered = useMemo(() => {
    return events.filter((evt) => {
      if (filterType && evt.eventType !== filterType) return false
      if (search) {
        const q = search.toLowerCase()
        if (
          !evt.title.toLowerCase().includes(q) &&
          !evt.summary.toLowerCase().includes(q) &&
          !evt.source.toLowerCase().includes(q)
        )
          return false
      }
      return true
    })
  }, [events, filterType, search])

  // Group by date
  const grouped = useMemo(() => {
    const map = new Map<string, SystemEvent[]>()
    for (const evt of filtered) {
      const dateKey = formatDate(evt.occurredAt)
      const arr = map.get(dateKey)
      if (arr) arr.push(evt)
      else map.set(dateKey, [evt])
    }
    return Array.from(map.entries())
  }, [filtered])

  return (
    <div className="space-y-4">
      {/* Search & filter bar */}
      <div className="flex items-center gap-2">
        <label
          className="flex flex-1 items-center gap-2.5 rounded-xl px-3 py-2.5"
          style={{
            background: 'var(--cp-surface)',
            border: '1px solid var(--cp-border)',
            color: 'var(--cp-muted)',
          }}
        >
          <Search size={16} />
          <input
            type="search"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder={t('taskCenter.events.search', 'Search events...')}
            className="w-full bg-transparent text-sm outline-none placeholder:text-[color:var(--cp-muted)]"
            style={{ color: 'var(--cp-text)' }}
          />
        </label>
        <button
          type="button"
          onClick={() => setShowFilters(!showFilters)}
          className="flex items-center gap-1.5 rounded-xl px-3 py-2.5 text-sm transition-colors"
          style={{
            background: showFilters ? 'color-mix(in srgb, var(--cp-accent) 12%, var(--cp-surface))' : 'var(--cp-surface)',
            color: showFilters ? 'var(--cp-accent)' : 'var(--cp-muted)',
            border: '1px solid var(--cp-border)',
          }}
        >
          <Filter size={14} />
        </button>
      </div>

      {showFilters && (
        <div
          className="flex flex-wrap gap-2 rounded-xl p-3"
          style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}
        >
          <select
            value={filterType}
            onChange={(e) => setFilterType(e.target.value as SystemEventType | '')}
            className="rounded-lg px-2.5 py-1.5 text-xs"
            style={{
              background: 'var(--cp-surface-2)',
              color: 'var(--cp-text)',
              border: '1px solid var(--cp-border)',
            }}
          >
            <option value="">{t('taskCenter.events.allTypes', 'All Event Types')}</option>
            {eventTypeOptions.filter(Boolean).map((tp) => (
              <option key={tp} value={tp}>{eventTypeLabel(tp as SystemEventType)}</option>
            ))}
          </select>
        </div>
      )}

      {/* Event count */}
      <div className="text-xs" style={{ color: 'var(--cp-muted)' }}>
        {filtered.length} {t('taskCenter.events.count', 'events')}
      </div>

      {/* Timeline grouped by date */}
      {grouped.map(([date, dayEvents]) => (
        <section key={date}>
          <h3
            className="text-xs font-semibold uppercase tracking-wide mb-2 sticky top-0 py-1"
            style={{ color: 'var(--cp-muted)', background: 'var(--cp-bg)' }}
          >
            {date}
          </h3>
          <div className="space-y-1.5">
            {dayEvents.map((evt) => {
              const hasTask = evt.relatedRootTaskId != null
              return (
                <button
                  key={evt.eventId}
                  type="button"
                  disabled={!hasTask}
                  onClick={() => {
                    if (hasTask) {
                      onNavigate({ page: 'tasks', taskId: evt.relatedRootTaskId! })
                    }
                  }}
                  className="flex w-full items-center gap-3 rounded-xl p-3 text-left transition-colors disabled:cursor-default"
                  style={{
                    background: 'var(--cp-surface)',
                    border: '1px solid var(--cp-border)',
                  }}
                >
                  {/* Timeline dot */}
                  <div
                    className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg"
                    style={{
                      background: `color-mix(in srgb, ${eventColor(evt.eventType)} 14%, transparent)`,
                      color: eventColor(evt.eventType),
                    }}
                  >
                    {eventIcon(evt.eventType)}
                  </div>
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-medium truncate" style={{ color: 'var(--cp-text)' }}>
                      {evt.title}
                    </div>
                    <div className="flex items-center gap-2 mt-0.5">
                      <span
                        className="text-xs font-medium"
                        style={{ color: eventColor(evt.eventType) }}
                      >
                        {eventTypeLabel(evt.eventType)}
                      </span>
                      <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>
                        · {evt.source} · {formatTime(evt.occurredAt)}
                      </span>
                    </div>
                    {evt.summary && (
                      <div className="text-xs mt-0.5 truncate" style={{ color: 'var(--cp-muted)' }}>
                        {evt.summary}
                      </div>
                    )}
                  </div>
                  {hasTask && (
                    <ChevronRight size={14} style={{ color: 'var(--cp-muted)' }} />
                  )}
                </button>
              )
            })}
          </div>
        </section>
      ))}

      {filtered.length === 0 && (
        <div className="text-center py-12 text-sm" style={{ color: 'var(--cp-muted)' }}>
          {t('taskCenter.events.empty', 'No events match your filters.')}
        </div>
      )}
    </div>
  )
}
