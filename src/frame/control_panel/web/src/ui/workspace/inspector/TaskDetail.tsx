import StatusPill from '../components/StatusPill'
import JsonViewer from '../components/JsonViewer'

type TaskDetailProps = {
  task: WsTask
}

const formatDuration = (seconds?: number): string => {
  if (seconds == null) return '—'
  return `${seconds}s`
}

const TaskDetail = ({ task }: TaskDetailProps) => {
  return (
    <div className="space-y-4">
      {/* Header */}
      <div>
        <div className="flex items-center gap-2">
          <span className="text-sm font-semibold text-[var(--cp-ink)]">Task</span>
          <StatusPill status={task.status} />
        </div>
        <p className="mt-1 text-xs text-[var(--cp-muted)]">Step: {task.step_id}</p>
        {task.behavior_id && (
          <p className="text-xs text-[var(--cp-muted)]">Behavior: {task.behavior_id}</p>
        )}
      </div>

      {/* Metadata */}
      <div className="grid grid-cols-2 gap-2 text-xs">
        <div className="rounded-lg bg-[var(--cp-surface-muted)] px-3 py-2">
          <p className="text-[var(--cp-muted)]">Model</p>
          <p className="font-medium text-[var(--cp-ink)]">{task.model}</p>
        </div>
        <div className="rounded-lg bg-[var(--cp-surface-muted)] px-3 py-2">
          <p className="text-[var(--cp-muted)]">Duration</p>
          <p className="font-medium text-[var(--cp-ink)]">{formatDuration(task.duration)}</p>
        </div>
        {task.tokens_in != null && (
          <div className="rounded-lg bg-[var(--cp-surface-muted)] px-3 py-2">
            <p className="text-[var(--cp-muted)]">Tokens In</p>
            <p className="font-medium text-[var(--cp-ink)]">{task.tokens_in.toLocaleString()}</p>
          </div>
        )}
        {task.tokens_out != null && (
          <div className="rounded-lg bg-[var(--cp-surface-muted)] px-3 py-2">
            <p className="text-[var(--cp-muted)]">Tokens Out</p>
            <p className="font-medium text-[var(--cp-ink)]">{task.tokens_out.toLocaleString()}</p>
          </div>
        )}
      </div>

      {/* Prompt */}
      {task.prompt_preview && (
        <div>
          <h4 className="mb-1.5 text-xs font-semibold uppercase tracking-wide text-[var(--cp-muted)]">
            Prompt
          </h4>
          <div className="rounded-lg border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2 text-xs leading-5 text-[var(--cp-ink)]">
            {task.prompt_preview}
          </div>
        </div>
      )}

      {/* Response */}
      {task.result_preview && (
        <div>
          <h4 className="mb-1.5 text-xs font-semibold uppercase tracking-wide text-[var(--cp-muted)]">
            Response
          </h4>
          <div className="rounded-lg border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2 text-xs leading-5 text-[var(--cp-ink)]">
            {task.result_preview}
          </div>
        </div>
      )}

      {/* Raw I/O */}
      {task.raw_input && (
        <JsonViewer label="Raw Input" data={JSON.parse(task.raw_input)} />
      )}
      {task.raw_output && (
        <JsonViewer label="Raw Output" data={JSON.parse(task.raw_output)} />
      )}

      <div className="text-[10px] text-[var(--cp-muted)]">ID: {task.task_id}</div>
    </div>
  )
}

export default TaskDetail
