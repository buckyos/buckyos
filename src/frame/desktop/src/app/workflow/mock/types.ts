/* ── Workflow WebUI mock types ── */

export type DefinitionStatus = 'draft' | 'active' | 'archived'
export type DefinitionSource =
  | 'system'
  | 'user_imported'
  | 'app_registered'
  | 'agent_generated'

export type IssueSeverity = 'error' | 'warn' | 'info'

export interface AnalysisIssue {
  severity: IssueSeverity
  code: string
  message: string
  nodeId?: string
  path?: string
}

export interface AnalysisReport {
  issues: AnalysisIssue[]
  errorCount: number
  warnCount: number
  infoCount: number
}

export type StepType = 'autonomous' | 'human_confirm' | 'human_required'
export type OutputMode = 'single' | 'finite_seekable' | 'finite_sequential'
export type ExecutorNamespace =
  | 'service'
  | 'http'
  | 'appservice'
  | 'operator'
  | 'func'

export interface NodeGuards {
  budget?: { maxTokens?: number; maxCostUsdb?: number; maxDuration?: string }
  permissions?: string[]
  retry?: {
    maxAttempts: number
    backoff: 'fixed' | 'exponential'
    fallback: 'human' | 'abort'
  }
  timeout?: string
}

export interface TaskNodeView {
  kind: 'task'
  id: string
  name: string
  description?: string
  stepType: StepType
  executor?: {
    raw: string
    resolvedNamespace?: ExecutorNamespace
    resolvedTarget?: string
  }
  inputSchema?: unknown
  outputSchema?: unknown
  outputMode: OutputMode
  idempotent: boolean
  skippable: boolean
  subjectRef?: { nodeId: string; fieldPath: string[] }
  prompt?: string
  guards?: NodeGuards
  inputBindings: Array<
    | { kind: 'literal'; field: string; value: unknown }
    | { kind: 'reference'; field: string; nodeId: string; fieldPath: string[] }
  >
}

export interface BranchControlNodeView {
  kind: 'control'
  controlType: 'branch'
  id: string
  name: string
  on: { nodeId: string; fieldPath: string[] }
  paths: Record<string, string>
  maxIterations?: number
}

export interface ParallelControlNodeView {
  kind: 'control'
  controlType: 'parallel'
  id: string
  name: string
  branches: string[]
  join: { strategy: 'all' | 'any' | 'n_of_m'; n?: number }
}

export interface ForEachControlNodeView {
  kind: 'control'
  controlType: 'for_each'
  id: string
  name: string
  items: { nodeId: string; fieldPath: string[] }
  steps: string[]
  maxItems: number
  concurrency: number
  effectiveConcurrency: number
  degradedReason?: string
}

export type ControlNodeView =
  | BranchControlNodeView
  | ParallelControlNodeView
  | ForEachControlNodeView

export type WorkflowGraphNode = TaskNodeView | ControlNodeView

export interface WorkflowGraphEdge {
  id: string
  source: string
  target: string | null
  implicit?: boolean
  conditionLabel?: string
}

export interface WorkflowGraphView {
  definitionId: string
  definitionVersion: number
  schemaVersion: string
  nodes: WorkflowGraphNode[]
  edges: WorkflowGraphEdge[]
}

export interface WorkflowDefinition {
  id: string
  schemaVersion: string
  name: string
  description?: string
  version: number
  source: DefinitionSource
  status: DefinitionStatus
  analysis: AnalysisReport
  graph: WorkflowGraphView
  createdAt: string
  updatedAt: string
  tags?: string[]
}

export interface AppWorkflowMountPoint {
  id: string
  appId: string
  name: string
  description?: string
  required: boolean
  allowEmpty: boolean
  defaultDefinitionId?: string
  currentBinding?: {
    definitionId: string
    definitionVersion: number
    boundAt: string
    boundBy: string
  }
}

export type AppKind = 'app' | 'script_app'

export interface WorkflowApp {
  id: string
  name: string
  kind: AppKind
  description?: string
  mountPoints: AppWorkflowMountPoint[]
}

export type RunStatus =
  | 'created'
  | 'running'
  | 'waiting_human'
  | 'completed'
  | 'failed'
  | 'paused'
  | 'aborted'
  | 'budget_exhausted'

export interface WorkflowRunSummary {
  runId: string
  rootTaskId: string
  definitionId: string
  definitionVersion: number
  planVersion: number
  status: RunStatus
  triggerSource: 'app' | 'manual' | 'agent' | 'system'
  appId?: string
  mountPointId?: string
  humanWaitingNodes: string[]
  startedAt: string
  finishedAt?: string
  durationMs?: number
  errorSummary?: string
  taskmgrUrl: string
}

export interface AmendmentSummary {
  runId: string
  planVersion: number
  submittedBy: string
  submittedAtStep: string
  approvalStatus: 'pending' | 'approved' | 'rejected'
  reason?: string
  operations: Array<{ op: string; target?: string; description?: string }>
}

export interface ExecutorEntry {
  id: string
  namespace: ExecutorNamespace
  description: string
  inputSummary?: string
  outputSummary?: string
}

/* ── Selection model used by the shell ── */

export type WorkflowSelection =
  | { kind: 'home' }
  | { kind: 'definition'; definitionId: string }
  | { kind: 'mount'; appId: string; mountPointId: string }
  | { kind: 'ai_prompt' }
