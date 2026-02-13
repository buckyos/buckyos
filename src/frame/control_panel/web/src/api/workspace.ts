// ── Agent Workspace Data Abstraction Layer ──
// All functions return { data, error } following the existing API pattern.
// Currently backed by mock data. Swap internals to callRpc<T>() when connecting to real backend.

// ── Mock Data ──

const now = new Date()
const ago = (minutes: number) => new Date(now.getTime() - minutes * 60_000).toISOString()

const mockAgents: WsAgent[] = [
  {
    agent_id: 'agent-main-001',
    agent_name: 'Planner Agent',
    agent_type: 'main',
    status: 'running',
    current_run_id: 'run-001',
    last_active_at: ago(1),
  },
  {
    agent_id: 'agent-sub-001',
    agent_name: 'Research Worker',
    agent_type: 'sub',
    status: 'running',
    parent_agent_id: 'agent-main-001',
    current_run_id: 'run-sub-001',
    last_active_at: ago(2),
  },
  {
    agent_id: 'agent-sub-002',
    agent_name: 'Code Writer',
    agent_type: 'sub',
    status: 'sleeping',
    parent_agent_id: 'agent-main-001',
    last_active_at: ago(10),
  },
  {
    agent_id: 'agent-main-002',
    agent_name: 'Monitor Agent',
    agent_type: 'main',
    status: 'idle',
    last_active_at: ago(60),
  },
  {
    agent_id: 'agent-main-003',
    agent_name: 'Deploy Agent',
    agent_type: 'main',
    status: 'error',
    last_active_at: ago(30),
  },
]

const mockLoopRuns: Record<string, LoopRun[]> = {
  'agent-main-001': [
    {
      run_id: 'run-001',
      agent_id: 'agent-main-001',
      trigger_event: 'user_request: "Plan feature implementation"',
      status: 'running',
      started_at: ago(15),
      current_step_index: 3,
      duration: 15 * 60,
      summary: { step_count: 5, task_count: 12, log_count: 28, todo_count: 6, sub_agent_count: 2 },
    },
    {
      run_id: 'run-002',
      agent_id: 'agent-main-001',
      trigger_event: 'scheduled: daily_review',
      status: 'success',
      started_at: ago(120),
      ended_at: ago(90),
      duration: 30 * 60,
      current_step_index: 3,
      summary: { step_count: 4, task_count: 8, log_count: 18, todo_count: 4, sub_agent_count: 0 },
    },
    {
      run_id: 'run-003',
      agent_id: 'agent-main-001',
      trigger_event: 'webhook: github_push',
      status: 'failed',
      started_at: ago(240),
      ended_at: ago(235),
      duration: 5 * 60,
      current_step_index: 1,
      summary: { step_count: 2, task_count: 3, log_count: 7, todo_count: 1, sub_agent_count: 0 },
    },
  ],
  'agent-sub-001': [
    {
      run_id: 'run-sub-001',
      agent_id: 'agent-sub-001',
      trigger_event: 'parent_dispatch: research_task',
      status: 'running',
      started_at: ago(10),
      current_step_index: 1,
      duration: 10 * 60,
      summary: { step_count: 2, task_count: 4, log_count: 8, todo_count: 2, sub_agent_count: 0 },
    },
  ],
  'agent-main-002': [],
  'agent-main-003': [
    {
      run_id: 'run-err-001',
      agent_id: 'agent-main-003',
      trigger_event: 'manual: deploy_v2',
      status: 'failed',
      started_at: ago(30),
      ended_at: ago(28),
      duration: 2 * 60,
      current_step_index: 0,
      summary: { step_count: 1, task_count: 1, log_count: 3, todo_count: 0, sub_agent_count: 0 },
    },
  ],
}

const mockSteps: Record<string, WsStep[]> = {
  'run-001': [
    {
      step_id: 'step-001-0',
      step_index: 0,
      title: 'Initialize Context',
      status: 'success',
      started_at: ago(15),
      ended_at: ago(14),
      duration: 60,
      task_count: 2,
      log_counts: { message: 1, function_call: 2, action: 0, sub_agent: 0 },
    },
    {
      step_id: 'step-001-1',
      step_index: 1,
      title: 'Analyze Requirements',
      status: 'success',
      started_at: ago(14),
      ended_at: ago(11),
      duration: 180,
      task_count: 3,
      log_counts: { message: 2, function_call: 3, action: 1, sub_agent: 0 },
    },
    {
      step_id: 'step-001-2',
      step_index: 2,
      title: 'Spawn Sub-Agents',
      status: 'success',
      started_at: ago(11),
      ended_at: ago(8),
      duration: 180,
      task_count: 2,
      log_counts: { message: 1, function_call: 0, action: 0, sub_agent: 2 },
    },
    {
      step_id: 'step-001-3',
      step_index: 3,
      title: 'Generate Implementation Plan',
      status: 'running',
      started_at: ago(8),
      task_count: 3,
      log_counts: { message: 3, function_call: 4, action: 2, sub_agent: 1 },
    },
    {
      step_id: 'step-001-4',
      step_index: 4,
      title: 'Execute Actions',
      status: 'queued' as StepStatus,
      started_at: '',
      task_count: 0,
      log_counts: { message: 0, function_call: 0, action: 0, sub_agent: 0 },
    },
  ],
  'run-002': [
    {
      step_id: 'step-002-0',
      step_index: 0,
      title: 'Load Review Context',
      status: 'success',
      started_at: ago(120),
      ended_at: ago(118),
      duration: 120,
      task_count: 2,
      log_counts: { message: 1, function_call: 1, action: 0, sub_agent: 0 },
    },
    {
      step_id: 'step-002-1',
      step_index: 1,
      title: 'Review Code Changes',
      status: 'success',
      started_at: ago(118),
      ended_at: ago(105),
      duration: 780,
      task_count: 3,
      log_counts: { message: 3, function_call: 4, action: 2, sub_agent: 0 },
    },
    {
      step_id: 'step-002-2',
      step_index: 2,
      title: 'Generate Report',
      status: 'success',
      started_at: ago(105),
      ended_at: ago(95),
      duration: 600,
      task_count: 2,
      log_counts: { message: 2, function_call: 1, action: 1, sub_agent: 0 },
    },
    {
      step_id: 'step-002-3',
      step_index: 3,
      title: 'Send Notifications',
      status: 'success',
      started_at: ago(95),
      ended_at: ago(90),
      duration: 300,
      task_count: 1,
      log_counts: { message: 1, function_call: 0, action: 1, sub_agent: 0 },
    },
  ],
  'run-sub-001': [
    {
      step_id: 'step-sub-0',
      step_index: 0,
      title: 'Fetch Source Material',
      status: 'success',
      started_at: ago(10),
      ended_at: ago(7),
      duration: 180,
      task_count: 2,
      log_counts: { message: 1, function_call: 3, action: 0, sub_agent: 0 },
    },
    {
      step_id: 'step-sub-1',
      step_index: 1,
      title: 'Synthesize Findings',
      status: 'running',
      started_at: ago(7),
      task_count: 2,
      log_counts: { message: 2, function_call: 1, action: 0, sub_agent: 0 },
    },
  ],
  'run-err-001': [
    {
      step_id: 'step-err-0',
      step_index: 0,
      title: 'Validate Deploy Config',
      status: 'failed',
      started_at: ago(30),
      ended_at: ago(28),
      duration: 120,
      task_count: 1,
      log_counts: { message: 0, function_call: 1, action: 0, sub_agent: 0 },
    },
  ],
}

const mockTasks: Record<string, WsTask[]> = {
  'run-001': [
    {
      task_id: 'task-001',
      step_id: 'step-001-0',
      status: 'success',
      model: 'claude-3-opus',
      tokens_in: 1200,
      tokens_out: 450,
      prompt_preview: 'Initialize the planning context for feature implementation...',
      result_preview: 'Context initialized. Identified 3 key modules to modify.',
      raw_input: '{"system":"You are a planning agent...","user":"Plan feature implementation for Agent Workspace"}',
      raw_output: '{"plan":{"modules":["workspace-layout","data-layer","routing"],"priority":"high"}}',
      created_at: ago(15),
      duration: 8,
    },
    {
      task_id: 'task-002',
      step_id: 'step-001-0',
      status: 'success',
      model: 'claude-3-opus',
      tokens_in: 800,
      tokens_out: 320,
      prompt_preview: 'Validate the initialized context against known constraints...',
      result_preview: 'Validation passed. No constraint violations detected.',
      created_at: ago(14.5),
      duration: 5,
    },
    {
      task_id: 'task-003',
      step_id: 'step-001-1',
      status: 'success',
      model: 'claude-3-opus',
      tokens_in: 2400,
      tokens_out: 1800,
      prompt_preview: 'Analyze the workspace requirements from the PRD document...',
      result_preview: 'Identified 5 tabs, 6 entity types, and 25+ component requirements.',
      created_at: ago(14),
      duration: 22,
    },
    {
      task_id: 'task-004',
      step_id: 'step-001-1',
      status: 'success',
      model: 'claude-3-haiku',
      tokens_in: 600,
      tokens_out: 200,
      prompt_preview: 'Extract entity relationships from the requirements...',
      result_preview: 'Agent -> LoopRun -> Step -> Task/WorkLog/Todo',
      created_at: ago(12),
      duration: 3,
    },
    {
      task_id: 'task-005',
      step_id: 'step-001-1',
      status: 'success',
      model: 'claude-3-opus',
      tokens_in: 1500,
      tokens_out: 900,
      prompt_preview: 'Generate component hierarchy based on entity relationships...',
      result_preview: 'WorkspaceLayout > [Sidebar, Header, Tabs[5], Inspector]',
      created_at: ago(11.5),
      duration: 15,
    },
    {
      task_id: 'task-006',
      step_id: 'step-001-2',
      status: 'success',
      model: 'claude-3-opus',
      tokens_in: 1000,
      tokens_out: 600,
      prompt_preview: 'Determine sub-agent allocation for parallel workstreams...',
      result_preview: 'Spawning 2 sub-agents: Research Worker, Code Writer',
      created_at: ago(11),
      duration: 10,
    },
    {
      task_id: 'task-007',
      step_id: 'step-001-2',
      status: 'success',
      model: 'claude-3-haiku',
      tokens_in: 400,
      tokens_out: 150,
      prompt_preview: 'Prepare dispatch messages for sub-agents...',
      result_preview: 'Dispatch messages prepared for Research Worker and Code Writer.',
      created_at: ago(9),
      duration: 2,
    },
    {
      task_id: 'task-008',
      step_id: 'step-001-3',
      status: 'success',
      model: 'claude-3-opus',
      tokens_in: 3200,
      tokens_out: 2400,
      prompt_preview: 'Generate detailed implementation plan with file structure...',
      result_preview: 'Plan generated: 25 new files across 5 phases.',
      created_at: ago(8),
      duration: 35,
    },
    {
      task_id: 'task-009',
      step_id: 'step-001-3',
      status: 'running',
      model: 'claude-3-opus',
      tokens_in: 2000,
      prompt_preview: 'Review and refine the implementation plan for consistency...',
      result_preview: '',
      created_at: ago(4),
    },
    {
      task_id: 'task-010',
      step_id: 'step-001-3',
      status: 'queued',
      model: 'claude-3-opus',
      prompt_preview: 'Finalize plan and prepare execution schedule...',
      result_preview: '',
      created_at: ago(3),
    },
  ],
}

const mockWorkLogs: Record<string, WsWorkLog[]> = {
  'run-001': [
    {
      log_id: 'log-001',
      type: 'message_sent',
      agent_id: 'agent-main-001',
      step_id: 'step-001-0',
      status: 'success',
      timestamp: ago(15),
      summary: 'Sent initialization request to context service',
      payload: { to: 'context-service', content: 'Initialize planning context' },
    },
    {
      log_id: 'log-002',
      type: 'function_call',
      agent_id: 'agent-main-001',
      step_id: 'step-001-0',
      status: 'success',
      timestamp: ago(14.8),
      duration: 1.2,
      summary: 'Called loadProjectConfig()',
      payload: { function: 'loadProjectConfig', input: { path: '/workspace' }, output: { success: true } },
    },
    {
      log_id: 'log-003',
      type: 'function_call',
      agent_id: 'agent-main-001',
      step_id: 'step-001-0',
      status: 'success',
      timestamp: ago(14.5),
      duration: 0.8,
      summary: 'Called validateConstraints()',
      payload: { function: 'validateConstraints', input: {}, output: { valid: true, warnings: [] } },
    },
    {
      log_id: 'log-004',
      type: 'message_sent',
      agent_id: 'agent-main-001',
      step_id: 'step-001-1',
      status: 'success',
      timestamp: ago(14),
      summary: 'Requested PRD analysis from research module',
    },
    {
      log_id: 'log-005',
      type: 'message_reply',
      agent_id: 'agent-main-001',
      related_agent_id: 'agent-sub-001',
      step_id: 'step-001-1',
      status: 'success',
      timestamp: ago(13),
      summary: 'Received PRD analysis results: 5 tabs, 6 entities, 25+ components',
    },
    {
      log_id: 'log-006',
      type: 'function_call',
      agent_id: 'agent-main-001',
      step_id: 'step-001-1',
      status: 'success',
      timestamp: ago(12.5),
      duration: 2.5,
      summary: 'Called readFile("notepads/worksapce.md")',
      payload: { function: 'readFile', input: { path: 'notepads/worksapce.md' }, output: { size: 28400 } },
    },
    {
      log_id: 'log-007',
      type: 'function_call',
      agent_id: 'agent-main-001',
      step_id: 'step-001-1',
      status: 'success',
      timestamp: ago(12),
      duration: 1.8,
      summary: 'Called parseRequirements()',
    },
    {
      log_id: 'log-008',
      type: 'function_call',
      agent_id: 'agent-main-001',
      step_id: 'step-001-1',
      status: 'success',
      timestamp: ago(11.5),
      duration: 0.5,
      summary: 'Called generateComponentTree()',
    },
    {
      log_id: 'log-009',
      type: 'action',
      agent_id: 'agent-main-001',
      step_id: 'step-001-1',
      status: 'success',
      timestamp: ago(11.2),
      duration: 3,
      summary: 'Executed: Write requirements analysis to workspace',
      payload: { action: 'write_file', target: 'analysis.json', status: 'success' },
    },
    {
      log_id: 'log-010',
      type: 'sub_agent_created',
      agent_id: 'agent-main-001',
      related_agent_id: 'agent-sub-001',
      step_id: 'step-001-2',
      status: 'info',
      timestamp: ago(11),
      summary: 'Created sub-agent: Research Worker',
    },
    {
      log_id: 'log-011',
      type: 'sub_agent_created',
      agent_id: 'agent-main-001',
      related_agent_id: 'agent-sub-002',
      step_id: 'step-001-2',
      status: 'info',
      timestamp: ago(10.8),
      summary: 'Created sub-agent: Code Writer',
    },
    {
      log_id: 'log-012',
      type: 'message_sent',
      agent_id: 'agent-main-001',
      related_agent_id: 'agent-sub-001',
      step_id: 'step-001-2',
      status: 'success',
      timestamp: ago(10.5),
      summary: 'Dispatched research task to Research Worker',
    },
    {
      log_id: 'log-013',
      type: 'message_sent',
      agent_id: 'agent-main-001',
      related_agent_id: 'agent-sub-002',
      step_id: 'step-001-3',
      status: 'success',
      timestamp: ago(8),
      summary: 'Sent implementation plan draft to Code Writer',
    },
    {
      log_id: 'log-014',
      type: 'message_reply',
      agent_id: 'agent-main-001',
      related_agent_id: 'agent-sub-001',
      step_id: 'step-001-3',
      status: 'success',
      timestamp: ago(7),
      summary: 'Research Worker reported: source material collected',
    },
    {
      log_id: 'log-015',
      type: 'function_call',
      agent_id: 'agent-main-001',
      step_id: 'step-001-3',
      status: 'success',
      timestamp: ago(6),
      duration: 4.2,
      summary: 'Called generateFileStructure()',
    },
    {
      log_id: 'log-016',
      type: 'function_call',
      agent_id: 'agent-main-001',
      step_id: 'step-001-3',
      status: 'success',
      timestamp: ago(5.5),
      duration: 2.1,
      summary: 'Called estimateComplexity()',
    },
    {
      log_id: 'log-017',
      type: 'function_call',
      agent_id: 'agent-main-001',
      step_id: 'step-001-3',
      status: 'failed',
      timestamp: ago(5),
      duration: 1.0,
      summary: 'Called validateDependencies() - timeout exceeded',
      payload: { function: 'validateDependencies', error: 'Request timed out after 10s' },
    },
    {
      log_id: 'log-018',
      type: 'function_call',
      agent_id: 'agent-main-001',
      step_id: 'step-001-3',
      status: 'success',
      timestamp: ago(4.5),
      duration: 0.8,
      summary: 'Called validateDependencies() - retry succeeded',
    },
    {
      log_id: 'log-019',
      type: 'action',
      agent_id: 'agent-main-001',
      step_id: 'step-001-3',
      status: 'partial',
      timestamp: ago(4),
      duration: 5,
      summary: 'Executed action batch: write 3 plan files (2 success, 1 skipped)',
      payload: {
        batch: [
          { name: 'write plan.md', status: 'success' },
          { name: 'write structure.json', status: 'success' },
          { name: 'write timeline.json', status: 'skipped', reason: 'dependency pending' },
        ],
      },
    },
    {
      log_id: 'log-020',
      type: 'sub_agent_sleep',
      agent_id: 'agent-main-001',
      related_agent_id: 'agent-sub-002',
      step_id: 'step-001-3',
      status: 'info',
      timestamp: ago(3),
      summary: 'Code Writer entered sleep mode (waiting for plan finalization)',
    },
    {
      log_id: 'log-021',
      type: 'message_sent',
      agent_id: 'agent-main-001',
      related_agent_id: 'agent-sub-001',
      step_id: 'step-001-3',
      status: 'success',
      timestamp: ago(2),
      summary: 'Requested additional research on component patterns',
    },
  ],
}

const mockTodos: Record<string, WsTodo[]> = {
  'agent-main-001': [
    {
      todo_id: 'todo-001',
      agent_id: 'agent-main-001',
      title: 'Define workspace type declarations',
      description: 'Add WsAgent, LoopRun, WsStep, WsTask, WsWorkLog, WsTodo types to interface.d.ts',
      status: 'done',
      created_at: ago(15),
      completed_at: ago(12),
      created_in_step_id: 'step-001-0',
      completed_in_step_id: 'step-001-1',
    },
    {
      todo_id: 'todo-002',
      agent_id: 'agent-main-001',
      title: 'Create data abstraction layer',
      description: 'Implement api/workspace.ts with mock data and clean API interface',
      status: 'done',
      created_at: ago(14),
      completed_at: ago(8),
      created_in_step_id: 'step-001-1',
      completed_in_step_id: 'step-001-3',
    },
    {
      todo_id: 'todo-003',
      agent_id: 'agent-main-001',
      title: 'Design component hierarchy',
      status: 'open',
      created_at: ago(12),
      created_in_step_id: 'step-001-1',
    },
    {
      todo_id: 'todo-004',
      agent_id: 'agent-main-001',
      title: 'Implement workspace layout shell',
      status: 'open',
      created_at: ago(8),
      created_in_step_id: 'step-001-3',
    },
    {
      todo_id: 'todo-005',
      agent_id: 'agent-main-001',
      title: 'Build all 5 tab components',
      status: 'open',
      created_at: ago(8),
      created_in_step_id: 'step-001-3',
    },
    {
      todo_id: 'todo-006',
      agent_id: 'agent-main-001',
      title: 'Implement inspector detail templates',
      status: 'open',
      created_at: ago(6),
      created_in_step_id: 'step-001-3',
    },
  ],
  'agent-sub-001': [
    {
      todo_id: 'todo-sub-001',
      agent_id: 'agent-sub-001',
      title: 'Research existing UI patterns in codebase',
      status: 'done',
      created_at: ago(10),
      completed_at: ago(7),
      created_in_step_id: 'step-sub-0',
      completed_in_step_id: 'step-sub-0',
    },
    {
      todo_id: 'todo-sub-002',
      agent_id: 'agent-sub-001',
      title: 'Document component reuse opportunities',
      status: 'open',
      created_at: ago(7),
      created_in_step_id: 'step-sub-1',
    },
  ],
  'agent-sub-002': [],
  'agent-main-002': [],
  'agent-main-003': [],
}

// ── API Functions ──

export const fetchAgents = async (): Promise<{ data: WsAgent[] | null; error: unknown }> => {
  return { data: mockAgents, error: null }
}

export const fetchLoopRuns = async (
  agentId: string,
): Promise<{ data: LoopRun[] | null; error: unknown }> => {
  return { data: mockLoopRuns[agentId] ?? [], error: null }
}

export const fetchSteps = async (
  runId: string,
): Promise<{ data: WsStep[] | null; error: unknown }> => {
  return { data: mockSteps[runId] ?? [], error: null }
}

export const fetchWsTasks = async (
  runId: string,
  filters?: WsTaskFilters,
): Promise<{ data: WsTask[] | null; error: unknown }> => {
  let tasks = mockTasks[runId] ?? []
  if (filters?.stepId) {
    tasks = tasks.filter((t) => t.step_id === filters.stepId)
  }
  if (filters?.status) {
    tasks = tasks.filter((t) => t.status === filters.status)
  }
  return { data: tasks, error: null }
}

export const fetchWorkLogs = async (
  runId: string,
  filters?: WsWorkLogFilters,
): Promise<{ data: WsWorkLog[] | null; error: unknown }> => {
  let logs = mockWorkLogs[runId] ?? []
  if (filters?.stepId) {
    logs = logs.filter((l) => l.step_id === filters.stepId)
  }
  if (filters?.type) {
    logs = logs.filter((l) => l.type === filters.type)
  }
  if (filters?.status) {
    logs = logs.filter((l) => l.status === filters.status)
  }
  if (filters?.keyword) {
    const kw = filters.keyword.toLowerCase()
    logs = logs.filter((l) => l.summary.toLowerCase().includes(kw))
  }
  return { data: logs, error: null }
}

export const fetchTodos = async (
  agentId: string,
): Promise<{ data: WsTodo[] | null; error: unknown }> => {
  return { data: mockTodos[agentId] ?? [], error: null }
}

export const fetchSubAgents = async (
  agentId: string,
): Promise<{ data: WsAgent[] | null; error: unknown }> => {
  const subs = mockAgents.filter((a) => a.parent_agent_id === agentId)
  return { data: subs, error: null }
}
