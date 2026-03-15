import StatusPill from '../components/StatusPill'

type TodoDetailProps = {
  todo: WsTodo
}

const formatTime = (iso?: string): string => {
  if (!iso) return '—'
  const d = new Date(iso)
  return d.toLocaleString(undefined, {
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  })
}

const TodoDetail = ({ todo }: TodoDetailProps) => {
  return (
    <div className="space-y-4">
      {/* Header */}
      <div>
        <div className="flex items-center gap-2">
          <span className="text-sm font-semibold text-[var(--cp-ink)]">{todo.title}</span>
          <StatusPill status={todo.status} />
        </div>
      </div>

      {/* Description */}
      {todo.description && (
        <div className="rounded-lg border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2 text-xs leading-5 text-[var(--cp-ink)]">
          {todo.description}
        </div>
      )}

      {/* Timeline */}
      <div>
        <h4 className="mb-2 text-xs font-semibold uppercase tracking-wide text-[var(--cp-muted)]">
          Timeline
        </h4>
        <div className="space-y-2">
          <div className="flex items-start gap-3">
            <div className="mt-1 flex flex-col items-center">
              <span className="size-2.5 rounded-full bg-sky-500" />
              {todo.status === 'done' && (
                <span className="h-6 w-px bg-[var(--cp-border)]" />
              )}
            </div>
            <div className="text-xs">
              <p className="font-medium text-[var(--cp-ink)]">Created</p>
              <p className="text-[var(--cp-muted)]">{formatTime(todo.created_at)}</p>
              {todo.created_in_step_id && (
                <p className="text-[var(--cp-muted)]">Step: {todo.created_in_step_id}</p>
              )}
            </div>
          </div>
          {todo.status === 'done' && (
            <div className="flex items-start gap-3">
              <div className="mt-1">
                <span className="size-2.5 rounded-full bg-emerald-500 block" />
              </div>
              <div className="text-xs">
                <p className="font-medium text-[var(--cp-ink)]">Completed</p>
                <p className="text-[var(--cp-muted)]">{formatTime(todo.completed_at)}</p>
                {todo.completed_in_step_id && (
                  <p className="text-[var(--cp-muted)]">Step: {todo.completed_in_step_id}</p>
                )}
              </div>
            </div>
          )}
        </div>
      </div>

      {/* Metadata */}
      <div className="grid grid-cols-1 gap-2 text-xs">
        <div className="rounded-lg bg-[var(--cp-surface-muted)] px-3 py-2">
          <p className="text-[var(--cp-muted)]">Agent</p>
          <p className="font-medium text-[var(--cp-ink)]">{todo.agent_id}</p>
        </div>
      </div>

      <div className="text-[10px] text-[var(--cp-muted)]">ID: {todo.todo_id}</div>
    </div>
  )
}

export default TodoDetail
