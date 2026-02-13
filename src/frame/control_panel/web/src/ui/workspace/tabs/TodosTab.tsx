import { useCallback, useEffect, useMemo, useState } from 'react'

import { fetchTodos } from '@/api/workspace'
import { useWorkspace } from '../WorkspaceContext'
import TodoItem from '../components/TodoItem'

type FilterStatus = 'all' | 'open' | 'done'

const TodosTab = () => {
  const { agents, selectedAgentId, openInspector } = useWorkspace()

  const [todosByAgent, setTodosByAgent] = useState<Record<string, WsTodo[]>>({})
  const [loading, setLoading] = useState(true)
  const [statusFilter, setStatusFilter] = useState<FilterStatus>('all')
  const [viewAgentId, setViewAgentId] = useState<string | null>(null)

  // Determine which agents to show todos for
  const relevantAgents = useMemo(() => {
    if (!selectedAgentId) return []
    const main = agents.find((a) => a.agent_id === selectedAgentId)
    if (!main) return []
    const subs = agents.filter((a) => a.parent_agent_id === selectedAgentId)
    return [main, ...subs]
  }, [agents, selectedAgentId])

  const loadData = useCallback(async () => {
    setLoading(true)
    const results: Record<string, WsTodo[]> = {}
    await Promise.all(
      relevantAgents.map(async (agent) => {
        const { data } = await fetchTodos(agent.agent_id)
        results[agent.agent_id] = data ?? []
      }),
    )
    setTodosByAgent(results)
    setLoading(false)
  }, [relevantAgents])

  useEffect(() => {
    loadData()
  }, [loadData])

  // Default view agent
  useEffect(() => {
    if (!viewAgentId && relevantAgents.length > 0) {
      setViewAgentId(relevantAgents[0].agent_id)
    }
  }, [viewAgentId, relevantAgents])

  const currentTodos = useMemo(() => {
    const all = viewAgentId ? (todosByAgent[viewAgentId] ?? []) : []
    if (statusFilter === 'all') return all
    return all.filter((t) => t.status === statusFilter)
  }, [todosByAgent, viewAgentId, statusFilter])

  const agentCounts = useMemo(() => {
    const counts: Record<string, { open: number; done: number }> = {}
    for (const [agentId, todos] of Object.entries(todosByAgent)) {
      counts[agentId] = {
        open: todos.filter((t) => t.status === 'open').length,
        done: todos.filter((t) => t.status === 'done').length,
      }
    }
    return counts
  }, [todosByAgent])

  return (
    <div className="flex gap-4">
      {/* Left: Agent selector */}
      <div className="w-48 flex-none space-y-1">
        <p className="px-2 pb-1 text-[10px] font-semibold uppercase tracking-wider text-[var(--cp-muted)]">
          Agents
        </p>
        {relevantAgents.map((agent) => {
          const counts = agentCounts[agent.agent_id]
          const isActive = viewAgentId === agent.agent_id
          return (
            <button
              key={agent.agent_id}
              type="button"
              onClick={() => setViewAgentId(agent.agent_id)}
              className={`flex w-full items-center justify-between rounded-xl px-3 py-2 text-left text-xs transition ${
                isActive
                  ? 'bg-[var(--cp-primary)] text-white shadow'
                  : 'text-[var(--cp-muted)] hover:bg-[var(--cp-surface-muted)]'
              }`}
            >
              <span className="truncate font-medium">{agent.agent_name}</span>
              {counts && (
                <span className={`text-[10px] ${isActive ? 'text-white/80' : ''}`}>
                  {counts.open}/{counts.done}
                </span>
              )}
            </button>
          )
        })}
      </div>

      {/* Right: Todo list */}
      <div className="min-w-0 flex-1">
        {/* Status filter tabs */}
        <div className="mb-3 flex gap-1">
          {(['all', 'open', 'done'] as const).map((value) => (
            <button
              key={value}
              type="button"
              onClick={() => setStatusFilter(value)}
              className={`cp-pill border transition ${
                statusFilter === value
                  ? 'border-transparent bg-[var(--cp-primary)] text-white shadow'
                  : 'border-[var(--cp-border)] bg-[var(--cp-surface-muted)] text-[var(--cp-muted)]'
              }`}
            >
              {value === 'all' ? 'All' : value === 'open' ? 'Open' : 'Done'}
            </button>
          ))}
        </div>

        {/* List */}
        <div className="rounded-2xl border border-[var(--cp-border)] bg-white">
          {loading ? (
            <div className="space-y-1 p-2">
              {Array.from({ length: 4 }).map((_, i) => (
                <div key={`todo-skeleton-${i}`} className="flex animate-pulse items-center gap-3 px-3 py-2.5">
                  <div className="size-5 rounded-md bg-[var(--cp-surface-muted)]" />
                  <div className="flex-1 space-y-1.5">
                    <div className="h-2.5 w-44 rounded-full bg-[var(--cp-surface-muted)]" />
                    <div className="h-2 w-28 rounded-full bg-[var(--cp-surface-muted)]" />
                  </div>
                </div>
              ))}
            </div>
          ) : currentTodos.length > 0 ? (
            <div className="divide-y divide-[var(--cp-border)]/30 p-1">
              {currentTodos.map((todo) => (
                <TodoItem
                  key={todo.todo_id}
                  todo={todo}
                  onClick={() => openInspector({ kind: 'todo', data: todo })}
                />
              ))}
            </div>
          ) : (
            <div className="px-4 py-8 text-center text-sm text-[var(--cp-muted)]">
              <p className="text-[var(--cp-ink)]">
                {statusFilter === 'all'
                  ? 'No todos for this agent.'
                  : `No ${statusFilter} todos.`}
              </p>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}

export default TodosTab
