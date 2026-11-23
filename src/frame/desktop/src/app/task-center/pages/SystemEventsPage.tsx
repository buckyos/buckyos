/* ── TaskCenter System events page ── */

import { useState } from 'react'
import { Search } from 'lucide-react'
import { useTaskCenterStore } from '../hooks/use-task-center-store'
import { StatusDot } from '../components/taskView'
import { useTaskText, type Tone } from '../components/taskModel'
import type { SystemEvent, SystemEventType } from '../../../api/task_mgr'
import type { TaskCenterNav } from '../components/layout/navigation'

function eventTone(eventType: SystemEventType): Tone {
  switch (eventType) {
    case 'task_completed':
    case 'notification_handled':
      return 'done'
    case 'task_failed':
      return 'failed'
    case 'task_milestone':
      return 'running'
    case 'notification_created':
      return 'waiting'
    case 'task_created':
      return 'pending'
    default:
      return 'idle'
  }
}

const eventTypeOptions: SystemEventType[] = [
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
  selectedId?: string
}

export function SystemEventsPage({ onNavigate, selectedId }: SystemEventsPageProps) {
  const store = useTaskCenterStore()
  const text = useTaskText()
  const { t } = text
  const [search, setSearch] = useState('')
  const [filterType, setFilterType] = useState<SystemEventType | ''>('')

  const typeLabel = (eventType: SystemEventType) => t(`taskCenter.events.type.${eventType}`, eventType)
  const sourceLabel = (source: string) => t(`taskCenter.source.${source}`, source)
  /** The task an event opens: the one it names when it is still listed, else its root. */
  const targetOf = (event: SystemEvent) =>
    event.relatedTaskId && store.getTaskById(event.relatedTaskId) ? event.relatedTaskId : event.relatedRootTaskId

  const query = search.trim().toLowerCase()
  const filtered = store.getEvents().filter((event) => {
    if (filterType && event.eventType !== filterType) return false
    if (!query) return true
    return [event.title, event.summary, event.source].some((value) => value.toLowerCase().includes(query))
  })

  const grouped = new Map<string, SystemEvent[]>()
  for (const event of filtered) {
    const day = text.day(event.occurredAt)
    const list = grouped.get(day)
    if (list) list.push(event)
    else grouped.set(day, [event])
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center gap-2">
        <label className="tc-search">
          <Search size={16} strokeWidth={1.75} className="shrink-0" />
          <input
            type="search"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            placeholder={t('taskCenter.events.search', 'Search events')}
            aria-label={t('taskCenter.events.search', 'Search events')}
          />
        </label>
        <select
          value={filterType}
          onChange={(event) => setFilterType(event.target.value as SystemEventType | '')}
          className="tc-select h-9"
          aria-label={t('taskCenter.events.allTypes', 'All event types')}
        >
          <option value="">{t('taskCenter.events.allTypes', 'All event types')}</option>
          {eventTypeOptions.map((type) => (
            <option key={type} value={type}>{typeLabel(type)}</option>
          ))}
        </select>
      </div>

      {filtered.length > 0 ? (
        <p className="tc-muted px-1 text-xs">
          {t('taskCenter.events.count', '{{count}} events', { count: filtered.length })}
        </p>
      ) : (
        <p className="tc-empty">{t('taskCenter.events.empty', 'No events match your filters')}</p>
      )}

      {[...grouped].map(([day, events]) => (
        <section key={day}>
          <h3 className="tc-heading sticky top-0 z-[1] -mx-1 mb-2 px-2 py-1" style={{ background: 'var(--cp-bg)' }}>
            {day}
          </h3>
          <div className="tc-list">
            {events.map((event) => {
              const target = targetOf(event)
              const open = () => {
                if (target) onNavigate({ page: 'events', taskId: target })
              }
              return (
                <div
                  key={event.eventId}
                  role="button"
                  tabIndex={target ? 0 : -1}
                  aria-disabled={target ? undefined : 'true'}
                  aria-current={target && target === selectedId ? 'true' : undefined}
                  className="tc-row"
                  onClick={open}
                  onKeyDown={(keyEvent) => {
                    if (keyEvent.key === 'Enter' || keyEvent.key === ' ') {
                      keyEvent.preventDefault()
                      open()
                    }
                  }}
                >
                  <span className="tc-row-dot">
                    <StatusDot tone={eventTone(event.eventType)} />
                  </span>
                  <span className="tc-row-body">
                    <span className="tc-row-title">{event.title}</span>
                    <span className="tc-row-meta">
                      {[typeLabel(event.eventType), sourceLabel(event.source), event.summary].filter(Boolean).join(' · ')}
                    </span>
                  </span>
                  <span className="tc-row-aside">
                    <span>{text.timeOfDay(event.occurredAt)}</span>
                  </span>
                </div>
              )
            })}
          </div>
        </section>
      ))}
    </div>
  )
}
