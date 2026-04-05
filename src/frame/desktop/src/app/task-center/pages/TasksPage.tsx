/* ── TaskCenter Tasks Page (full task list with filters) ── */

import { useState, useMemo } from 'react'
import {
  Search,
  Play,
  CheckCircle2,
  XCircle,
  Clock,
  ChevronRight,
  Filter,
  Pause,
} from 'lucide-react'
import { useI18n } from '../../../i18n/provider'
import { useTaskCenterStore } from '../hooks/use-task-center-store'
import type { Task, TaskStatus, TaskType, TaskSource } from '../mock/types'
import type { TaskCenterNav } from '../components/layout/navigation'

function statusIcon(status: Task['status']) {
  switch (status) {
    case 'running':
      return <Play size={14} />
    case 'paused':
      return <Pause size={14} />
    case 'completed':
      return <CheckCircle2 size={14} />
    case 'failed':
      return <XCircle size={14} />
    default:
      return <Clock size={14} />
  }
}

function statusColor(status: Task['status']) {
  switch (status) {
    case 'running':
      return 'var(--cp-accent)'
    case 'paused':
      return 'var(--cp-warning)'
    case 'completed':
      return 'var(--cp-success)'
    case 'failed':
      return 'var(--cp-danger)'
    default:
      return 'var(--cp-muted)'
  }
}

function formatTime(iso: string) {
  const d = new Date(iso)
  return d.toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })
}

const statusOptions: (TaskStatus | '')[] = ['', 'running', 'paused', 'pending', 'completed', 'failed', 'cancelled']
const typeOptions: (TaskType | '')[] = ['', 'one-time', 'scheduled', 'download', 'sync', 'install', 'workflow']
const sourceOptions: (TaskSource | '')[] = ['', 'system', 'user', 'agent', 'app']

interface TasksPageProps {
  onNavigate: (nav: TaskCenterNav) => void
}

export function TasksPage({ onNavigate }: TasksPageProps) {
  const store = useTaskCenterStore()
  const { t } = useI18n()
  const [search, setSearch] = useState('')
  const [filterStatus, setFilterStatus] = useState<TaskStatus | ''>('')
  const [filterType, setFilterType] = useState<TaskType | ''>('')
  const [filterSource, setFilterSource] = useState<TaskSource | ''>('')
  const [showFilters, setShowFilters] = useState(false)

  const tasks = useMemo(() => {
    return store.filterTasks({
      status: filterStatus || undefined,
      type: filterType || undefined,
      source: filterSource || undefined,
      search: search || undefined,
    })
  }, [store, filterStatus, filterType, filterSource, search])

  // Sort by updatedAt descending
  const sorted = useMemo(
    () => [...tasks].sort((a, b) => new Date(b.updatedAt).getTime() - new Date(a.updatedAt).getTime()),
    [tasks],
  )

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
            placeholder={t('taskCenter.tasks.search', 'Search tasks...')}
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

      {/* Filters */}
      {showFilters && (
        <div
          className="flex flex-wrap gap-2 rounded-xl p-3"
          style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}
        >
          <select
            value={filterStatus}
            onChange={(e) => setFilterStatus(e.target.value as TaskStatus | '')}
            className="rounded-lg px-2.5 py-1.5 text-xs"
            style={{
              background: 'var(--cp-surface-2)',
              color: 'var(--cp-text)',
              border: '1px solid var(--cp-border)',
            }}
          >
            <option value="">{t('taskCenter.tasks.allStatuses', 'All Statuses')}</option>
            {statusOptions.filter(Boolean).map((s) => (
              <option key={s} value={s}>{s}</option>
            ))}
          </select>
          <select
            value={filterType}
            onChange={(e) => setFilterType(e.target.value as TaskType | '')}
            className="rounded-lg px-2.5 py-1.5 text-xs"
            style={{
              background: 'var(--cp-surface-2)',
              color: 'var(--cp-text)',
              border: '1px solid var(--cp-border)',
            }}
          >
            <option value="">{t('taskCenter.tasks.allTypes', 'All Types')}</option>
            {typeOptions.filter(Boolean).map((tp) => (
              <option key={tp} value={tp}>{tp}</option>
            ))}
          </select>
          <select
            value={filterSource}
            onChange={(e) => setFilterSource(e.target.value as TaskSource | '')}
            className="rounded-lg px-2.5 py-1.5 text-xs"
            style={{
              background: 'var(--cp-surface-2)',
              color: 'var(--cp-text)',
              border: '1px solid var(--cp-border)',
            }}
          >
            <option value="">{t('taskCenter.tasks.allSources', 'All Sources')}</option>
            {sourceOptions.filter(Boolean).map((src) => (
              <option key={src} value={src}>{src}</option>
            ))}
          </select>
        </div>
      )}

      {/* Task count */}
      <div className="text-xs" style={{ color: 'var(--cp-muted)' }}>
        {sorted.length} {t('taskCenter.tasks.count', 'tasks')}
      </div>

      {/* Task list */}
      <div className="space-y-1.5">
        {sorted.map((task) => (
          <button
            key={task.taskId}
            type="button"
            onClick={() => onNavigate({ page: 'tasks', taskId: task.taskId })}
            className="flex w-full items-center gap-3 rounded-xl p-3 text-left transition-colors"
            style={{
              background: 'var(--cp-surface)',
              border: '1px solid var(--cp-border)',
            }}
          >
            <div
              className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg"
              style={{
                background: `color-mix(in srgb, ${statusColor(task.status)} 14%, transparent)`,
                color: statusColor(task.status),
              }}
            >
              {statusIcon(task.status)}
            </div>
            <div className="flex-1 min-w-0">
              <div className="text-sm font-medium truncate" style={{ color: 'var(--cp-text)' }}>
                {task.title}
              </div>
              <div className="flex items-center gap-2 mt-0.5">
                <span
                  className="text-xs font-medium uppercase"
                  style={{ color: statusColor(task.status) }}
                >
                  {task.status}
                </span>
                <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>
                  · {task.type} · {task.source}
                </span>
              </div>
              {task.progress != null && (task.status === 'running' || task.status === 'paused') && (
                <div className="mt-1.5">
                  <div
                    className="h-1 w-full max-w-[200px] rounded-full overflow-hidden"
                    style={{ background: 'var(--cp-border)' }}
                  >
                    <div
                      className="h-full rounded-full"
                      style={{
                        width: `${task.progress}%`,
                        background: statusColor(task.status),
                      }}
                    />
                  </div>
                </div>
              )}
            </div>
            <div className="shrink-0 text-right">
              <div className="text-xs" style={{ color: 'var(--cp-muted)' }}>
                {formatTime(task.updatedAt)}
              </div>
            </div>
            <ChevronRight size={14} style={{ color: 'var(--cp-muted)' }} />
          </button>
        ))}
      </div>

      {sorted.length === 0 && (
        <div className="text-center py-12 text-sm" style={{ color: 'var(--cp-muted)' }}>
          {t('taskCenter.tasks.empty', 'No tasks match your filters.')}
        </div>
      )}
    </div>
  )
}
