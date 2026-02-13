import { createContext, useCallback, useContext, useEffect, useMemo, useState } from 'react'
import type { ReactNode } from 'react'

import { fetchAgents, fetchLoopRuns } from '@/api/workspace'

type WorkspaceState = {
  agents: WsAgent[]
  runs: LoopRun[]
  selectedAgentId: string | null
  selectedRunId: string | null
  activeTab: WsTabId
  inspectorTarget: InspectorTarget | null
  liveMode: boolean
  agentsLoading: boolean
  runsLoading: boolean
  pendingWorkLogFilters: WsWorkLogFilters | null
}

type WorkspaceActions = {
  setSelectedAgentId: (id: string) => void
  setSelectedRunId: (id: string) => void
  setActiveTab: (tab: WsTabId) => void
  openInspector: (target: InspectorTarget) => void
  closeInspector: () => void
  toggleLiveMode: () => void
  navigateToWorkLog: (filters: WsWorkLogFilters) => void
  clearPendingWorkLogFilters: () => void
}

type WorkspaceContextValue = WorkspaceState & WorkspaceActions

const WorkspaceContext = createContext<WorkspaceContextValue | null>(null)

const POLL_INTERVAL = 5000

export const WorkspaceProvider = ({ children }: { children: ReactNode }) => {
  const [agents, setAgents] = useState<WsAgent[]>([])
  const [runs, setRuns] = useState<LoopRun[]>([])
  const [selectedAgentId, setSelectedAgentIdRaw] = useState<string | null>(null)
  const [selectedRunId, setSelectedRunIdRaw] = useState<string | null>(null)
  const [activeTab, setActiveTab] = useState<WsTabId>('overview')
  const [inspectorTarget, setInspectorTarget] = useState<InspectorTarget | null>(null)
  const [liveMode, setLiveMode] = useState(true)
  const [agentsLoading, setAgentsLoading] = useState(true)
  const [runsLoading, setRunsLoading] = useState(false)
  const [pendingWorkLogFilters, setPendingWorkLogFilters] = useState<WsWorkLogFilters | null>(null)

  // Load agents
  const loadAgents = useCallback(async (silent = false) => {
    if (!silent) setAgentsLoading(true)
    const { data } = await fetchAgents()
    if (data) {
      setAgents(data)
      // Auto-select first agent if none selected
      setSelectedAgentIdRaw((prev) => {
        if (prev && data.some((a) => a.agent_id === prev)) return prev
        return data[0]?.agent_id ?? null
      })
    }
    setAgentsLoading(false)
  }, [])

  // Load runs when agent changes
  const loadRuns = useCallback(async (agentId: string) => {
    setRunsLoading(true)
    const { data } = await fetchLoopRuns(agentId)
    const runList = data ?? []
    setRuns(runList)
    // Auto-select current or first run
    setSelectedRunIdRaw((prev) => {
      if (prev && runList.some((r) => r.run_id === prev)) return prev
      const agent = agents.find((a) => a.agent_id === agentId)
      if (agent?.current_run_id && runList.some((r) => r.run_id === agent.current_run_id)) {
        return agent.current_run_id
      }
      return runList[0]?.run_id ?? null
    })
    setRunsLoading(false)
  }, [agents])

  // Initial load
  useEffect(() => {
    loadAgents()
  }, [loadAgents])

  // Load runs when selected agent changes
  useEffect(() => {
    if (selectedAgentId) {
      loadRuns(selectedAgentId)
    } else {
      setRuns([])
      setSelectedRunIdRaw(null)
    }
  }, [selectedAgentId, loadRuns])

  // Polling
  useEffect(() => {
    if (!liveMode) return
    const id = window.setInterval(() => {
      loadAgents(true)
      if (selectedAgentId) loadRuns(selectedAgentId)
    }, POLL_INTERVAL)
    return () => window.clearInterval(id)
  }, [liveMode, selectedAgentId, loadAgents, loadRuns])

  // Actions
  const setSelectedAgentId = useCallback((id: string) => {
    setSelectedAgentIdRaw(id)
    setInspectorTarget(null)
    setActiveTab('overview')
  }, [])

  const setSelectedRunId = useCallback((id: string) => {
    setSelectedRunIdRaw(id)
    setInspectorTarget(null)
  }, [])

  const openInspector = useCallback((target: InspectorTarget) => {
    setInspectorTarget(target)
  }, [])

  const closeInspector = useCallback(() => {
    setInspectorTarget(null)
  }, [])

  const toggleLiveMode = useCallback(() => {
    setLiveMode((prev) => !prev)
  }, [])

  const navigateToWorkLog = useCallback((filters: WsWorkLogFilters) => {
    setPendingWorkLogFilters(filters)
    setActiveTab('worklog')
  }, [])

  const clearPendingWorkLogFilters = useCallback(() => {
    setPendingWorkLogFilters(null)
  }, [])

  const value = useMemo<WorkspaceContextValue>(
    () => ({
      agents,
      runs,
      selectedAgentId,
      selectedRunId,
      activeTab,
      inspectorTarget,
      liveMode,
      agentsLoading,
      runsLoading,
      pendingWorkLogFilters,
      setSelectedAgentId,
      setSelectedRunId,
      setActiveTab,
      openInspector,
      closeInspector,
      toggleLiveMode,
      navigateToWorkLog,
      clearPendingWorkLogFilters,
    }),
    [
      agents,
      runs,
      selectedAgentId,
      selectedRunId,
      activeTab,
      inspectorTarget,
      liveMode,
      agentsLoading,
      runsLoading,
      pendingWorkLogFilters,
      setSelectedAgentId,
      setSelectedRunId,
      setActiveTab,
      openInspector,
      closeInspector,
      toggleLiveMode,
      navigateToWorkLog,
      clearPendingWorkLogFilters,
    ],
  )

  return <WorkspaceContext.Provider value={value}>{children}</WorkspaceContext.Provider>
}

export const useWorkspace = (): WorkspaceContextValue => {
  const ctx = useContext(WorkspaceContext)
  if (!ctx) throw new Error('useWorkspace must be used within WorkspaceProvider')
  return ctx
}

export const useSelectedAgent = (): WsAgent | null => {
  const { agents, selectedAgentId } = useWorkspace()
  return useMemo(
    () => agents.find((a) => a.agent_id === selectedAgentId) ?? null,
    [agents, selectedAgentId],
  )
}

export const useCurrentRun = (): LoopRun | null => {
  const { runs, selectedRunId } = useWorkspace()
  return useMemo(
    () => runs.find((r) => r.run_id === selectedRunId) ?? null,
    [runs, selectedRunId],
  )
}
