import { OpenDanClient, TaskManagerClient, buckyos } from 'buckyos'

// ── Agent Workspace Data Abstraction Layer ──
// All functions return { data, error } following the existing API pattern.
// Prefer real OpenDan / TaskManager data, and fall back to mock data on error.

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

const mockAgentSessions: Record<string, WsAgentSession[]> = {
  'agent-main-001': [
    {
      session_id: 'sess-main-001-a',
      owner_agent: 'agent-main-001',
      title: 'Feature planning session',
      summary: 'Current planning and decomposition workflow',
      status: 'active',
      created_at: ago(20),
      updated_at: ago(2),
      last_activity_at: ago(2),
    },
    {
      session_id: 'sess-main-001-b',
      owner_agent: 'agent-main-001',
      title: 'Daily review session',
      summary: 'Periodic review and housekeeping',
      status: 'closed',
      created_at: ago(150),
      updated_at: ago(90),
      last_activity_at: ago(90),
    },
  ],
  'agent-sub-001': [
    {
      session_id: 'sess-sub-001-a',
      owner_agent: 'agent-sub-001',
      title: 'Research thread',
      summary: 'Collecting references for UI implementation',
      status: 'active',
      created_at: ago(20),
      updated_at: ago(4),
      last_activity_at: ago(4),
    },
  ],
  'agent-sub-002': [
    {
      session_id: 'sess-sub-002-a',
      owner_agent: 'agent-sub-002',
      title: 'Code generation buffer',
      summary: 'Pending updates after plan confirmation',
      status: 'sleeping',
      created_at: ago(30),
      updated_at: ago(12),
      last_activity_at: ago(12),
    },
  ],
  'agent-main-002': [],
  'agent-main-003': [
    {
      session_id: 'sess-main-003-a',
      owner_agent: 'agent-main-003',
      title: 'Deploy retry session',
      summary: 'Last deployment run',
      status: 'error',
      created_at: ago(40),
      updated_at: ago(28),
      last_activity_at: ago(28),
    },
  ],
}

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

type TaskManagerTask = Awaited<ReturnType<TaskManagerClient['getTask']>>
type OpenDanAgent = Awaited<ReturnType<OpenDanClient['getAgent']>>
type OpenDanWorklog = Awaited<ReturnType<OpenDanClient['listWorkspaceWorklogs']>>['items'][number]
type OpenDanTodo = Awaited<ReturnType<OpenDanClient['listWorkspaceTodos']>>['items'][number]
type OpenDanSubAgent = Awaited<ReturnType<OpenDanClient['listWorkspaceSubAgents']>>['items'][number]
type OpenDanAgentSessionId = Awaited<ReturnType<OpenDanClient['listAgentSessions']>>['items'][number]
type OpenDanAgentSession = Awaited<ReturnType<OpenDanClient['getAgentSession']>>

type RunMeta = {
  run_id: string
  agent_id: string
  owner_session_id?: string
  started_at_ms: number
  ended_at_ms?: number
  step_ids: Set<string>
}

type TaskRunRef = {
  run_id: string
  step_index: number
}

type ParsedAiccRequestId = {
  run_id: string
  behavior: string
  step_index: number
}

type AgentRunData = {
  runs: LoopRun[]
  steps_by_run: Map<string, WsStep[]>
  tasks_by_run: Map<string, WsTask[]>
  run_metas: Map<string, RunMeta>
}

const opendanClient = new OpenDanClient(new buckyos.kRPCClient('/kapi/opendan/'))
const taskMgrClient = new TaskManagerClient(new buckyos.kRPCClient('/kapi/task-manager/'))

const WORKSPACE_TASK_APP_ID = 'opendan-llm-behavior'
const AGENT_RUN_CACHE_TTL_MS = 5000
const RUN_WORKLOG_CACHE_TTL_MS = 5000
const AGENT_SESSION_CACHE_TTL_MS = 5000

const agentRunCache = new Map<string, { at: number; data: AgentRunData }>()
const runMetaCache = new Map<string, RunMeta>()
const runWorklogCache = new Map<string, { at: number; logs: WsWorkLog[] }>()
const agentSessionCache = new Map<string, { at: number; sessions: WsAgentSession[] }>()

const normalizeKey = (value: string): string => value.trim().toLowerCase().replace(/[\s-]+/g, '_')

const asObject = (value: unknown): Record<string, unknown> | null => {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  return value as Record<string, unknown>
}

const readPath = (value: unknown, path: Array<string | number>): unknown => {
  let cursor: unknown = value
  for (const key of path) {
    if (typeof key === 'number') {
      if (!Array.isArray(cursor) || key < 0 || key >= cursor.length) return undefined
      cursor = cursor[key]
      continue
    }
    const obj = asObject(cursor)
    if (!obj || !(key in obj)) return undefined
    cursor = obj[key]
  }
  return cursor
}

const readString = (value: unknown, paths: Array<Array<string | number>>): string | undefined => {
  for (const path of paths) {
    const raw = readPath(value, path)
    if (typeof raw === 'string') {
      const trimmed = raw.trim()
      if (trimmed) return trimmed
    }
  }
  return undefined
}

const readNumber = (value: unknown, paths: Array<Array<string | number>>): number | undefined => {
  for (const path of paths) {
    const raw = readPath(value, path)
    if (typeof raw === 'number' && Number.isFinite(raw)) return raw
    if (typeof raw === 'string') {
      const parsed = Number(raw)
      if (Number.isFinite(parsed)) return parsed
    }
  }
  return undefined
}

const toMillis = (value: unknown): number | undefined => {
  if (typeof value === 'number' && Number.isFinite(value)) {
    return value < 1_000_000_000_000 ? Math.round(value * 1000) : Math.round(value)
  }
  if (typeof value === 'string') {
    const trimmed = value.trim()
    if (!trimmed) return undefined
    const numeric = Number(trimmed)
    if (Number.isFinite(numeric)) {
      return numeric < 1_000_000_000_000 ? Math.round(numeric * 1000) : Math.round(numeric)
    }
    const parsed = Date.parse(trimmed)
    if (Number.isFinite(parsed)) return parsed
  }
  return undefined
}

const toIso = (value: unknown, fallback = now.toISOString()): string => {
  const ms = toMillis(value)
  if (ms == null) return fallback
  return new Date(ms).toISOString()
}

const preview = (value: unknown, max = 140): string => {
  if (value == null) return ''
  const raw =
    typeof value === 'string'
      ? value
      : typeof value === 'number' || typeof value === 'boolean'
        ? String(value)
        : JSON.stringify(value)
  const text = raw.trim().replace(/\s+/g, ' ')
  if (!text) return ''
  return text.length > max ? `${text.slice(0, max - 3)}...` : text
}

const stringifyJson = (value: unknown): string | undefined => {
  if (value == null) return undefined
  try {
    return JSON.stringify(value)
  } catch {
    return undefined
  }
}

const stepIdFromIndex = (stepIndex: number): string => `step-${Math.max(0, Math.trunc(stepIndex))}`

const taskStatusToWsTaskStatus = (status: string): WsTaskStatus => {
  switch (normalizeKey(status)) {
    case 'running':
      return 'running'
    case 'completed':
      return 'success'
    case 'failed':
    case 'canceled':
      return 'failed'
    default:
      return 'queued'
  }
}

const taskStatusToStepStatus = (status: string): StepStatus => {
  switch (normalizeKey(status)) {
    case 'running':
      return 'running'
    case 'completed':
      return 'success'
    case 'failed':
      return 'failed'
    case 'canceled':
      return 'skipped'
    default:
      return 'running'
  }
}

const taskStatusToRunStatus = (status: string): LoopRunStatus => {
  switch (normalizeKey(status)) {
    case 'running':
      return 'running'
    case 'completed':
      return 'success'
    case 'canceled':
      return 'cancelled'
    case 'failed':
      return 'failed'
    default:
      return 'running'
  }
}

const normalizeAgentType = (agentType: string | undefined): AgentType =>
  normalizeKey(agentType ?? '') === 'sub' ? 'sub' : 'main'

const normalizeAgentStatus = (status: string | undefined): AgentStatus => {
  switch (normalizeKey(status ?? '')) {
    case 'running':
      return 'running'
    case 'sleeping':
    case 'paused':
      return 'sleeping'
    case 'error':
    case 'failed':
      return 'error'
    case 'offline':
    case 'disabled':
    case 'stopped':
      return 'offline'
    default:
      return 'idle'
  }
}

const normalizeTodoStatus = (status: string | undefined): WsTodoStatus =>
  normalizeKey(status ?? '') === 'done' || normalizeKey(status ?? '') === 'cancelled'
    ? 'done'
    : 'open'

const normalizeWorkLogStatus = (status: string | undefined): WorkLogStatus => {
  switch (normalizeKey(status ?? '')) {
    case 'success':
      return 'success'
    case 'failed':
      return 'failed'
    case 'partial':
      return 'partial'
    default:
      return 'info'
  }
}

const normalizeWorkLogType = (logType: string | undefined): WorkLogType => {
  switch (normalizeKey(logType ?? '')) {
    case 'message_sent':
      return 'message_sent'
    case 'message_reply':
      return 'message_reply'
    case 'function_call':
      return 'function_call'
    case 'sub_agent_created':
      return 'sub_agent_created'
    case 'sub_agent_sleep':
      return 'sub_agent_sleep'
    case 'sub_agent_wake':
      return 'sub_agent_wake'
    case 'sub_agent_destroyed':
      return 'sub_agent_destroyed'
    default:
      return 'action'
  }
}

const taskBelongsToAgent = (task: TaskManagerTask, agentId: string): boolean => {
  if (task.user_id === agentId) return true
  return readString(task.data, [['agent_did'], ['trace', 'agent_did']]) === agentId
}

const isWorkspaceTask = (task: TaskManagerTask): boolean => {
  if (task.task_type === 'llm_behavior') return true
  if (task.task_type.startsWith('aicc.')) return true
  const hasWakeup = readString(task.data, [['wakeup_id'], ['aicc', 'request', 'id']])
  return Boolean(hasWakeup)
}

const parseAiccRequestId = (taskData: unknown): ParsedAiccRequestId | undefined => {
  const requestId = readString(taskData, [['aicc', 'request', 'id']])
  if (!requestId) return undefined
  const parts = requestId.split(':')
  if (parts.length < 4) return undefined
  const runId = parts[1]?.trim()
  const behavior = parts[2]?.trim()
  const step = Number(parts[3])
  if (!runId || !behavior || !Number.isFinite(step)) return undefined
  return {
    run_id: runId,
    behavior,
    step_index: Math.max(0, Math.trunc(step)),
  }
}

const parseRefFromAiccRequestId = (taskData: unknown): TaskRunRef | undefined => {
  const parsed = parseAiccRequestId(taskData)
  if (!parsed) return undefined
  return { run_id: parsed.run_id, step_index: parsed.step_index }
}

const resolveTaskRunRef = (
  task: TaskManagerTask,
  taskById: Map<number, TaskManagerTask>,
  behaviorRefs: Map<number, TaskRunRef>,
): TaskRunRef | undefined => {
  const directRunId = readString(task.data, [['wakeup_id']])
  const directStep = readNumber(task.data, [['step_idx']])
  if (directRunId) {
    return {
      run_id: directRunId,
      step_index: Math.max(0, Math.trunc(directStep ?? 0)),
    }
  }

  const parsed = parseRefFromAiccRequestId(task.data)
  if (parsed) return parsed

  const firstParent = task.parent_id ?? task.root_id
  if (firstParent == null) return undefined

  let cursor: number | null = firstParent
  const visited = new Set<number>()
  while (cursor != null && !visited.has(cursor)) {
    visited.add(cursor)
    const ref = behaviorRefs.get(cursor)
    if (ref) return ref
    const parent = taskById.get(cursor)
    if (!parent) break
    const parentDirectRunId = readString(parent.data, [['wakeup_id']])
    const parentStep = readNumber(parent.data, [['step_idx']])
    if (parentDirectRunId) {
      return {
        run_id: parentDirectRunId,
        step_index: Math.max(0, Math.trunc(parentStep ?? 0)),
      }
    }
    const parentParsed = parseRefFromAiccRequestId(parent.data)
    if (parentParsed) return parentParsed
    cursor = parent.parent_id ?? parent.root_id
  }

  return undefined
}

const extractTaskPrompt = (task: TaskManagerTask): string => {
  const messages = readPath(task.data, ['aicc', 'request', 'payload', 'messages'])
  if (Array.isArray(messages)) {
    for (let index = messages.length - 1; index >= 0; index -= 1) {
      const msg = asObject(messages[index])
      const content = msg?.content
      if (typeof content === 'string' && content.trim()) return preview(content)
    }
  }
  const fromData = readString(task.data, [['prompt'], ['input'], ['message'], ['aicc', 'request', 'id']])
  if (fromData) return preview(fromData)
  return preview(task.name)
}

const extractTaskResult = (task: TaskManagerTask): string => {
  const resultText = readString(task.data, [
    ['aicc', 'output', 'text'],
    ['result', 'text'],
    ['output', 'text'],
    ['error', 'message'],
    ['aicc', 'error', 'message'],
  ])
  if (resultText) return preview(resultText)
  const resultJson =
    readPath(task.data, ['aicc', 'output', 'json']) ??
    readPath(task.data, ['result', 'json']) ??
    readPath(task.data, ['result'])
  return preview(resultJson)
}

const extractTaskBehavior = (task: TaskManagerTask): string | undefined => {
  const fromTask = readString(task.data, [['behavior']])
  if (fromTask) return fromTask
  return parseAiccRequestId(task.data)?.behavior
}

const extractTaskOwnerSessionId = (task: TaskManagerTask): string | undefined =>
  readString(task.data, [
    ['owner_session_id'],
    ['ownerSessionId'],
    ['session_id'],
    ['sessionId'],
    ['aicc', 'request', 'owner_session_id'],
    ['aicc', 'request', 'ownerSessionId'],
    ['aicc', 'request', 'session_id'],
    ['aicc', 'request', 'sessionId'],
    ['trace', 'owner_session_id'],
    ['trace', 'session_id'],
  ])

const mapTaskToWsTask = (task: TaskManagerTask, runRef: TaskRunRef): WsTask => {
  const createdAtMs = toMillis(task.created_at) ?? Date.now()
  const updatedAtMs = toMillis(task.updated_at) ?? createdAtMs
  const duration =
    updatedAtMs >= createdAtMs ? Math.round((updatedAtMs - createdAtMs) / 1000) : undefined

  const outputData =
    readPath(task.data, ['aicc', 'output']) ??
    readPath(task.data, ['result']) ??
    readPath(task.data, ['output'])

  const model =
    readString(task.data, [
      ['aicc', 'output', 'extra', 'model'],
      ['result', 'extra', 'model'],
      ['aicc', 'route', 'provider_model'],
      ['aicc', 'request', 'model', 'id'],
      ['task_type'],
    ]) ?? task.task_type

  const tokensIn = readNumber(task.data, [
    ['aicc', 'output', 'usage', 'input_tokens'],
    ['result', 'usage', 'input_tokens'],
    ['usage', 'input_tokens'],
  ])
  const tokensOut = readNumber(task.data, [
    ['aicc', 'output', 'usage', 'output_tokens'],
    ['result', 'usage', 'output_tokens'],
    ['usage', 'output_tokens'],
  ])

  return {
    task_id: String(task.id),
    step_id: stepIdFromIndex(runRef.step_index),
    behavior_id: extractTaskBehavior(task),
    status: taskStatusToWsTaskStatus(String(task.status)),
    model,
    tokens_in: tokensIn != null ? Math.max(0, Math.trunc(tokensIn)) : undefined,
    tokens_out: tokensOut != null ? Math.max(0, Math.trunc(tokensOut)) : undefined,
    prompt_preview: extractTaskPrompt(task),
    result_preview: extractTaskResult(task),
    raw_input: stringifyJson(readPath(task.data, ['aicc', 'request']) ?? task.data),
    raw_output: stringifyJson(outputData),
    created_at: new Date(createdAtMs).toISOString(),
    duration,
  }
}

const loadAgentTasks = async (agentId: string): Promise<TaskManagerTask[]> => {
  const byWorkspaceApp = await taskMgrClient.listTasks({
    filter: { app_id: WORKSPACE_TASK_APP_ID },
  })
  let tasks = byWorkspaceApp.filter((task) => taskBelongsToAgent(task, agentId))
  if (tasks.length > 0) return tasks.filter(isWorkspaceTask)

  const allTasks = await taskMgrClient.listTasks()
  tasks = allTasks.filter((task) => taskBelongsToAgent(task, agentId))
  return tasks.filter(isWorkspaceTask)
}

const buildAgentRunData = (agentId: string, tasks: TaskManagerTask[]): AgentRunData => {
  const ordered = [...tasks].sort(
    (a, b) => (toMillis(a.created_at) ?? 0) - (toMillis(b.created_at) ?? 0),
  )
  const taskById = new Map<number, TaskManagerTask>()
  ordered.forEach((task) => taskById.set(task.id, task))

  const behaviorRefs = new Map<number, TaskRunRef>()
  for (const task of ordered) {
    if (task.task_type !== 'llm_behavior') continue
    const runId = readString(task.data, [['wakeup_id']]) ?? `run-${task.id}`
    const stepIndex = Math.max(0, Math.trunc(readNumber(task.data, [['step_idx']]) ?? 0))
    behaviorRefs.set(task.id, { run_id: runId, step_index: stepIndex })
  }

  const runTaskRefs = new Map<string, Array<{ task: TaskManagerTask; ref: TaskRunRef }>>()
  for (const task of ordered) {
    const ref = resolveTaskRunRef(task, taskById, behaviorRefs)
    if (!ref) continue
    const list = runTaskRefs.get(ref.run_id) ?? []
    list.push({ task, ref })
    runTaskRefs.set(ref.run_id, list)
  }

  const runs: LoopRun[] = []
  const stepsByRun = new Map<string, WsStep[]>()
  const tasksByRun = new Map<string, WsTask[]>()
  const runMetas = new Map<string, RunMeta>()

  for (const [runId, refs] of runTaskRefs.entries()) {
    const wsTasks = refs
      .map(({ task, ref }) => mapTaskToWsTask(task, ref))
      .sort(
        (a, b) => (toMillis(b.created_at) ?? 0) - (toMillis(a.created_at) ?? 0),
      )

    const stepAgg = new Map<
      number,
      {
        items: WsTask[]
        behaviorTasks: TaskManagerTask[]
        startedAtMs: number
        endedAtMs: number
      }
    >()

    refs.forEach(({ task, ref }) => {
      const step = stepAgg.get(ref.step_index) ?? {
        items: [],
        behaviorTasks: [],
        startedAtMs: Number.MAX_SAFE_INTEGER,
        endedAtMs: 0,
      }
      const wsTask = wsTasks.find((item) => item.task_id === String(task.id))
      if (wsTask) step.items.push(wsTask)
      if (task.task_type === 'llm_behavior') step.behaviorTasks.push(task)

      const start = toMillis(task.created_at) ?? Date.now()
      const end = toMillis(task.updated_at) ?? start
      step.startedAtMs = Math.min(step.startedAtMs, start)
      if (end > step.endedAtMs) step.endedAtMs = end
      stepAgg.set(ref.step_index, step)
    })

    const steps = [...stepAgg.entries()]
      .sort((a, b) => a[0] - b[0])
      .map(([stepIndex, agg]) => {
        const latestBehavior = [...agg.behaviorTasks].sort(
          (a, b) => (toMillis(b.updated_at) ?? 0) - (toMillis(a.updated_at) ?? 0),
        )[0]
        const statusFromBehavior = latestBehavior
          ? taskStatusToStepStatus(String(latestBehavior.status))
          : undefined

        const fallbackStatus: StepStatus =
          agg.items.some((item) => item.status === 'running')
            ? 'running'
            : agg.items.some((item) => item.status === 'failed')
              ? 'failed'
              : 'success'
        const status = statusFromBehavior ?? fallbackStatus
        const endedAt =
          status === 'running' ? undefined : new Date(agg.endedAtMs || agg.startedAtMs).toISOString()

        const behaviorName = latestBehavior
          ? readString(latestBehavior.data, [['behavior']])
          : undefined
        const outputSnapshot = latestBehavior
          ? preview(readPath(latestBehavior.data, ['result']), 260) || undefined
          : undefined

        return {
          step_id: stepIdFromIndex(stepIndex),
          step_index: stepIndex,
          title: behaviorName ? `Behavior: ${behaviorName}` : undefined,
          status,
          started_at: new Date(agg.startedAtMs).toISOString(),
          ended_at: endedAt,
          duration:
            status === 'running'
              ? undefined
              : Math.max(0, Math.round((agg.endedAtMs - agg.startedAtMs) / 1000)),
          task_count: agg.items.length,
          log_counts: { message: 0, function_call: 0, action: 0, sub_agent: 0 },
          output_snapshot: outputSnapshot,
        }
      })

    const runStartMs = steps.length
      ? Math.min(...steps.map((step) => toMillis(step.started_at) ?? Date.now()))
      : Date.now()
    const runEndMs = steps
      .map((step) => toMillis(step.ended_at))
      .filter((value): value is number => value != null)
    const hasRunning = steps.some((step) => step.status === 'running')
    const hasFailed = steps.some((step) => step.status === 'failed')
    const hasCancelled = refs.some(
      ({ task }) => normalizeKey(String(task.status)) === 'canceled',
    )
    const runStatus: LoopRunStatus = hasRunning
      ? 'running'
      : hasFailed
        ? 'failed'
        : hasCancelled
          ? 'cancelled'
          : taskStatusToRunStatus(String(refs[refs.length - 1]?.task.status ?? 'Completed'))

    const currentStepIndex =
      [...steps]
        .reverse()
        .find((step) => step.status === 'running')?.step_index ??
      steps[steps.length - 1]?.step_index ??
      0

    const triggerBehavior = refs
      .map(({ task }) => extractTaskBehavior(task))
      .find((value): value is string => Boolean(value))
    const ownerSessionId = refs
      .map(({ task }) => extractTaskOwnerSessionId(task))
      .find((value): value is string => Boolean(value))
    const triggerEvent = triggerBehavior ? `behavior:${triggerBehavior}` : `wakeup:${runId}`

    const run: LoopRun = {
      run_id: runId,
      agent_id: agentId,
      trigger_event: triggerEvent,
      status: runStatus,
      started_at: new Date(runStartMs).toISOString(),
      ended_at: hasRunning
        ? undefined
        : new Date(Math.max(...(runEndMs.length > 0 ? runEndMs : [runStartMs]))).toISOString(),
      duration: hasRunning
        ? Math.max(0, Math.round((Date.now() - runStartMs) / 1000))
        : Math.max(
            0,
            Math.round(
              ((runEndMs.length > 0 ? Math.max(...runEndMs) : runStartMs) - runStartMs) / 1000,
            ),
          ),
      current_step_index: currentStepIndex,
      summary: {
        step_count: steps.length,
        task_count: wsTasks.length,
        log_count: 0,
        todo_count: 0,
        sub_agent_count: 0,
      },
    }

    runs.push(run)
    stepsByRun.set(runId, steps)
    tasksByRun.set(runId, wsTasks)
    runMetas.set(runId, {
      run_id: runId,
      agent_id: agentId,
      owner_session_id: ownerSessionId,
      started_at_ms: runStartMs,
      ended_at_ms: hasRunning ? undefined : Math.max(...(runEndMs.length > 0 ? runEndMs : [runStartMs])),
      step_ids: new Set(steps.map((step) => step.step_id)),
    })
  }

  runs.sort((a, b) => (toMillis(b.started_at) ?? 0) - (toMillis(a.started_at) ?? 0))
  return {
    runs,
    steps_by_run: stepsByRun,
    tasks_by_run: tasksByRun,
    run_metas: runMetas,
  }
}

const loadAgentRunData = async (agentId: string): Promise<AgentRunData> => {
  const cached = agentRunCache.get(agentId)
  if (cached && Date.now() - cached.at < AGENT_RUN_CACHE_TTL_MS) return cached.data

  const tasks = await loadAgentTasks(agentId)
  const data = buildAgentRunData(agentId, tasks)
  agentRunCache.set(agentId, { at: Date.now(), data })
  data.run_metas.forEach((meta, runId) => {
    runMetaCache.set(runId, meta)
  })
  return data
}

const findRunMeta = (runId: string): RunMeta | undefined => {
  const fromIndex = runMetaCache.get(runId)
  if (fromIndex) return fromIndex
  for (const item of agentRunCache.values()) {
    const meta = item.data.run_metas.get(runId)
    if (meta) return meta
  }
  return undefined
}

const mapOpenDanSession = (
  item: Partial<OpenDanAgentSession> & { session_id: string },
  agentId: string,
): WsAgentSession => ({
  session_id: item.session_id,
  owner_agent: item.owner_agent ?? agentId,
  title: item.title?.trim() || `Session ${item.session_id.slice(0, 8)}`,
  summary: item.summary?.trim() || undefined,
  status: item.status?.trim() || 'unknown',
  created_at: toIso(item.created_at_ms),
  updated_at: toIso(item.updated_at_ms),
  last_activity_at: toIso(item.last_activity_ms ?? item.updated_at_ms ?? item.created_at_ms),
})

const loadAgentSessions = async (agentId: string): Promise<WsAgentSession[]> => {
  const cached = agentSessionCache.get(agentId)
  if (cached && Date.now() - cached.at < AGENT_SESSION_CACHE_TTL_MS) return cached.sessions

  const listed = await opendanClient.listAgentSessions({
    agentId,
    limit: 200,
  })
  const sessionIds = listed.items.filter((item): item is OpenDanAgentSessionId => Boolean(item?.trim()))
  if (sessionIds.length === 0) {
    agentSessionCache.set(agentId, { at: Date.now(), sessions: [] })
    return []
  }

  const details = await Promise.all(
    sessionIds.map(async (sessionId) => {
      try {
        const record = await opendanClient.getAgentSession(agentId, sessionId)
        return mapOpenDanSession(record, agentId)
      } catch {
        return mapOpenDanSession({ session_id: sessionId }, agentId)
      }
    }),
  )

  details.sort((a, b) => (toMillis(b.last_activity_at) ?? 0) - (toMillis(a.last_activity_at) ?? 0))
  agentSessionCache.set(agentId, { at: Date.now(), sessions: details })
  return details
}

const resolveOwnerSessionId = async (
  agentId: string,
  preferredSessionId?: string,
): Promise<string | undefined> => {
  if (preferredSessionId?.trim()) return preferredSessionId.trim()
  const sessions = await loadAgentSessions(agentId)
  return sessions[0]?.session_id
}

const mapOpenDanWorklog = (item: OpenDanWorklog): WsWorkLog => {
  const payloadObj = asObject(item.payload) ?? undefined
  const durationMs =
    readNumber(payloadObj, [['duration_ms'], ['duration'], ['elapsed_ms']]) ??
    readNumber(item.payload, [['duration_ms'], ['duration'], ['elapsed_ms']])
  const duration = durationMs != null ? Math.max(0, Math.round(durationMs / 1000)) : undefined

  return {
    log_id: item.log_id,
    type: normalizeWorkLogType(item.log_type),
    agent_id: item.agent_id ?? '',
    related_agent_id: item.related_agent_id ?? undefined,
    step_id: item.step_id ?? undefined,
    status: normalizeWorkLogStatus(item.status),
    timestamp: toIso(item.timestamp),
    duration,
    summary: item.summary?.trim() || preview(item.payload) || item.log_type,
    payload: payloadObj,
  }
}

const loadRunWorklogs = async (runMeta: RunMeta): Promise<WsWorkLog[]> => {
  const cached = runWorklogCache.get(runMeta.run_id)
  if (cached && Date.now() - cached.at < RUN_WORKLOG_CACHE_TTL_MS) return cached.logs

  const ownerSessionId = await resolveOwnerSessionId(runMeta.agent_id, runMeta.owner_session_id)
  if (!ownerSessionId) {
    console.warn(`loadRunWorklogs skipped: missing ownerSessionId for agent ${runMeta.agent_id}`)
    runWorklogCache.set(runMeta.run_id, { at: Date.now(), logs: [] })
    return []
  }

  const result = await opendanClient.listWorkspaceWorklogs({
    agentId: runMeta.agent_id,
    ownerSessionId,
    limit: 500,
  })
  const startMs = runMeta.started_at_ms - 120_000
  const endMs = (runMeta.ended_at_ms ?? Date.now()) + 120_000

  const logs = result.items
    .map(mapOpenDanWorklog)
    .filter((log) => {
      const ts = toMillis(log.timestamp)
      if (ts != null && (ts < startMs || ts > endMs)) return false
      if (!log.step_id) return true
      return runMeta.step_ids.has(log.step_id)
    })
    .sort((a, b) => (toMillis(b.timestamp) ?? 0) - (toMillis(a.timestamp) ?? 0))

  runWorklogCache.set(runMeta.run_id, { at: Date.now(), logs })
  return logs
}

const applyTaskFilters = (tasks: WsTask[], filters?: WsTaskFilters): WsTask[] => {
  let filtered = tasks
  if (filters?.stepId) filtered = filtered.filter((task) => task.step_id === filters.stepId)
  if (filters?.status) filtered = filtered.filter((task) => task.status === filters.status)
  return filtered
}

const applyWorkLogFilters = (logs: WsWorkLog[], filters?: WsWorkLogFilters): WsWorkLog[] => {
  let filtered = logs
  if (filters?.stepId) filtered = filtered.filter((log) => log.step_id === filters.stepId)
  if (filters?.type) filtered = filtered.filter((log) => log.type === filters.type)
  if (filters?.status) filtered = filtered.filter((log) => log.status === filters.status)
  if (filters?.keyword) {
    const keyword = filters.keyword.toLowerCase()
    filtered = filtered.filter((log) => log.summary.toLowerCase().includes(keyword))
  }
  return filtered
}

const withStepLogCounts = (steps: WsStep[], logs: WsWorkLog[]): WsStep[] => {
  const counter = new Map<string, StepLogCounts>()

  for (const log of logs) {
    if (!log.step_id) continue
    const curr = counter.get(log.step_id) ?? { message: 0, function_call: 0, action: 0, sub_agent: 0 }
    if (log.type === 'message_sent' || log.type === 'message_reply') curr.message += 1
    else if (log.type === 'function_call') curr.function_call += 1
    else if (log.type === 'action') curr.action += 1
    else curr.sub_agent += 1
    counter.set(log.step_id, curr)
  }

  return steps.map((step) => ({
    ...step,
    log_counts: counter.get(step.step_id) ?? step.log_counts,
  }))
}

const mapOpenDanAgent = (item: OpenDanAgent): WsAgent => ({
  agent_id: item.agent_id,
  agent_name: item.agent_name?.trim() || item.agent_id,
  agent_type: normalizeAgentType(item.agent_type),
  status: normalizeAgentStatus(item.status),
  parent_agent_id: item.parent_agent_id ?? undefined,
  current_run_id: item.current_run_id ?? undefined,
  last_active_at: toIso(item.last_active_at ?? item.updated_at),
})

const mapOpenDanTodo = (item: OpenDanTodo, agentId: string): WsTodo => {
  const extra = asObject(item.extra)
  return {
    todo_id: item.todo_id,
    agent_id: item.agent_id ?? agentId,
    title: item.title,
    description: item.description ?? undefined,
    status: normalizeTodoStatus(item.status),
    created_at: toIso(item.created_at ?? readNumber(extra, [['created_at']])),
    completed_at:
      normalizeTodoStatus(item.status) === 'done'
        ? toIso(item.completed_at ?? readNumber(extra, [['completed_at']]))
        : undefined,
    created_in_step_id:
      item.created_in_step_id ??
      readString(extra, [['created_in_step_id'], ['created_step_id']]),
    completed_in_step_id:
      item.completed_in_step_id ??
      readString(extra, [['completed_in_step_id'], ['completed_step_id']]),
  }
}

const mapOpenDanSubAgent = (item: OpenDanSubAgent, parentAgentId: string): WsAgent => ({
  agent_id: item.agent_id,
  agent_name: item.agent_name?.trim() || item.agent_id,
  agent_type: 'sub',
  status: normalizeAgentStatus(item.status),
  parent_agent_id: parentAgentId,
  current_run_id: item.current_run_id ?? undefined,
  last_active_at: toIso(item.last_active_at),
})

export const fetchAgents = async (): Promise<{ data: WsAgent[] | null; error: unknown }> => {
  try {
    const result = await opendanClient.listAgents({
      includeSubAgents: true,
      limit: 200,
    })
    return { data: result.items.map(mapOpenDanAgent), error: null }
  } catch (error) {
    console.warn('fetchAgents failed, fallback to mock data', error)
    return { data: mockAgents, error }
  }
}

export const fetchAgentSessions = async (
  agentId: string,
): Promise<{ data: WsAgentSession[] | null; error: unknown }> => {
  try {
    const sessions = await loadAgentSessions(agentId)
    return { data: sessions, error: null }
  } catch (error) {
    console.warn('fetchAgentSessions failed, fallback to mock data', error)
    return { data: mockAgentSessions[agentId] ?? [], error }
  }
}

export const fetchLoopRuns = async (
  agentId: string,
): Promise<{ data: LoopRun[] | null; error: unknown }> => {
  try {
    const data = await loadAgentRunData(agentId)
    return { data: data.runs, error: null }
  } catch (error) {
    console.warn('fetchLoopRuns failed, fallback to mock data', error)
    return { data: mockLoopRuns[agentId] ?? [], error }
  }
}

export const fetchSteps = async (
  runId: string,
): Promise<{ data: WsStep[] | null; error: unknown }> => {
  try {
    const runMeta = findRunMeta(runId)
    if (!runMeta) throw new Error(`run not found: ${runId}`)
    const runData = await loadAgentRunData(runMeta.agent_id)
    const baseSteps = runData.steps_by_run.get(runId) ?? []
    let runLogs: WsWorkLog[] = []
    try {
      runLogs = await loadRunWorklogs(runMeta)
    } catch (error) {
      console.warn(`fetchSteps failed to load worklogs for run ${runId}, keep base steps`, error)
    }
    const steps = withStepLogCounts(baseSteps, runLogs)
    return { data: steps, error: null }
  } catch (error) {
    console.warn('fetchSteps failed, fallback to mock data', error)
    return { data: mockSteps[runId] ?? [], error }
  }
}

export const fetchWsTasks = async (
  runId: string,
  filters?: WsTaskFilters,
): Promise<{ data: WsTask[] | null; error: unknown }> => {
  try {
    const runMeta = findRunMeta(runId)
    if (!runMeta) throw new Error(`run not found: ${runId}`)
    const runData = await loadAgentRunData(runMeta.agent_id)
    const tasks = runData.tasks_by_run.get(runId) ?? []
    return { data: applyTaskFilters(tasks, filters), error: null }
  } catch (error) {
    console.warn('fetchWsTasks failed, fallback to mock data', error)
    return { data: applyTaskFilters(mockTasks[runId] ?? [], filters), error }
  }
}

export const fetchWorkLogs = async (
  runId: string,
  filters?: WsWorkLogFilters,
): Promise<{ data: WsWorkLog[] | null; error: unknown }> => {
  try {
    const runMeta = findRunMeta(runId)
    if (!runMeta) throw new Error(`run not found: ${runId}`)
    const logs = await loadRunWorklogs(runMeta)
    return { data: applyWorkLogFilters(logs, filters), error: null }
  } catch (error) {
    console.warn('fetchWorkLogs failed, fallback to mock data', error)
    return { data: applyWorkLogFilters(mockWorkLogs[runId] ?? [], filters), error }
  }
}

export const fetchTodos = async (
  agentId: string,
): Promise<{ data: WsTodo[] | null; error: unknown }> => {
  try {
    const ownerSessionId = await resolveOwnerSessionId(agentId)
    if (!ownerSessionId) throw new Error(`owner session not found for agent ${agentId}`)

    const result = await opendanClient.listWorkspaceTodos({
      agentId,
      ownerSessionId,
      includeClosed: true,
      limit: 200,
    })
    return { data: result.items.map((item) => mapOpenDanTodo(item, agentId)), error: null }
  } catch (error) {
    console.warn('fetchTodos failed, fallback to mock data', error)
    return { data: mockTodos[agentId] ?? [], error }
  }
}

export const fetchSubAgents = async (
  agentId: string,
): Promise<{ data: WsAgent[] | null; error: unknown }> => {
  try {
    const result = await opendanClient.listWorkspaceSubAgents({
      agentId,
      includeDisabled: true,
      limit: 200,
    })
    return {
      data: result.items.map((item) => mapOpenDanSubAgent(item, agentId)),
      error: null,
    }
  } catch (error) {
    console.warn('fetchSubAgents failed, fallback to mock data', error)
    const subs = mockAgents.filter((agent) => agent.parent_agent_id === agentId)
    return { data: subs, error }
  }
}
