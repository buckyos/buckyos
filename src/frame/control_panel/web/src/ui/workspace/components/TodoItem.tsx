import StatusPill from './StatusPill'

type TodoItemProps = {
  todo: WsTodo
  onClick: () => void
}

const formatTime = (iso?: string): string => {
  if (!iso) return '—'
  return new Date(iso).toLocaleString(undefined, {
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  })
}

const TodoItem = ({ todo, onClick }: TodoItemProps) => {
  const isDone = todo.status === 'done'

  return (
    <button
      type="button"
      onClick={onClick}
      className="flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-left transition hover:bg-[var(--cp-surface-muted)]"
    >
      {/* Checkbox indicator */}
      <span
        className={`inline-flex size-5 flex-none items-center justify-center rounded-md border ${
          isDone
            ? 'border-emerald-300 bg-emerald-100 text-emerald-600'
            : 'border-[var(--cp-border)] bg-white'
        }`}
      >
        {isDone && (
          <svg viewBox="0 0 12 12" className="size-3" fill="none" stroke="currentColor" strokeWidth="2">
            <path d="M2.5 6l2.5 2.5 4.5-4.5" />
          </svg>
        )}
      </span>

      {/* Content */}
      <div className="min-w-0 flex-1">
        <p
          className={`text-xs font-medium ${
            isDone ? 'text-[var(--cp-muted)] line-through' : 'text-[var(--cp-ink)]'
          }`}
        >
          {todo.title}
        </p>
        <div className="mt-0.5 flex flex-wrap items-center gap-2 text-[10px] text-[var(--cp-muted)]">
          <span>Created: {formatTime(todo.created_at)}</span>
          {isDone && <span>Done: {formatTime(todo.completed_at)}</span>}
          {todo.created_in_step_id && (
            <span className="cp-pill border border-[var(--cp-border)] bg-white text-[var(--cp-muted)]">
              {todo.created_in_step_id.slice(-6)}
            </span>
          )}
        </div>
      </div>

      {/* Status */}
      <StatusPill status={todo.status} />
    </button>
  )
}

export default TodoItem
