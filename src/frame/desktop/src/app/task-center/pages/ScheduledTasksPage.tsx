import { useMemo, useState } from 'react'
import {
  Archive,
  CalendarClock,
  CheckCircle2,
  ChevronRight,
  Clock,
  Filter,
  Pause,
  Play,
  Search,
  Timer,
  Workflow,
  XCircle,
} from 'lucide-react'
import { useI18n } from '../../../i18n/provider'
import { useTaskCenterStore } from '../hooks/use-task-center-store'
import type {
  Task,
  WorkflowScheduleSpec,
  WorkflowScheduleStatus,
  WorkflowScheduleTaskPayload,
} from '../../../api/task_mgr'
import type { TaskCenterNav } from '../components/layout/navigation'

interface ScheduledTasksPageProps {
  onNavigate: (nav: TaskCenterNav) => void
}

interface ScheduleView {
  task: Task
  scheduleId: string
  name: string
  status: WorkflowScheduleStatus
  scheduleText: string
  timezone: string
  targetText: string
  nextFireAt: string | null
  lastFireAt: string | null
  lastTaskId: string | null
  consecutiveFailures: number
  lastError: string | null
}

const statusOptions: (WorkflowScheduleStatus | '')[] = ['', 'enabled', 'paused', 'error', 'archived']

function statusIcon(status: WorkflowScheduleStatus) {
  switch (status) {
    case 'enabled':
      return <Play size={14} />
    case 'paused':
      return <Pause size={14} />
    case 'archived':
      return <Archive size={14} />
    case 'error':
      return <XCircle size={14} />
    default:
      return <Clock size={14} />
  }
}

function statusColor(status: WorkflowScheduleStatus) {
  switch (status) {
    case 'enabled':
      return 'var(--cp-accent)'
    case 'paused':
      return 'var(--cp-warning)'
    case 'archived':
      return 'var(--cp-muted)'
    case 'error':
      return 'var(--cp-danger)'
    default:
      return 'var(--cp-muted)'
  }
}

function statusFromTask(task: Task): WorkflowScheduleStatus {
  switch (task.status) {
    case 'running':
      return 'enabled'
    case 'paused':
      return 'paused'
    case 'failed':
      return 'error'
    case 'cancelled':
    case 'completed':
      return 'archived'
    default:
      return 'enabled'
  }
}

function formatTimestamp(value: string | number | null | undefined) {
  if (value == null || value === '') return null
  const date = typeof value === 'number'
    ? new Date(value < 10_000_000_000 ? value * 1000 : value)
    : new Date(value)
  if (Number.isNaN(date.getTime())) return null
  return date.toLocaleString(undefined, {
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  })
}

function formatSchedule(spec: WorkflowScheduleSpec | undefined) {
  if (!spec) return ''
  switch (spec.kind) {
    case 'cron':
      return spec.expr
    case 'once':
      return `once ${formatTimestamp(spec.run_at) ?? spec.run_at}`
    case 'run_every':
      return `every ${formatDuration(spec.every_sec)}`
    default:
      return ''
  }
}

function formatDuration(seconds: number) {
  if (seconds % 86_400 === 0) return `${seconds / 86_400}d`
  if (seconds % 3_600 === 0) return `${seconds / 3_600}h`
  if (seconds % 60 === 0) return `${seconds / 60}m`
  return `${seconds}s`
}

function toScheduleView(task: Task): ScheduleView {
  const payload = task.payload as WorkflowScheduleTaskPayload
  const request = payload.request
  const result = payload.result
  const status = request?.status ?? statusFromTask(task)
  const schedule = request?.schedule
  const target = request?.target
  const lastError = result?.last_error == null ? null : String(result.last_error)

  return {
    task,
    scheduleId: request?.schedule_id ?? task.rootTaskId,
    name: request?.name ?? task.title.replace(/^workflow\/schedule\//, ''),
    status,
    scheduleText: formatSchedule(schedule) || task.summary,
    timezone: schedule && 'timezone' in schedule && schedule.timezone ? schedule.timezone : 'UTC',
    targetText: target ? target.task_type : task.type,
    nextFireAt: formatTimestamp(result?.next_fire_at),
    lastFireAt: formatTimestamp(result?.last_fire_at),
    lastTaskId: result?.last_task_id == null ? null : String(result.last_task_id),
    consecutiveFailures: result?.consecutive_failures ?? 0,
    lastError,
  }
}

export function ScheduledTasksPage({ onNavigate }: ScheduledTasksPageProps) {
  const store = useTaskCenterStore()
  const { t } = useI18n()
  const [search, setSearch] = useState('')
  const [filterStatus, setFilterStatus] = useState<WorkflowScheduleStatus | ''>('')
  const [showFilters, setShowFilters] = useState(false)

  const schedules = useMemo(() => store.getScheduledTasks().map(toScheduleView), [store])
  const filtered = useMemo(() => {
    const query = search.trim().toLowerCase()
    return schedules.filter((schedule) => {
      if (filterStatus && schedule.status !== filterStatus) return false
      if (!query) return true
      return [
        schedule.name,
        schedule.scheduleId,
        schedule.scheduleText,
        schedule.targetText,
        schedule.timezone,
      ].some((value) => value.toLowerCase().includes(query))
    })
  }, [filterStatus, schedules, search])

  const stats = useMemo(() => {
    return schedules.reduce(
      (acc, schedule) => {
        acc.total += 1
        acc[schedule.status] += 1
        return acc
      },
      { total: 0, enabled: 0, paused: 0, archived: 0, error: 0 },
    )
  }, [schedules])

  const summaryStats = [
    {
      key: 'total',
      label: t('taskCenter.schedules.total', 'Total'),
      value: stats.total,
      icon: <CalendarClock size={13} />,
      color: 'var(--cp-text)',
    },
    {
      key: 'enabled',
      label: t('taskCenter.schedules.enabled', 'Enabled'),
      value: stats.enabled,
      icon: <Play size={13} />,
      color: 'var(--cp-accent)',
    },
    {
      key: 'paused',
      label: t('taskCenter.schedules.paused', 'Paused'),
      value: stats.paused,
      icon: <Pause size={13} />,
      color: 'var(--cp-warning)',
    },
    {
      key: 'error',
      label: t('taskCenter.schedules.errors', 'Errors'),
      value: stats.error,
      icon: <XCircle size={13} />,
      color: 'var(--cp-danger)',
    },
  ]

  const labelStatus = (status: WorkflowScheduleStatus) => {
    switch (status) {
      case 'enabled':
        return t('taskCenter.schedules.status.enabled', 'Enabled')
      case 'paused':
        return t('taskCenter.schedules.status.paused', 'Paused')
      case 'archived':
        return t('taskCenter.schedules.status.archived', 'Archived')
      case 'error':
        return t('taskCenter.schedules.status.error', 'Error')
      default:
        return status
    }
  }

  return (
    <div className="space-y-4">
      <div
        className="grid grid-cols-4 overflow-hidden rounded-xl"
        style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}
      >
        {summaryStats.map((item, index) => (
          <div
            key={item.key}
            className="flex min-w-0 items-center justify-center gap-1.5 px-1.5 py-2 sm:gap-2 sm:px-3"
            style={{
              borderLeft: index === 0 ? undefined : '1px solid var(--cp-border)',
              color: 'var(--cp-muted)',
            }}
          >
            <span className="hidden shrink-0 sm:inline-flex" style={{ color: item.color }}>
              {item.icon}
            </span>
            <span className="min-w-0 truncate text-[11px] sm:text-xs">
              {item.label}
            </span>
            <span className="shrink-0 text-sm font-semibold sm:text-base" style={{ color: item.color }}>
              {item.value}
            </span>
          </div>
        ))}
      </div>

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
            onChange={(event) => setSearch(event.target.value)}
            placeholder={t('taskCenter.schedules.search', 'Search schedules...')}
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
            value={filterStatus}
            onChange={(event) => setFilterStatus(event.target.value as WorkflowScheduleStatus | '')}
            className="rounded-lg px-2.5 py-1.5 text-xs"
            style={{
              background: 'var(--cp-surface-2)',
              color: 'var(--cp-text)',
              border: '1px solid var(--cp-border)',
            }}
          >
            <option value="">{t('taskCenter.schedules.allStatuses', 'All Statuses')}</option>
            {statusOptions.filter(Boolean).map((status) => (
              <option key={status} value={status}>{labelStatus(status as WorkflowScheduleStatus)}</option>
            ))}
          </select>
        </div>
      )}

      <div className="text-xs" style={{ color: 'var(--cp-muted)' }}>
        {t('taskCenter.schedules.count', '{{count}} schedules', { count: filtered.length })}
      </div>

      <div className="space-y-2">
        {filtered.map((schedule) => (
          <button
            key={schedule.scheduleId}
            type="button"
            onClick={() => onNavigate({ page: 'schedules', taskId: schedule.task.taskId })}
            className="w-full rounded-xl p-4 text-left transition-colors"
            style={{
              background: 'var(--cp-surface)',
              border: '1px solid var(--cp-border)',
            }}
          >
            <div className="flex items-start gap-3">
              <div
                className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg"
                style={{
                  background: `color-mix(in srgb, ${statusColor(schedule.status)} 14%, transparent)`,
                  color: statusColor(schedule.status),
                }}
              >
                {statusIcon(schedule.status)}
              </div>
              <div className="min-w-0 flex-1">
                <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
                  <span className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>
                    {schedule.name}
                  </span>
                  <span
                    className="text-xs font-medium uppercase"
                    style={{ color: statusColor(schedule.status) }}
                  >
                    {labelStatus(schedule.status)}
                  </span>
                </div>
                <div className="mt-0.5 text-xs" style={{ color: 'var(--cp-muted)' }}>
                  {schedule.scheduleId}
                </div>
                <div className="mt-3 grid gap-2 text-xs sm:grid-cols-2 lg:grid-cols-4">
                  <div className="flex min-w-0 items-center gap-1.5">
                    <Timer size={13} style={{ color: 'var(--cp-muted)' }} />
                    <span className="truncate" style={{ color: 'var(--cp-text)' }}>
                      {schedule.scheduleText}
                    </span>
                  </div>
                  <div className="flex min-w-0 items-center gap-1.5">
                    <Clock size={13} style={{ color: 'var(--cp-muted)' }} />
                    <span className="truncate" style={{ color: 'var(--cp-text)' }}>
                      {schedule.nextFireAt ?? t('taskCenter.schedules.noNextFire', 'No next fire')}
                    </span>
                  </div>
                  <div className="flex min-w-0 items-center gap-1.5">
                    <Workflow size={13} style={{ color: 'var(--cp-muted)' }} />
                    <span className="truncate" style={{ color: 'var(--cp-text)' }}>
                      {schedule.targetText}
                    </span>
                  </div>
                  <div className="flex min-w-0 items-center gap-1.5">
                    <CheckCircle2 size={13} style={{ color: 'var(--cp-muted)' }} />
                    <span className="truncate" style={{ color: 'var(--cp-text)' }}>
                      {schedule.lastFireAt ?? t('taskCenter.schedules.neverFired', 'Never fired')}
                    </span>
                  </div>
                </div>
                {schedule.lastError && (
                  <div className="mt-3 text-xs" style={{ color: 'var(--cp-danger)' }}>
                    {schedule.lastError}
                  </div>
                )}
                <div className="mt-3 flex flex-wrap gap-2 text-xs" style={{ color: 'var(--cp-muted)' }}>
                  <span>{schedule.timezone}</span>
                  {schedule.lastTaskId && (
                    <span>
                      {t('taskCenter.schedules.lastTask', 'Last task')}: {schedule.lastTaskId}
                    </span>
                  )}
                  {schedule.consecutiveFailures > 0 && (
                    <span>
                      {t('taskCenter.schedules.failures', '{{count}} failures', {
                        count: schedule.consecutiveFailures,
                      })}
                    </span>
                  )}
                </div>
              </div>
              <ChevronRight size={14} style={{ color: 'var(--cp-muted)', marginTop: 10 }} />
            </div>
          </button>
        ))}
      </div>

      {filtered.length === 0 && (
        <div className="text-center py-12 text-sm" style={{ color: 'var(--cp-muted)' }}>
          {t('taskCenter.schedules.empty', 'No scheduled tasks match your filters.')}
        </div>
      )}
    </div>
  )
}
