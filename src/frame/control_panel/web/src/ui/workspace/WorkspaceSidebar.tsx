import Icon from '../icons'
import { useWorkspace } from './WorkspaceContext'

const statusDotColor: Record<AgentStatus, string> = {
  running: 'bg-[var(--cp-primary)]',
  idle: 'bg-slate-400',
  sleeping: 'bg-amber-400',
  error: 'bg-[var(--cp-danger)]',
  offline: 'bg-slate-300',
}

const statusLabel: Record<AgentStatus, string> = {
  running: 'Running',
  idle: 'Idle',
  sleeping: 'Sleeping',
  error: 'Error',
  offline: 'Offline',
}

const WorkspaceSidebar = () => {
  const { agents, selectedAgentId, setSelectedAgentId, agentsLoading } = useWorkspace()

  const mainAgents = agents.filter((a) => a.agent_type === 'main')
  const subAgents = agents.filter((a) => a.agent_type === 'sub')

  if (agentsLoading) {
    return (
      <aside className="flex h-full flex-col border-r border-[var(--cp-border)] bg-white/85 p-4 backdrop-blur">
        <div className="mb-6 flex items-center gap-3 text-lg font-semibold text-[var(--cp-ink)]">
          <span className="inline-flex size-9 items-center justify-center rounded-2xl bg-[var(--cp-primary)] text-white">
            <Icon name="agent" className="size-4" />
          </span>
          <span>Workspace</span>
        </div>
        <div className="space-y-3">
          {Array.from({ length: 4 }).map((_, i) => (
            <div key={`skeleton-${i}`} className="flex animate-pulse items-center gap-3 rounded-xl px-3 py-2.5">
              <div className="size-2.5 rounded-full bg-[var(--cp-border)]" />
              <div className="h-3 w-24 rounded-full bg-[var(--cp-surface-muted)]" />
            </div>
          ))}
        </div>
      </aside>
    )
  }

  const renderAgentGroup = (label: string, list: WsAgent[]) => {
    if (!list.length) return null
    return (
      <div className="space-y-1">
        <p className="px-3 text-[10px] font-semibold uppercase tracking-wider text-[var(--cp-muted)]">
          {label}
        </p>
        {list.map((agent) => {
          const isSelected = agent.agent_id === selectedAgentId
          return (
            <button
              key={agent.agent_id}
              type="button"
              onClick={() => setSelectedAgentId(agent.agent_id)}
              className={`flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-left text-sm transition ${
                isSelected
                  ? 'bg-[var(--cp-primary)] text-white shadow-lg shadow-emerald-200'
                  : 'text-[var(--cp-muted)] hover:bg-[var(--cp-surface-muted)] hover:text-[var(--cp-ink)]'
              }`}
            >
              <span
                className={`inline-flex size-2.5 flex-none rounded-full ${
                  isSelected ? 'bg-white' : statusDotColor[agent.status]
                }`}
                title={statusLabel[agent.status]}
              />
              <span className="flex-1 truncate font-medium">{agent.agent_name}</span>
              {agent.status === 'running' && !isSelected && (
                <span className="size-1.5 animate-pulse rounded-full bg-[var(--cp-primary)]" />
              )}
            </button>
          )
        })}
      </div>
    )
  }

  return (
    <aside className="flex h-full flex-col overflow-y-auto border-r border-[var(--cp-border)] bg-white/85 p-4 backdrop-blur">
      <div className="mb-6 flex items-center gap-3 text-lg font-semibold text-[var(--cp-ink)]">
        <span className="inline-flex size-9 items-center justify-center rounded-2xl bg-[var(--cp-primary)] text-white shadow-lg shadow-emerald-200">
          <Icon name="agent" className="size-4" />
        </span>
        <div className="flex flex-col leading-tight">
          <span className="font-semibold">Agent</span>
          <span className="text-xs font-medium text-[var(--cp-muted)]">Workspace</span>
        </div>
      </div>

      <nav className="space-y-5 text-sm">
        {renderAgentGroup('Main Agents', mainAgents)}
        {renderAgentGroup('Sub-Agents', subAgents)}
      </nav>

      {!agents.length && (
        <div className="mt-8 rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-6 text-center text-sm text-[var(--cp-muted)]">
          <p className="text-[var(--cp-ink)]">No agents available</p>
          <p className="mt-1 text-xs">Configure agents to get started.</p>
        </div>
      )}
    </aside>
  )
}

export default WorkspaceSidebar
