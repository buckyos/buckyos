/* ── TaskCenter Tasks page: every task as one tree ── */

import { useState } from 'react'
import { Download, Search, SlidersHorizontal } from 'lucide-react'
import { useTaskCenterStore } from '../hooks/use-task-center-store'
import { NewDownloadDialog } from '../components/NewDownloadDialog'
import { TaskTree } from '../components/TaskTree'
import { StatusDot } from '../components/taskView'
import { ancestorsOf, pruneTree, useTaskText } from '../components/taskModel'
import {
  matchesTaskFilter,
  type TaskCenterFilter,
  type TaskSource,
  type TaskStatus,
  type TaskType,
} from '../../../api/task_mgr'
import type { TaskCenterNav } from '../components/layout/navigation'

const statusOptions: TaskStatus[] = ['running', 'paused', 'pending', 'completed', 'failed', 'cancelled']
const typeOptions: TaskType[] = ['one-time', 'scheduled', 'download', 'sync', 'install', 'workflow']
const sourceOptions: TaskSource[] = ['system', 'user', 'agent', 'app']

interface TasksPageProps {
  onNavigate: (nav: TaskCenterNav) => void
  selectedId?: string
  followFocus?: boolean
}

export function TasksPage({ onNavigate, selectedId, followFocus }: TasksPageProps) {
  const store = useTaskCenterStore()
  const text = useTaskText()
  const { t } = text
  const [search, setSearch] = useState('')
  const [filterStatus, setFilterStatus] = useState<TaskStatus | ''>('')
  const [filterType, setFilterType] = useState<TaskType | ''>('')
  const [filterSource, setFilterSource] = useState<TaskSource | ''>('')
  const [showFilters, setShowFilters] = useState(false)
  const [downloadOpen, setDownloadOpen] = useState(false)
  /** Nodes the user opened or closed, with the selection they did it under. */
  const [toggled, setToggled] = useState(new Map<string, { open: boolean; selection?: string }>())

  // Not memoized: the store keeps its identity and notifies through
  // useSyncExternalStore, so a memo keyed on it would never recompute.
  const all = [...store.getAllTasks()].sort(
    (a, b) => new Date(b.updatedAt).getTime() - new Date(a.updatedAt).getTime(),
  )
  const filter: TaskCenterFilter = {
    status: filterStatus || undefined,
    type: filterType || undefined,
    source: filterSource || undefined,
    search: search.trim() || undefined,
  }
  const filtering = Object.values(filter).some(Boolean)
  const filtersSet = Boolean(filterStatus || filterType || filterSource)
  const { roots, open: matchPath } = filtering
    ? pruneTree(all, (task) => matchesTaskFilter(task, filter))
    : { roots: all, open: new Set<string>() }
  const selectionPath = new Set(ancestorsOf(roots, selectedId))

  // Collapsed by default; the path to the selected task and to search hits
  // opens by itself, unless the user closed it while that task was selected.
  const isOpen = (taskId: string) => {
    const entry = toggled.get(taskId)
    if (selectionPath.has(taskId) && entry?.selection !== selectedId) return true
    if (entry) return entry.open
    return matchPath.has(taskId)
  }
  const toggle = (taskId: string) => {
    const next = new Map(toggled)
    next.set(taskId, { open: !isOpen(taskId), selection: selectedId })
    setToggled(next)
  }

  const clearFilters = () => {
    setFilterStatus('')
    setFilterType('')
    setFilterSource('')
  }

  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2">
        <label className="tc-search">
          <Search size={16} strokeWidth={1.75} className="shrink-0" />
          <input
            type="search"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            placeholder={t('taskCenter.tasks.search', 'Search tasks')}
            aria-label={t('taskCenter.tasks.search', 'Search tasks')}
          />
        </label>
        <button
          type="button"
          onClick={() => setShowFilters(!showFilters)}
          className="tc-icon-btn"
          aria-pressed={showFilters}
          aria-label={t('taskCenter.tasks.filters', 'Filters')}
          title={t('taskCenter.tasks.filters', 'Filters')}
        >
          <SlidersHorizontal size={16} strokeWidth={1.75} />
          {filtersSet && <StatusDot tone="running" />}
        </button>
        <button type="button" onClick={() => setDownloadOpen(true)} className="tc-btn h-9">
          <Download size={16} strokeWidth={1.75} />
          {t('taskCenter.download.new', 'New download')}
        </button>
      </div>
      <NewDownloadDialog
        open={downloadOpen}
        onClose={() => setDownloadOpen(false)}
        onCreated={(taskId) => {
          setDownloadOpen(false)
          onNavigate({ page: 'tasks', taskId })
        }}
      />

      {showFilters && (
        <div className="flex flex-wrap items-center gap-2">
          <select
            value={filterStatus}
            onChange={(event) => setFilterStatus(event.target.value as TaskStatus | '')}
            className="tc-select"
            aria-label={t('taskCenter.detail.status', 'Status')}
          >
            <option value="">{t('taskCenter.tasks.allStatuses', 'All statuses')}</option>
            {statusOptions.map((status) => (
              <option key={status} value={status}>{text.status({ status })}</option>
            ))}
          </select>
          <select
            value={filterType}
            onChange={(event) => setFilterType(event.target.value as TaskType | '')}
            className="tc-select"
            aria-label={t('taskCenter.detail.type', 'Type')}
          >
            <option value="">{t('taskCenter.tasks.allTypes', 'All types')}</option>
            {typeOptions.map((type) => (
              <option key={type} value={type}>{text.type(type)}</option>
            ))}
          </select>
          <select
            value={filterSource}
            onChange={(event) => setFilterSource(event.target.value as TaskSource | '')}
            className="tc-select"
            aria-label={t('taskCenter.detail.source', 'Source')}
          >
            <option value="">{t('taskCenter.tasks.allSources', 'All sources')}</option>
            {sourceOptions.map((source) => (
              <option key={source} value={source}>{text.source(source)}</option>
            ))}
          </select>
          {filtersSet && (
            <button type="button" onClick={clearFilters} className="tc-link px-2 text-[13px]">
              {t('taskCenter.tasks.clearFilters', 'Clear')}
            </button>
          )}
        </div>
      )}

      {roots.length > 0 ? (
        <>
          <p className="tc-muted px-1 text-xs">
            {t('taskCenter.tasks.count', '{{count}} tasks', { count: roots.length })}
          </p>
          <TaskTree
            roots={roots}
            text={text}
            isOpen={isOpen}
            onToggle={toggle}
            selectedId={selectedId}
            onSelect={(taskId) => onNavigate({ page: 'tasks', taskId })}
            followFocus={followFocus}
          />
        </>
      ) : (
        <p className="tc-empty">{t('taskCenter.tasks.empty', 'No tasks match your filters')}</p>
      )}
    </div>
  )
}
