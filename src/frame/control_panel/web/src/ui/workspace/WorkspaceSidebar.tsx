import { useCallback, useEffect, useState } from 'react'

import { fetchAgentSessions } from '@/api/workspace'

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

const sessionStatusDotClass = (status: string): string => {
  const key = status.trim().toLowerCase()
  if (key === 'active' || key === 'running') return 'bg-[var(--cp-primary)]'
  if (key === 'error' || key === 'failed') return 'bg-[var(--cp-danger)]'
  if (key === 'closed' || key === 'done') return 'bg-slate-400'
  return 'bg-amber-400'
}

const shortSessionId = (sessionId: string): string =>
  sessionId.length > 18 ? `${sessionId.slice(0, 10)}...${sessionId.slice(-6)}` : sessionId

const WorkspaceSidebar = () => {
  const { agents, selectedAgentId, setSelectedAgentId, agentsLoading } = useWorkspace()
  const [expandedAgents, setExpandedAgents] = useState<Record<string, boolean>>({})
  const [sessionsByAgent, setSessionsByAgent] = useState<Record<string, WsAgentSession[]>>({})
  const [sessionLoadingByAgent, setSessionLoadingByAgent] = useState<Record<string, boolean>>({})
  const [sessionErrorByAgent, setSessionErrorByAgent] = useState<Record<string, boolean>>({})

  const mainAgents = agents.filter((a) => a.agent_type === 'main')
  const subAgents = agents.filter((a) => a.agent_type === 'sub')

  const ensureSessionsLoaded = useCallback(async (agentId: string) => {
    if (sessionLoadingByAgent[agentId] || sessionsByAgent[agentId]) return

    setSessionLoadingByAgent((prev) => ({ ...prev, [agentId]: true }))
    setSessionErrorByAgent((prev) => ({ ...prev, [agentId]: false }))
    const { data, error } = await fetchAgentSessions(agentId)
    setSessionsByAgent((prev) => ({ ...prev, [agentId]: data ?? [] }))
    setSessionErrorByAgent((prev) => ({ ...prev, [agentId]: Boolean(error) }))
    setSessionLoadingByAgent((prev) => ({ ...prev, [agentId]: false }))
  }, [sessionLoadingByAgent, sessionsByAgent])

  useEffect(() => {
    if (!selectedAgentId) return
    setExpandedAgents((prev) =>
      prev[selectedAgentId] ? prev : { ...prev, [selectedAgentId]: true },
    )
    void ensureSessionsLoaded(selectedAgentId)
  }, [selectedAgentId, ensureSessionsLoaded])

  useEffect(() => {
    setExpandedAgents((prev) => {
      const next: Record<string, boolean> = {}
      agents.forEach((agent) => {
        if (prev[agent.agent_id]) next[agent.agent_id] = true
      })
      if (selectedAgentId) next[selectedAgentId] = true
      return next
    })
  }, [agents, selectedAgentId])

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

  const toggleSessions = (agentId: string) => {
    const shouldExpand = !expandedAgents[agentId]
    setExpandedAgents((prev) => ({ ...prev, [agentId]: shouldExpand }))
    if (shouldExpand) {
      void ensureSessionsLoaded(agentId)
    }
  }

  const renderSessions = (agentId: string) => {
    if (!expandedAgents[agentId]) return null
    const loading = sessionLoadingByAgent[agentId]
    const hasError = sessionErrorByAgent[agentId]
    const sessions = sessionsByAgent[agentId] ?? []

    return (
      <div className="ml-7 mt-1 space-y-1 border-l border-[var(--cp-border)] pl-3">
        {loading && (
          <div className="rounded-lg bg-[var(--cp-surface-muted)] px-3 py-2 text-[11px] text-[var(--cp-muted)]">
            Loading sessions...
          </div>
        )}
        {!loading && hasError && (
          <button
            type="button"
            onClick={() => {
              setSessionsByAgent((prev) => {
                const next = { ...prev }
                delete next[agentId]
                return next
              })
              void ensureSessionsLoaded(agentId)
            }}
            className="w-full rounded-lg border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2 text-left text-[11px] text-[var(--cp-muted)] transition hover:border-[var(--cp-primary)] hover:text-[var(--cp-ink)]"
          >
            Sessions unavailable. Click to retry.
          </button>
        )}
        {!loading && !hasError && sessions.length === 0 && (
          <div className="rounded-lg bg-[var(--cp-surface-muted)] px-3 py-2 text-[11px] text-[var(--cp-muted)]">
            No sessions
          </div>
        )}
        {!loading &&
          !hasError &&
          sessions.map((session) => (
            <div
              key={session.session_id}
              className="rounded-lg border border-[var(--cp-border)] bg-white px-3 py-2"
            >
              <div className="flex items-center gap-2">
                <span className={`size-2 rounded-full ${sessionStatusDotClass(session.status)}`} />
                <p className="flex-1 truncate text-[11px] font-medium text-[var(--cp-ink)]">
                  {session.title}
                </p>
                <span className="text-[10px] uppercase tracking-wide text-[var(--cp-muted)]">
                  {session.status}
                </span>
              </div>
              <p className="mt-1 truncate text-[10px] text-[var(--cp-muted)]">
                {shortSessionId(session.session_id)}
              </p>
            </div>
          ))}
      </div>
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
          const isExpanded = Boolean(expandedAgents[agent.agent_id])
          return (
            <div key={agent.agent_id} className="space-y-1">
              <div className="flex items-center gap-1">
                <button
                  type="button"
                  onClick={() => setSelectedAgentId(agent.agent_id)}
                  className={`flex min-h-11 flex-1 items-center gap-3 rounded-xl px-3 py-2.5 text-left text-sm transition ${
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
                <button
                  type="button"
                  onClick={() => toggleSessions(agent.agent_id)}
                  className={`inline-flex min-h-11 w-11 flex-none items-center justify-center rounded-xl transition ${
                    isSelected
                      ? 'bg-[var(--cp-primary)] text-white shadow-lg shadow-emerald-200'
                      : 'text-[var(--cp-muted)] hover:bg-[var(--cp-surface-muted)] hover:text-[var(--cp-ink)]'
                  }`}
                  aria-label={`${isExpanded ? 'Collapse' : 'Expand'} sessions for ${agent.agent_name}`}
                  aria-expanded={isExpanded}
                >
                  <Icon name={isExpanded ? 'chevron-down' : 'chevron-right'} className="size-4" />
                </button>
              </div>
              {renderSessions(agent.agent_id)}
            </div>
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
