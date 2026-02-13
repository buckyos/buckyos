import { useCallback, useEffect, useMemo, useState } from 'react'

import { fetchWsTasks, fetchSteps } from '@/api/workspace'
import { useCurrentRun, useWorkspace } from '../WorkspaceContext'
import FilterBar from '../components/FilterBar'
import TaskRow from '../components/TaskRow'

const statusOptions = [
  { value: 'queued', label: 'Queued' },
  { value: 'running', label: 'Running' },
  { value: 'success', label: 'Success' },
  { value: 'failed', label: 'Failed' },
]

const TasksTab = () => {
  const { selectedRunId, liveMode, openInspector } = useWorkspace()
  const run = useCurrentRun()

  const [tasks, setTasks] = useState<WsTask[]>([])
  const [steps, setSteps] = useState<WsStep[]>([])
  const [loading, setLoading] = useState(true)
  const [stepFilter, setStepFilter] = useState('')
  const [statusFilter, setStatusFilter] = useState('')
  const [keyword, setKeyword] = useState('')

  const loadData = useCallback(
    async (silent = false) => {
      if (!selectedRunId) return
      if (!silent) setLoading(true)
      const [taskRes, stepRes] = await Promise.all([
        fetchWsTasks(selectedRunId, {
          stepId: stepFilter || undefined,
          status: (statusFilter || undefined) as WsTaskStatus | undefined,
        }),
        fetchSteps(selectedRunId),
      ])
      if (taskRes.data) setTasks(taskRes.data)
      if (stepRes.data) setSteps(stepRes.data)
      setLoading(false)
    },
    [selectedRunId, stepFilter, statusFilter],
  )

  useEffect(() => {
    loadData()
  }, [loadData])

  useEffect(() => {
    if (!liveMode || !run || run.status !== 'running') return
    const id = window.setInterval(() => loadData(true), 5000)
    return () => window.clearInterval(id)
  }, [liveMode, run, loadData])

  const stepOptions = useMemo(
    () =>
      steps.map((s) => ({
        value: s.step_id,
        label: `Step ${s.step_index}${s.title ? `: ${s.title}` : ''}`,
      })),
    [steps],
  )

  const filteredTasks = useMemo(() => {
    if (!keyword) return tasks
    const kw = keyword.toLowerCase()
    return tasks.filter(
      (t) =>
        t.prompt_preview.toLowerCase().includes(kw) ||
        t.result_preview.toLowerCase().includes(kw) ||
        t.model.toLowerCase().includes(kw),
    )
  }, [tasks, keyword])

  const totals = useMemo(
    () => ({
      total: filteredTasks.length,
      running: filteredTasks.filter((t) => t.status === 'running').length,
      failed: filteredTasks.filter((t) => t.status === 'failed').length,
    }),
    [filteredTasks],
  )

  return (
    <div className="space-y-4">
      {/* Filter bar */}
      <FilterBar
        keyword={keyword}
        onKeywordChange={setKeyword}
        placeholder="Search tasks..."
        dropdowns={[
          { label: 'Step', value: stepFilter, options: stepOptions, onChange: setStepFilter },
          { label: 'Status', value: statusFilter, options: statusOptions, onChange: setStatusFilter },
        ]}
      />

      {/* Stats strip */}
      <div className="flex gap-3 text-xs">
        <span className="text-[var(--cp-muted)]">
          Total: <span className="font-semibold text-[var(--cp-ink)]">{totals.total}</span>
        </span>
        {totals.running > 0 && (
          <span className="text-[var(--cp-primary)]">
            Running: <span className="font-semibold">{totals.running}</span>
          </span>
        )}
        {totals.failed > 0 && (
          <span className="text-rose-600">
            Failed: <span className="font-semibold">{totals.failed}</span>
          </span>
        )}
      </div>

      {/* Task list */}
      <div className="rounded-2xl border border-[var(--cp-border)] bg-white">
        {loading ? (
          <div className="space-y-1 p-2">
            {Array.from({ length: 5 }).map((_, i) => (
              <div key={`task-skeleton-${i}`} className="flex animate-pulse items-center gap-3 px-3 py-2.5">
                <div className="h-5 w-14 rounded-full bg-[var(--cp-surface-muted)]" />
                <div className="flex-1 space-y-1.5">
                  <div className="h-2.5 w-40 rounded-full bg-[var(--cp-surface-muted)]" />
                  <div className="h-2.5 w-56 rounded-full bg-[var(--cp-surface-muted)]" />
                </div>
              </div>
            ))}
          </div>
        ) : filteredTasks.length > 0 ? (
          <div className="divide-y divide-[var(--cp-border)]/30 p-1">
            {filteredTasks.map((task) => (
              <TaskRow
                key={task.task_id}
                task={task}
                onClick={() => openInspector({ kind: 'task', data: task })}
              />
            ))}
          </div>
        ) : (
          <div className="px-4 py-8 text-center text-sm text-[var(--cp-muted)]">
            <p className="text-[var(--cp-ink)]">No tasks found.</p>
            <p className="mt-1 text-xs">Adjust filters or select a different run.</p>
          </div>
        )}
      </div>
    </div>
  )
}

export default TasksTab
