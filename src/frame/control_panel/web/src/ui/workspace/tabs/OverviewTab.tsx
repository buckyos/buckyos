import { useCallback, useEffect, useState } from 'react'

import { fetchSteps, fetchTodos, fetchSubAgents } from '@/api/workspace'
import Icon from '../../icons'
import { useCurrentRun, useWorkspace } from '../WorkspaceContext'
import StepCard from '../components/StepCard'
import StatusPill from '../components/StatusPill'

const OverviewTab = () => {
  const { selectedAgentId, selectedRunId, liveMode, setActiveTab, openInspector } = useWorkspace()
  const run = useCurrentRun()
  const [steps, setSteps] = useState<WsStep[]>([])
  const [todos, setTodos] = useState<WsTodo[]>([])
  const [subAgents, setSubAgents] = useState<WsAgent[]>([])
  const [loading, setLoading] = useState(true)

  const loadData = useCallback(
    async (silent = false) => {
      if (!selectedRunId || !selectedAgentId) return
      if (!silent) setLoading(true)
      const [stepRes, todoRes, subRes] = await Promise.all([
        fetchSteps(selectedRunId),
        fetchTodos(selectedAgentId),
        fetchSubAgents(selectedAgentId),
      ])
      if (stepRes.data) setSteps(stepRes.data)
      if (todoRes.data) setTodos(todoRes.data)
      if (subRes.data) setSubAgents(subRes.data)
      setLoading(false)
    },
    [selectedRunId, selectedAgentId],
  )

  useEffect(() => {
    loadData()
  }, [loadData])

  // Polling
  useEffect(() => {
    if (!liveMode || !run || run.status !== 'running') return
    const id = window.setInterval(() => loadData(true), 5000)
    return () => window.clearInterval(id)
  }, [liveMode, run, loadData])

  if (loading) {
    return (
      <div className="space-y-4">
        {Array.from({ length: 4 }).map((_, i) => (
          <div
            key={`step-skeleton-${i}`}
            className="animate-pulse rounded-2xl border border-[var(--cp-border)] bg-white px-4 py-4"
          >
            <div className="flex items-center gap-3">
              <div className="size-3 rounded-full bg-[var(--cp-border)]" />
              <div className="h-3 w-32 rounded-full bg-[var(--cp-surface-muted)]" />
              <div className="h-3 w-16 rounded-full bg-[var(--cp-surface-muted)]" />
            </div>
            <div className="mt-3 flex gap-2">
              <div className="h-5 w-14 rounded-full bg-[var(--cp-surface-muted)]" />
              <div className="h-5 w-14 rounded-full bg-[var(--cp-surface-muted)]" />
            </div>
          </div>
        ))}
      </div>
    )
  }

  if (!run) {
    return (
      <div className="rounded-2xl border border-[var(--cp-border)] bg-white px-6 py-12 text-center">
        <Icon name="loop" className="mx-auto size-8 text-[var(--cp-muted)]" />
        <p className="mt-3 text-sm font-medium text-[var(--cp-ink)]">No run selected</p>
        <p className="mt-1 text-xs text-[var(--cp-muted)]">
          This agent has no loop runs to display.
        </p>
      </div>
    )
  }

  const recentTodos = todos.slice(0, 5)
  const openTodos = todos.filter((t) => t.status === 'open')
  const doneTodos = todos.filter((t) => t.status === 'done')

  return (
    <div className="space-y-6">
      {/* Run summary strip */}
      <div className="flex flex-wrap items-center gap-3 rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-3">
        <StatusPill status={run.status} />
        <span className="text-xs text-[var(--cp-muted)]">
          Step {run.current_step_index + 1} / {run.summary.step_count}
        </span>
        <div className="h-1.5 flex-1 rounded-full bg-[var(--cp-border)]">
          <div
            className="h-full rounded-full bg-[var(--cp-primary)] transition-all"
            style={{
              width: `${run.summary.step_count > 0 ? ((run.current_step_index + 1) / run.summary.step_count) * 100 : 0}%`,
            }}
          />
        </div>
      </div>

      {/* Step Timeline */}
      <div>
        <h3 className="mb-3 text-xs font-semibold uppercase tracking-wide text-[var(--cp-muted)]">
          Step Timeline
        </h3>
        <div className="space-y-2">
          {steps.map((step) => (
            <StepCard
              key={step.step_id}
              step={step}
              isCurrent={step.step_index === run.current_step_index}
            />
          ))}
        </div>
        {steps.length === 0 && (
          <div className="rounded-xl border border-[var(--cp-border)] bg-white px-4 py-6 text-center text-sm text-[var(--cp-muted)]">
            No steps in this run.
          </div>
        )}
      </div>

      {/* Bottom summary panels */}
      <div className="grid gap-4 lg:grid-cols-3">
        {/* Recent Todos */}
        <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
          <div className="mb-3 flex items-center justify-between">
            <h4 className="text-xs font-semibold uppercase tracking-wide text-[var(--cp-muted)]">
              Todos
            </h4>
            <button
              type="button"
              onClick={() => setActiveTab('todos')}
              className="text-xs font-medium text-[var(--cp-primary)] hover:underline"
            >
              View all
            </button>
          </div>
          <div className="mb-2 flex gap-3 text-xs">
            <span className="text-sky-600">{openTodos.length} open</span>
            <span className="text-emerald-600">{doneTodos.length} done</span>
          </div>
          {recentTodos.length > 0 ? (
            <div className="space-y-1.5">
              {recentTodos.map((todo) => (
                <button
                  key={todo.todo_id}
                  type="button"
                  onClick={() => openInspector({ kind: 'todo', data: todo })}
                  className="flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-xs transition hover:bg-[var(--cp-surface-muted)]"
                >
                  <span
                    className={`size-2 flex-none rounded-full ${
                      todo.status === 'done' ? 'bg-emerald-500' : 'bg-sky-500'
                    }`}
                  />
                  <span
                    className={`flex-1 truncate ${
                      todo.status === 'done'
                        ? 'text-[var(--cp-muted)] line-through'
                        : 'text-[var(--cp-ink)]'
                    }`}
                  >
                    {todo.title}
                  </span>
                </button>
              ))}
            </div>
          ) : (
            <p className="text-xs text-[var(--cp-muted)]">No todos yet.</p>
          )}
        </div>

        {/* Active Sub-Agents */}
        <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
          <div className="mb-3 flex items-center justify-between">
            <h4 className="text-xs font-semibold uppercase tracking-wide text-[var(--cp-muted)]">
              Sub-Agents
            </h4>
            {subAgents.length > 0 && (
              <button
                type="button"
                onClick={() => setActiveTab('sub-agents')}
                className="text-xs font-medium text-[var(--cp-primary)] hover:underline"
              >
                View all
              </button>
            )}
          </div>
          {subAgents.length > 0 ? (
            <div className="space-y-2">
              {subAgents.map((sa) => (
                <button
                  key={sa.agent_id}
                  type="button"
                  onClick={() => openInspector({ kind: 'sub-agent', data: sa })}
                  className="flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-xs transition hover:bg-[var(--cp-surface-muted)]"
                >
                  <StatusPill status={sa.status} />
                  <span className="flex-1 truncate font-medium text-[var(--cp-ink)]">
                    {sa.agent_name}
                  </span>
                </button>
              ))}
            </div>
          ) : (
            <p className="text-xs text-[var(--cp-muted)]">No sub-agents in this run.</p>
          )}
        </div>

        {/* Run Summary */}
        <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4">
          <h4 className="mb-3 text-xs font-semibold uppercase tracking-wide text-[var(--cp-muted)]">
            Run Summary
          </h4>
          <div className="space-y-2 text-xs">
            <div className="flex justify-between">
              <span className="text-[var(--cp-muted)]">Trigger</span>
              <span className="max-w-[60%] truncate text-right text-[var(--cp-ink)]">
                {run.trigger_event}
              </span>
            </div>
            <div className="flex justify-between">
              <span className="text-[var(--cp-muted)]">Steps</span>
              <span className="text-[var(--cp-ink)]">{run.summary.step_count}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-[var(--cp-muted)]">Tasks</span>
              <span className="text-[var(--cp-ink)]">{run.summary.task_count}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-[var(--cp-muted)]">Logs</span>
              <span className="text-[var(--cp-ink)]">{run.summary.log_count}</span>
            </div>
            {run.status !== 'running' && run.ended_at && (
              <div className="flex justify-between">
                <span className="text-[var(--cp-muted)]">Ended</span>
                <span className="text-[var(--cp-ink)]">
                  {new Date(run.ended_at).toLocaleTimeString()}
                </span>
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  )
}

export default OverviewTab
