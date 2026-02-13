import { useCallback, useEffect, useState } from 'react'

import { fetchTodos } from '@/api/workspace'
import StatusPill from '../components/StatusPill'

type SubAgentDetailProps = {
  agent: WsAgent
}

const formatTime = (iso: string): string => {
  if (!iso) return '—'
  const d = new Date(iso)
  return d.toLocaleString(undefined, {
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  })
}

const SubAgentDetail = ({ agent }: SubAgentDetailProps) => {
  const [todos, setTodos] = useState<WsTodo[]>([])

  const loadTodos = useCallback(async () => {
    const { data } = await fetchTodos(agent.agent_id)
    if (data) setTodos(data)
  }, [agent.agent_id])

  useEffect(() => {
    loadTodos()
  }, [loadTodos])

  const openTodos = todos.filter((t) => t.status === 'open')
  const doneTodos = todos.filter((t) => t.status === 'done')

  return (
    <div className="space-y-4">
      {/* Header */}
      <div>
        <div className="flex items-center gap-2">
          <span className="text-sm font-semibold text-[var(--cp-ink)]">{agent.agent_name}</span>
          <StatusPill status={agent.status} />
        </div>
        <p className="mt-1 text-xs text-[var(--cp-muted)]">
          Type: {agent.agent_type === 'main' ? 'Main Agent' : 'Sub-Agent'}
        </p>
        {agent.parent_agent_id && (
          <p className="text-xs text-[var(--cp-muted)]">Parent: {agent.parent_agent_id}</p>
        )}
      </div>

      {/* Metadata */}
      <div className="grid grid-cols-2 gap-2 text-xs">
        <div className="rounded-lg bg-[var(--cp-surface-muted)] px-3 py-2">
          <p className="text-[var(--cp-muted)]">Last Active</p>
          <p className="font-medium text-[var(--cp-ink)]">{formatTime(agent.last_active_at)}</p>
        </div>
        <div className="rounded-lg bg-[var(--cp-surface-muted)] px-3 py-2">
          <p className="text-[var(--cp-muted)]">Current Run</p>
          <p className="font-medium text-[var(--cp-ink)]">{agent.current_run_id ?? '—'}</p>
        </div>
      </div>

      {/* Todo Overview */}
      <div>
        <h4 className="mb-2 text-xs font-semibold uppercase tracking-wide text-[var(--cp-muted)]">
          Todos ({openTodos.length} open / {doneTodos.length} done)
        </h4>
        {todos.length === 0 ? (
          <p className="text-xs text-[var(--cp-muted)]">No todos for this agent.</p>
        ) : (
          <div className="space-y-1.5">
            {openTodos.map((todo) => (
              <div
                key={todo.todo_id}
                className="flex items-center gap-2 rounded-lg border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2 text-xs"
              >
                <span className="size-2 rounded-full bg-sky-500" />
                <span className="flex-1 text-[var(--cp-ink)]">{todo.title}</span>
              </div>
            ))}
            {doneTodos.map((todo) => (
              <div
                key={todo.todo_id}
                className="flex items-center gap-2 rounded-lg border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2 text-xs"
              >
                <span className="size-2 rounded-full bg-emerald-500" />
                <span className="flex-1 text-[var(--cp-muted)] line-through">{todo.title}</span>
              </div>
            ))}
          </div>
        )}
      </div>

      <div className="text-[10px] text-[var(--cp-muted)]">ID: {agent.agent_id}</div>
    </div>
  )
}

export default SubAgentDetail
