/* ── Workflow WebUI view types (mapped from workflow service RPC records) ── */

import type {
  WorkflowDefinitionSource as SdkDefinitionSource,
  WorkflowExecutionError,
  WorkflowNodeSource,
} from 'buckyos'

export type DefinitionStatus = 'draft' | 'active' | 'archived'
export type DefinitionSource = SdkDefinitionSource

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
  revision: string
  source: DefinitionSource
  readOnly: boolean
  status: DefinitionStatus
  owner: { userId: string; appId: string }
  analysis: AnalysisReport
  graph: WorkflowGraphView
  resolvedFunctions: Record<string, string>
  createdAt: string
  updatedAt: string
  tags?: string[]
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

export type NodeRunState =
  | 'pending'
  | 'ready'
  | 'running'
  | 'completed'
  | 'failed'
  | 'retrying'
  | 'waiting_human'
  | 'skipped'
  | 'aborted'
  | 'cancelled'

export type StepSource = WorkflowNodeSource
export type StepError = WorkflowExecutionError

export interface WorkflowRunSummary {
  runId: string
  definitionId: string
  definitionName: string
  revision: string
  status: RunStatus
  requestedBy?: string
  owner: { userId: string; appId: string }
  startedAt: string
  updatedAt: string
  progress: number
}

export interface RunStepView {
  nodeId: string
  name: string
  state: NodeRunState
  attempt: number
  source?: StepSource
  error?: StepError
  output?: unknown
  pendingExecutionIds: string[]
}

export interface WorkflowRunDetail {
  runId: string
  definitionId: string
  definitionName: string
  revision: string
  status: RunStatus
  terminal: boolean
  requestedBy?: string
  executor: string
  owner: { userId: string; appId: string }
  steps: RunStepView[]
  humanWaitingNodes: string[]
  output: Record<string, unknown>
  runError?: StepError
  deadlineAt?: number
  executionEpoch: number
  createdAt: string
  updatedAt: string
  seq: number
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
  | { kind: 'run'; runId: string }
  | { kind: 'ai_prompt' }
