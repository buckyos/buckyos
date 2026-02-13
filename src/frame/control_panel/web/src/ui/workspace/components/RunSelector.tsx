import StatusPill from './StatusPill'

type RunSelectorProps = {
  runs: LoopRun[]
  selectedRunId: string | null
  onSelect: (runId: string) => void
}

const formatRunLabel = (run: LoopRun): string => {
  const date = new Date(run.started_at)
  const time = date.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' })
  return `${run.run_id.slice(0, 12)} · ${time}`
}

const RunSelector = ({ runs, selectedRunId, onSelect }: RunSelectorProps) => {
  if (!runs.length) {
    return (
      <span className="rounded-full border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-1.5 text-xs text-[var(--cp-muted)]">
        No runs
      </span>
    )
  }

  const selectedRun = runs.find((r) => r.run_id === selectedRunId)

  return (
    <div className="flex items-center gap-2">
      <select
        value={selectedRunId ?? ''}
        onChange={(e) => onSelect(e.target.value)}
        className="rounded-full border border-[var(--cp-border)] bg-white px-3 py-1.5 text-xs font-medium text-[var(--cp-ink)] focus:outline-none focus:ring-2 focus:ring-[var(--cp-primary-soft)]"
      >
        {runs.map((run) => (
          <option key={run.run_id} value={run.run_id}>
            {formatRunLabel(run)}
          </option>
        ))}
      </select>
      {selectedRun && <StatusPill status={selectedRun.status} />}
    </div>
  )
}

export default RunSelector
