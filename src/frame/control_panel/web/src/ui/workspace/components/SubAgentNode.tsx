import Icon from '../../icons'
import StatusPill from './StatusPill'

type SubAgentNodeProps = {
  agent: WsAgent
  onClick: () => void
  onOpenWorkspace?: () => void
}

const formatTime = (iso: string): string => {
  if (!iso) return '—'
  return new Date(iso).toLocaleString(undefined, {
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  })
}

const SubAgentNode = ({ agent, onClick, onOpenWorkspace }: SubAgentNodeProps) => {
  return (
    <div
      className="cursor-pointer rounded-2xl border border-[var(--cp-border)] bg-white p-4 transition hover:shadow-md"
      onClick={onClick}
    >
      <div className="flex items-start justify-between">
        <div className="flex items-center gap-2">
          <span className="inline-flex size-8 items-center justify-center rounded-xl bg-[var(--cp-surface-muted)]">
            <Icon name="agent" className="size-4 text-[var(--cp-muted)]" />
          </span>
          <div>
            <p className="text-sm font-medium text-[var(--cp-ink)]">{agent.agent_name}</p>
            <p className="text-[10px] text-[var(--cp-muted)]">{agent.agent_id}</p>
          </div>
        </div>
        <StatusPill status={agent.status} />
      </div>

      <div className="mt-3 grid grid-cols-2 gap-2 text-xs">
        <div>
          <p className="text-[var(--cp-muted)]">Last Active</p>
          <p className="font-medium text-[var(--cp-ink)]">{formatTime(agent.last_active_at)}</p>
        </div>
        <div>
          <p className="text-[var(--cp-muted)]">Current Run</p>
          <p className="font-medium text-[var(--cp-ink)]">
            {agent.current_run_id?.slice(0, 12) ?? '—'}
          </p>
        </div>
      </div>

      {onOpenWorkspace && (
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation()
            onOpenWorkspace()
          }}
          className="mt-3 flex w-full items-center justify-center gap-1.5 rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-1.5 text-xs font-medium text-[var(--cp-primary)] transition hover:bg-[var(--cp-primary-soft)]"
        >
          <Icon name="external" className="size-3" />
          Open Workspace
        </button>
      )}
    </div>
  )
}

export default SubAgentNode
