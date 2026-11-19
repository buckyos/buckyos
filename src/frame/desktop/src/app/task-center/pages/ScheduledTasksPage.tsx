/* ── TaskCenter Scheduled tasks page ── */

import { useState } from 'react'
import { Search } from 'lucide-react'
import { useTaskCenterStore } from '../hooks/use-task-center-store'
import { StatusDot } from '../components/taskView'
import { useTaskText, type Tone } from '../components/taskModel'
import type {
  Task,
  WorkflowScheduleSpec,
  WorkflowScheduleStatus,
  WorkflowScheduleTaskPayload,
} from '../../../api/task_mgr'
import type { TaskCenterNav } from '../components/layout/navigation'

interface ScheduledTasksPageProps {
  onNavigate: (nav: TaskCenterNav) => void
  selectedId?: string
}

interface ScheduleView {
  task: Task
  scheduleId: string
  name: string
  status: WorkflowScheduleStatus
  scheduleText: string
  timezone: string
  targetText: string
  nextFireAt: string | number | null
  lastFireAt: string | number | null
  lastTaskId: string | null
  consecutiveFailures: number
  lastError: string | null
}

const statusOptions: (WorkflowScheduleStatus | '')[] = ['', 'enabled', 'paused', 'error', 'archived']

function statusTone(status: WorkflowScheduleStatus): Tone {
  switch (status) {
    case 'enabled':
      return 'running'
    case 'paused':
      return 'waiting'
    case 'error':
      return 'failed'
    default:
      return 'idle'
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

function formatSchedule(spec: WorkflowScheduleSpec | undefined, dateTime: (value: string | number) => string | null) {
  if (!spec) return ''
  switch (spec.kind) {
    case 'cron':
      return spec.expr
    case 'once':
      return `once ${dateTime(spec.run_at) ?? spec.run_at}`
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

function toScheduleView(task: Task, dateTime: (value: string | number) => string | null): ScheduleView {
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
    scheduleText: formatSchedule(schedule, dateTime) || task.summary,
    timezone: schedule && 'timezone' in schedule && schedule.timezone ? schedule.timezone : 'UTC',
    targetText: target ? target.task_type : task.type,
    nextFireAt: result?.next_fire_at ?? null,
    lastFireAt: result?.last_fire_at ?? null,
    lastTaskId: result?.last_task_id == null ? null : String(result.last_task_id),
    consecutiveFailures: result?.consecutive_failures ?? 0,
    lastError,
  }
}

export function ScheduledTasksPage({ onNavigate, selectedId }: ScheduledTasksPageProps) {
  const store = useTaskCenterStore()
  const text = useTaskText()
  const { t } = text
  const [search, setSearch] = useState('')
  const [filterStatus, setFilterStatus] = useState<WorkflowScheduleStatus | ''>('')

  // Not memoized: the store keeps its identity and notifies through
  // useSyncExternalStore, so a memo keyed on it would never recompute.
  const schedules = store.getScheduledTasks().map((task) => toScheduleView(task, text.dateTime))
  const query = search.trim().toLowerCase()
  const filtered = schedules.filter((schedule) => {
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

  const stats = schedules.reduce(
    (acc, schedule) => {
      acc.total += 1
      acc[schedule.status] += 1
      return acc
    },
    { total: 0, enabled: 0, paused: 0, archived: 0, error: 0 },
  )

  const summaryStats = [
    { key: 'total', label: t('taskCenter.schedules.total', 'Total'), value: stats.total },
    { key: 'enabled', label: t('taskCenter.schedules.enabled', 'Enabled'), value: stats.enabled },
    { key: 'paused', label: t('taskCenter.schedules.paused', 'Paused'), value: stats.paused },
    { key: 'error', label: t('taskCenter.schedules.errors', 'Errors'), value: stats.error },
  ]

  const labelStatus = (status: WorkflowScheduleStatus) => t(`taskCenter.schedules.status.${status}`, status)

  return (
    <div className="space-y-4">
      <dl className="tc-surface grid grid-cols-4">
        {summaryStats.map((item, index) => (
          <div
            key={item.key}
            className="min-w-0 px-4 py-3"
            style={index > 0 ? { borderLeft: '1px solid var(--tc-line)' } : undefined}
          >
            <dt className="tc-muted truncate text-xs">{item.label}</dt>
            <dd
              className={`tc-num mt-1 text-xl font-medium leading-7${item.key === 'error' && item.value > 0 ? ' tc-danger' : ''}`}
            >
              {item.value}
            </dd>
          </div>
        ))}
      </dl>

      <div className="flex items-center gap-2">
        <label className="tc-search">
          <Search size={16} strokeWidth={1.75} className="shrink-0" />
          <input
            type="search"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            placeholder={t('taskCenter.schedules.search', 'Search schedules')}
            aria-label={t('taskCenter.schedules.search', 'Search schedules')}
          />
        </label>
        <select
          value={filterStatus}
          onChange={(event) => setFilterStatus(event.target.value as WorkflowScheduleStatus | '')}
          className="tc-select h-9"
          aria-label={t('taskCenter.schedules.allStatuses', 'All statuses')}
        >
          <option value="">{t('taskCenter.schedules.allStatuses', 'All statuses')}</option>
          {statusOptions.filter(Boolean).map((status) => (
            <option key={status} value={status}>{labelStatus(status as WorkflowScheduleStatus)}</option>
          ))}
        </select>
      </div>

      {filtered.length > 0 ? (
        <>
          <p className="tc-muted px-1 text-xs">
            {t('taskCenter.schedules.count', '{{count}} schedules', { count: filtered.length })}
          </p>
          <div className="tc-list">
            {filtered.map((schedule) => {
              const facts = [
                { label: t('taskCenter.schedules.rule', 'Schedule'), value: schedule.scheduleText, mono: true },
                {
                  label: t('taskCenter.schedules.nextRun', 'Next run'),
                  value: text.dateTime(schedule.nextFireAt) ?? t('taskCenter.schedules.noNextFire', 'None'),
                },
                { label: t('taskCenter.schedules.target', 'Target'), value: schedule.targetText, mono: true },
                {
                  label: t('taskCenter.schedules.lastRun', 'Last run'),
                  value: text.dateTime(schedule.lastFireAt) ?? t('taskCenter.schedules.neverFired', 'Never'),
                },
              ]
              const meta = [
                schedule.timezone,
                schedule.lastTaskId ? `${t('taskCenter.schedules.lastTask', 'Last task')} ${schedule.lastTaskId}` : null,
                schedule.consecutiveFailures > 0
                  ? t('taskCenter.schedules.failures', '{{count}} failures', { count: schedule.consecutiveFailures })
                  : null,
              ].filter(Boolean)
              const open = () => onNavigate({ page: 'schedules', taskId: schedule.task.taskId })
              return (
                <div
                  key={schedule.scheduleId}
                  role="button"
                  tabIndex={0}
                  className="tc-row"
                  aria-current={schedule.task.taskId === selectedId ? 'true' : undefined}
                  onClick={open}
                  onKeyDown={(event) => {
                    if (event.key === 'Enter' || event.key === ' ') {
                      event.preventDefault()
                      open()
                    }
                  }}
                  style={{ padding: 12, gap: 12 }}
                  data-testid="schedule-row"
                >
                  <span className="tc-row-dot">
                    <StatusDot tone={statusTone(schedule.status)} />
                  </span>
                  <div className="tc-row-body">
                    <div className="flex flex-wrap items-baseline gap-x-2">
                      <span className="text-sm font-medium leading-5">{schedule.name}</span>
                      <span className="tc-muted text-xs">{labelStatus(schedule.status)}</span>
                    </div>
                    <div className="tc-mono tc-muted truncate text-xs leading-4">{schedule.scheduleId}</div>
                    <dl className="mt-3 grid grid-cols-2 gap-x-4 gap-y-2 lg:grid-cols-4">
                      {facts.map((fact) => (
                        <div key={fact.label} className="min-w-0">
                          <dt className="tc-muted text-xs leading-4">{fact.label}</dt>
                          <dd className={`truncate text-[13px] leading-5${fact.mono ? ' tc-mono' : ''}`}>{fact.value}</dd>
                        </div>
                      ))}
                    </dl>
                    {schedule.lastError && <p className="tc-danger mt-2 text-xs leading-4">{schedule.lastError}</p>}
                    <p className="tc-muted mt-2 truncate text-xs leading-4">{meta.join(' · ')}</p>
                  </div>
                </div>
              )
            })}
          </div>
        </>
      ) : (
        <p className="tc-empty">{t('taskCenter.schedules.empty', 'No scheduled tasks match your filters')}</p>
      )}
    </div>
  )
}
