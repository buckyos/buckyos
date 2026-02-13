import { useCallback, useEffect, useMemo, useState } from 'react'

import { fetchWorkLogs, fetchSteps } from '@/api/workspace'
import { useCurrentRun, useWorkspace } from '../WorkspaceContext'
import FilterBar from '../components/FilterBar'
import WorkLogRow from '../components/WorkLogRow'

const typeOptions = [
  { value: 'message_sent', label: 'Message Sent' },
  { value: 'message_reply', label: 'Message Reply' },
  { value: 'function_call', label: 'Function Call' },
  { value: 'action', label: 'Action' },
  { value: 'sub_agent_created', label: 'Sub-Agent Created' },
  { value: 'sub_agent_sleep', label: 'Sub-Agent Sleep' },
  { value: 'sub_agent_wake', label: 'Sub-Agent Wake' },
  { value: 'sub_agent_destroyed', label: 'Sub-Agent Destroyed' },
]

const statusOptions = [
  { value: 'info', label: 'Info' },
  { value: 'success', label: 'Success' },
  { value: 'failed', label: 'Failed' },
  { value: 'partial', label: 'Partial' },
]

const WorkLogTab = () => {
  const { selectedRunId, liveMode, openInspector, pendingWorkLogFilters, clearPendingWorkLogFilters } =
    useWorkspace()
  const run = useCurrentRun()

  const [logs, setLogs] = useState<WsWorkLog[]>([])
  const [steps, setSteps] = useState<WsStep[]>([])
  const [loading, setLoading] = useState(true)
  const [typeFilter, setTypeFilter] = useState('')
  const [statusFilter, setStatusFilter] = useState('')
  const [stepFilter, setStepFilter] = useState('')
  const [keyword, setKeyword] = useState('')

  // Apply pending filters from cross-tab navigation
  useEffect(() => {
    if (pendingWorkLogFilters) {
      if (pendingWorkLogFilters.type) setTypeFilter(pendingWorkLogFilters.type)
      if (pendingWorkLogFilters.status) setStatusFilter(pendingWorkLogFilters.status)
      if (pendingWorkLogFilters.stepId) setStepFilter(pendingWorkLogFilters.stepId)
      if (pendingWorkLogFilters.keyword) setKeyword(pendingWorkLogFilters.keyword)
      clearPendingWorkLogFilters()
    }
  }, [pendingWorkLogFilters, clearPendingWorkLogFilters])

  const loadData = useCallback(
    async (silent = false) => {
      if (!selectedRunId) return
      if (!silent) setLoading(true)
      const [logRes, stepRes] = await Promise.all([
        fetchWorkLogs(selectedRunId, {
          type: (typeFilter || undefined) as WorkLogType | undefined,
          status: (statusFilter || undefined) as WorkLogStatus | undefined,
          stepId: stepFilter || undefined,
          keyword: keyword || undefined,
        }),
        fetchSteps(selectedRunId),
      ])
      if (logRes.data) setLogs(logRes.data)
      if (stepRes.data) setSteps(stepRes.data)
      setLoading(false)
    },
    [selectedRunId, typeFilter, statusFilter, stepFilter, keyword],
  )

  useEffect(() => {
    loadData()
  }, [loadData])

  useEffect(() => {
    if (!liveMode || !run || run.status !== 'running') return
    const id = window.setInterval(() => loadData(true), 5000)
    return () => window.clearInterval(id)
  }, [liveMode, run, loadData])

  const stepOptions = useMemo(
    () =>
      steps.map((s) => ({
        value: s.step_id,
        label: `Step ${s.step_index}${s.title ? `: ${s.title}` : ''}`,
      })),
    [steps],
  )

  const totals = useMemo(
    () => ({
      total: logs.length,
      failed: logs.filter((l) => l.status === 'failed').length,
      partial: logs.filter((l) => l.status === 'partial').length,
    }),
    [logs],
  )

  return (
    <div className="space-y-4">
      {/* Filter bar */}
      <FilterBar
        keyword={keyword}
        onKeywordChange={setKeyword}
        placeholder="Search logs..."
        dropdowns={[
          { label: 'Type', value: typeFilter, options: typeOptions, onChange: setTypeFilter },
          { label: 'Status', value: statusFilter, options: statusOptions, onChange: setStatusFilter },
          { label: 'Step', value: stepFilter, options: stepOptions, onChange: setStepFilter },
        ]}
      />

      {/* Stats strip */}
      <div className="flex gap-3 text-xs">
        <span className="text-[var(--cp-muted)]">
          Total: <span className="font-semibold text-[var(--cp-ink)]">{totals.total}</span>
        </span>
        {totals.failed > 0 && (
          <span className="text-rose-600">
            Failed: <span className="font-semibold">{totals.failed}</span>
          </span>
        )}
        {totals.partial > 0 && (
          <span className="text-amber-600">
            Partial: <span className="font-semibold">{totals.partial}</span>
          </span>
        )}
      </div>

      {/* Log list */}
      <div className="rounded-2xl border border-[var(--cp-border)] bg-white">
        {loading ? (
          <div className="space-y-1 p-2">
            {Array.from({ length: 6 }).map((_, i) => (
              <div key={`log-skeleton-${i}`} className="flex animate-pulse items-center gap-3 px-3 py-2.5">
                <div className="size-7 rounded-lg bg-[var(--cp-surface-muted)]" />
                <div className="flex-1 space-y-1.5">
                  <div className="h-2.5 w-32 rounded-full bg-[var(--cp-surface-muted)]" />
                  <div className="h-2.5 w-48 rounded-full bg-[var(--cp-surface-muted)]" />
                </div>
              </div>
            ))}
          </div>
        ) : logs.length > 0 ? (
          <div className="divide-y divide-[var(--cp-border)]/30 p-1">
            {logs.map((log) => (
              <WorkLogRow
                key={log.log_id}
                log={log}
                onClick={() => openInspector({ kind: 'worklog', data: log })}
              />
            ))}
          </div>
        ) : (
          <div className="px-4 py-8 text-center text-sm text-[var(--cp-muted)]">
            <p className="text-[var(--cp-ink)]">No logs match your filters.</p>
            <p className="mt-1 text-xs">Try adjusting filters or selecting a different run.</p>
          </div>
        )}
      </div>
    </div>
  )
}

export default WorkLogTab
