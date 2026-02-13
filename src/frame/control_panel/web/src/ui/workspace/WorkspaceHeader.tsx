import Icon from '../icons'
import { useCurrentRun, useSelectedAgent, useWorkspace } from './WorkspaceContext'
import StatusPill from './components/StatusPill'
import CountBadge from './components/CountBadge'
import RunSelector from './components/RunSelector'

const formatDuration = (seconds: number): string => {
  if (seconds < 60) return `${seconds}s`
  const m = Math.floor(seconds / 60)
  const s = seconds % 60
  if (m < 60) return `${m}m ${s}s`
  const h = Math.floor(m / 60)
  return `${h}h ${m % 60}m`
}

const formatTime = (iso: string): string => {
  if (!iso) return '—'
  const d = new Date(iso)
  return d.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit', second: '2-digit' })
}

const WorkspaceHeader = () => {
  const { liveMode, toggleLiveMode, runs, selectedRunId, setSelectedRunId, navigateToWorkLog } =
    useWorkspace()
  const agent = useSelectedAgent()
  const run = useCurrentRun()

  if (!agent) {
    return (
      <header className="cp-panel px-6 py-5">
        <div className="flex items-center gap-3 text-sm text-[var(--cp-muted)]">
          <Icon name="agent" className="size-4" />
          Select an agent to view its workspace
        </div>
      </header>
    )
  }

  return (
    <header className="cp-panel px-6 py-5">
      <div className="flex flex-wrap items-start justify-between gap-4">
        {/* Left: Agent info */}
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-3">
            <h1 className="text-xl font-semibold text-[var(--cp-ink)]">{agent.agent_name}</h1>
            <StatusPill status={agent.status} />
            <span className="cp-pill border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] text-[var(--cp-muted)]">
              {agent.agent_type === 'main' ? 'Main' : 'Sub'}
            </span>
            {agent.parent_agent_id && (
              <span className="text-xs text-[var(--cp-muted)]">
                Parent: {agent.parent_agent_id}
              </span>
            )}
          </div>

          {/* Run info */}
          <div className="mt-3 flex flex-wrap items-center gap-3 text-sm">
            <RunSelector runs={runs} selectedRunId={selectedRunId} onSelect={setSelectedRunId} />
            {run && (
              <>
                <span className="text-xs text-[var(--cp-muted)]">
                  Trigger: <span className="text-[var(--cp-ink)]">{run.trigger_event}</span>
                </span>
                <span className="text-xs text-[var(--cp-muted)]">
                  Started: <span className="text-[var(--cp-ink)]">{formatTime(run.started_at)}</span>
                </span>
                {run.duration != null && (
                  <span className="text-xs text-[var(--cp-muted)]">
                    Duration:{' '}
                    <span className="text-[var(--cp-ink)]">{formatDuration(run.duration)}</span>
                  </span>
                )}
              </>
            )}
          </div>
        </div>

        {/* Right: KPI chips + live toggle */}
        <div className="flex items-center gap-2">
          {run && (
            <div className="flex flex-wrap items-center gap-1.5">
              <CountBadge
                icon="activity"
                label="Steps"
                count={run.summary.step_count}
                onClick={() => {}}
              />
              <CountBadge
                icon="spark"
                label="Tasks"
                count={run.summary.task_count}
                onClick={() => {}}
              />
              <CountBadge
                icon="chart"
                label="Logs"
                count={run.summary.log_count}
                onClick={() => navigateToWorkLog({})}
              />
              <CountBadge
                icon="todo"
                label="Todos"
                count={run.summary.todo_count}
                onClick={() => {}}
              />
              <CountBadge
                icon="branch"
                label="Sub-Agents"
                count={run.summary.sub_agent_count}
                onClick={() => {}}
              />
            </div>
          )}
          <button
            type="button"
            onClick={toggleLiveMode}
            className={`cp-pill ml-2 border transition ${
              liveMode
                ? 'border-transparent bg-[var(--cp-primary)] text-white shadow'
                : 'border-[var(--cp-border)] bg-[var(--cp-surface-muted)] text-[var(--cp-muted)]'
            }`}
          >
            <Icon name={liveMode ? 'play' : 'pause'} className="size-3" />
            {liveMode ? 'Live' : 'Paused'}
          </button>
        </div>
      </div>
    </header>
  )
}

export default WorkspaceHeader
