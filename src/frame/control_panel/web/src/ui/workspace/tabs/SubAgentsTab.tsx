import { useCallback, useEffect, useState } from 'react'

import { fetchSubAgents } from '@/api/workspace'
import Icon from '../../icons'
import { useWorkspace } from '../WorkspaceContext'
import SubAgentNode from '../components/SubAgentNode'

const SubAgentsTab = () => {
  const { selectedAgentId, liveMode, openInspector, setSelectedAgentId } = useWorkspace()

  const [subAgents, setSubAgents] = useState<WsAgent[]>([])
  const [loading, setLoading] = useState(true)

  const loadData = useCallback(
    async (silent = false) => {
      if (!selectedAgentId) return
      if (!silent) setLoading(true)
      const { data } = await fetchSubAgents(selectedAgentId)
      if (data) setSubAgents(data)
      setLoading(false)
    },
    [selectedAgentId],
  )

  useEffect(() => {
    loadData()
  }, [loadData])

  useEffect(() => {
    if (!liveMode) return
    const id = window.setInterval(() => loadData(true), 5000)
    return () => window.clearInterval(id)
  }, [liveMode, loadData])

  if (loading) {
    return (
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {Array.from({ length: 3 }).map((_, i) => (
          <div
            key={`sub-skeleton-${i}`}
            className="animate-pulse rounded-2xl border border-[var(--cp-border)] bg-white p-4"
          >
            <div className="flex items-center gap-2">
              <div className="size-8 rounded-xl bg-[var(--cp-surface-muted)]" />
              <div className="space-y-1.5">
                <div className="h-3 w-24 rounded-full bg-[var(--cp-surface-muted)]" />
                <div className="h-2 w-16 rounded-full bg-[var(--cp-surface-muted)]" />
              </div>
            </div>
            <div className="mt-3 grid grid-cols-2 gap-2">
              <div className="h-10 rounded-lg bg-[var(--cp-surface-muted)]" />
              <div className="h-10 rounded-lg bg-[var(--cp-surface-muted)]" />
            </div>
          </div>
        ))}
      </div>
    )
  }

  if (subAgents.length === 0) {
    return (
      <div className="rounded-2xl border border-[var(--cp-border)] bg-white px-6 py-12 text-center">
        <Icon name="branch" className="mx-auto size-8 text-[var(--cp-muted)]" />
        <p className="mt-3 text-sm font-medium text-[var(--cp-ink)]">No sub-agents</p>
        <p className="mt-1 text-xs text-[var(--cp-muted)]">
          This agent has not created any parallel sub-agents in the current run.
        </p>
      </div>
    )
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center gap-2 text-xs text-[var(--cp-muted)]">
        <Icon name="branch" className="size-3.5" />
        <span>
          {subAgents.length} sub-agent{subAgents.length !== 1 ? 's' : ''}
        </span>
        <span className="text-[var(--cp-primary)]">
          {subAgents.filter((a) => a.status === 'running').length} active
        </span>
      </div>

      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {subAgents.map((agent) => (
          <SubAgentNode
            key={agent.agent_id}
            agent={agent}
            onClick={() => openInspector({ kind: 'sub-agent', data: agent })}
            onOpenWorkspace={() => setSelectedAgentId(agent.agent_id)}
          />
        ))}
      </div>
    </div>
  )
}

export default SubAgentsTab
