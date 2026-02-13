import StatusPill from './StatusPill'

type TaskRowProps = {
  task: WsTask
  onClick: () => void
}

const formatDuration = (seconds?: number): string => {
  if (seconds == null) return '—'
  return `${seconds}s`
}

const TaskRow = ({ task, onClick }: TaskRowProps) => {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-left transition hover:bg-[var(--cp-surface-muted)]"
    >
      {/* Status */}
      <StatusPill status={task.status} />

      {/* Main info */}
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2 text-[10px]">
          <span className="font-mono text-[var(--cp-muted)]">{task.task_id.slice(0, 12)}</span>
          <span className="text-[var(--cp-muted)]">{task.model}</span>
          <span className="cp-pill border border-[var(--cp-border)] bg-white text-[var(--cp-muted)]">
            {task.step_id.slice(-6)}
          </span>
        </div>
        <p className="mt-0.5 truncate text-xs text-[var(--cp-ink)]">{task.prompt_preview}</p>
        {task.result_preview && (
          <p className="truncate text-xs text-[var(--cp-muted)]">{task.result_preview}</p>
        )}
      </div>

      {/* Tokens + Duration */}
      <div className="flex flex-none flex-col items-end text-[10px] text-[var(--cp-muted)]">
        <span>{formatDuration(task.duration)}</span>
        {task.tokens_in != null && task.tokens_out != null && (
          <span>
            {task.tokens_in}/{task.tokens_out} tok
          </span>
        )}
      </div>
    </button>
  )
}

export default TaskRow
