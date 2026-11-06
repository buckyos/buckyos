/* ── RPC record → view model mapping ── */

import type {
  WorkflowAnalysisIssue,
  WorkflowAnalysisReport,
  WorkflowCompiledNode,
  WorkflowDefinition as DslDefinition,
  WorkflowDefinitionRecord,
  WorkflowFunctionEntry,
  WorkflowRunGraph,
  WorkflowRunSummary as SdkRunSummary,
  WorkflowStepDefinition,
} from 'buckyos'
import type {
  AnalysisIssue,
  AnalysisReport,
  ControlNodeView,
  ExecutorEntry,
  ExecutorNamespace,
  NodeGuards,
  NodeRunState,
  RunStatus,
  RunStepView,
  TaskNodeView,
  WorkflowDefinition,
  WorkflowGraphEdge,
  WorkflowGraphNode,
  WorkflowRunDetail,
  WorkflowRunSummary,
} from '../types'

const TERMINAL: RunStatus[] = ['completed', 'failed', 'aborted', 'budget_exhausted']

export function isTerminalStatus(status: RunStatus): boolean {
  return TERMINAL.includes(status)
}

function isoFromUnix(seconds: number | undefined): string {
  if (!seconds) return new Date(0).toISOString()
  return new Date(seconds * 1000).toISOString()
}

function issue(severity: AnalysisIssue['severity'], raw: WorkflowAnalysisIssue): AnalysisIssue {
  return {
    severity,
    code: raw.code,
    message: raw.message,
    nodeId: raw.node_id ?? undefined,
  }
}

export function mapAnalysis(report: WorkflowAnalysisReport | undefined): AnalysisReport {
  const errors = (report?.errors ?? []).map((r) => issue('error', r))
  const warnings = (report?.warnings ?? []).map((r) => issue('warn', r))
  return {
    issues: [...errors, ...warnings],
    errorCount: errors.length,
    warnCount: warnings.length,
    infoCount: 0,
  }
}

export function parseReference(raw: string): { nodeId: string; fieldPath: string[] } | undefined {
  if (!raw.startsWith('${') || !raw.endsWith('}')) return undefined
  const parts = raw.slice(2, -1).split('.')
  if (parts.length < 2 || parts[1] !== 'output' || !parts[0]) return undefined
  return { nodeId: parts[0], fieldPath: parts.slice(2) }
}

function namespaceOf(executor: string): ExecutorNamespace | undefined {
  const ns = executor.split('::')[0]
  if (ns === 'service' || ns === 'http' || ns === 'appservice' || ns === 'operator' || ns === 'func') {
    return ns
  }
  return undefined
}

function mapGuards(step: WorkflowStepDefinition): NodeGuards | undefined {
  const g = step.guards
  if (!g) return undefined
  const guards: NodeGuards = {}
  if (g.budget) {
    guards.budget = {
      maxTokens: g.budget.max_tokens ?? undefined,
      maxCostUsdb: g.budget.max_cost_usdb ?? undefined,
      maxDuration: g.budget.max_duration ?? undefined,
    }
  }
  if (g.permissions && g.permissions.length > 0) guards.permissions = g.permissions
  if (g.retry) {
    guards.retry = {
      maxAttempts: g.retry.max_attempts ?? 1,
      backoff: g.retry.backoff === 'exponential' ? 'exponential' : 'fixed',
      fallback: g.retry.fallback === 'abort' ? 'abort' : 'human',
    }
  }
  if (g.timeout) guards.timeout = g.timeout
  return guards
}

function mapStep(
  step: WorkflowStepDefinition,
  resolved: Record<string, string>,
): TaskNodeView {
  const bindings: TaskNodeView['inputBindings'] = []
  if (step.input && typeof step.input === 'object' && !Array.isArray(step.input)) {
    for (const [field, value] of Object.entries(step.input as Record<string, unknown>)) {
      const ref = typeof value === 'string' ? parseReference(value) : undefined
      bindings.push(ref ? { kind: 'reference', field, ...ref } : { kind: 'literal', field, value })
    }
  }
  const funId = resolved[step.id]
  const executor = step.executor
    ? {
        raw: step.executor,
        resolvedNamespace: funId ? ('func' as const) : namespaceOf(step.executor),
        resolvedTarget: funId ? `func::${funId}` : step.executor.includes('::') ? step.executor : undefined,
      }
    : undefined
  return {
    kind: 'task',
    id: step.id,
    name: step.name,
    stepType: step.type,
    executor,
    inputSchema: step.input_schema,
    outputSchema: step.output_schema,
    outputMode: step.output_mode ?? 'single',
    idempotent: step.idempotent ?? true,
    skippable: step.skippable ?? true,
    subjectRef: step.subject_ref ? parseReference(step.subject_ref) : undefined,
    prompt: step.prompt,
    guards: mapGuards(step),
    inputBindings: bindings,
  }
}

function actualConcurrency(compiled: WorkflowCompiledNode | undefined): number | undefined {
  const expr = compiled?.expr as { Map?: { actual_concurrency?: number } } | undefined
  return expr?.Map?.actual_concurrency
}

export function mapGraph(
  definition: DslDefinition,
  compiledNodes: Record<string, WorkflowCompiledNode>,
  resolved: Record<string, string>,
  version: number,
): WorkflowDefinition['graph'] {
  const nodes: WorkflowGraphNode[] = definition.steps.map((s) => mapStep(s, resolved))
  const edges: WorkflowGraphEdge[] = []
  let edgeSeq = 0
  const edge = (source: string, target: string | null, implicit?: boolean, conditionLabel?: string) => {
    edgeSeq += 1
    edges.push({ id: `e${edgeSeq}`, source, target, implicit, conditionLabel })
  }
  for (const node of definition.nodes ?? []) {
    if (node.type === 'branch') {
      const view: ControlNodeView = {
        kind: 'control',
        controlType: 'branch',
        id: node.id,
        name: node.id,
        on: parseReference(node.on) ?? { nodeId: node.on, fieldPath: [] },
        paths: node.paths,
        maxIterations: node.max_iterations,
      }
      nodes.push(view)
      for (const [label, target] of Object.entries(node.paths)) edge(node.id, target, true, label)
    } else if (node.type === 'parallel') {
      nodes.push({
        kind: 'control',
        controlType: 'parallel',
        id: node.id,
        name: node.id,
        branches: node.branches,
        join: { strategy: node.join, n: node.n ?? undefined },
      })
      for (const branch of node.branches) edge(node.id, branch, true)
    } else {
      const concurrency = node.concurrency ?? 1
      const effective = actualConcurrency(compiledNodes[node.id]) ?? concurrency
      nodes.push({
        kind: 'control',
        controlType: 'for_each',
        id: node.id,
        name: node.id,
        items: parseReference(node.items) ?? { nodeId: node.items, fieldPath: [] },
        steps: node.steps,
        maxItems: node.max_items,
        concurrency,
        effectiveConcurrency: effective,
        degradedReason:
          effective < concurrency
            ? `upstream output is finite_sequential; concurrency capped to ${effective}`
            : undefined,
      })
      if (node.steps[0]) edge(node.id, node.steps[0], true)
      for (let i = 1; i < node.steps.length; i += 1) edge(node.steps[i - 1], node.steps[i], true)
    }
  }
  for (const e of definition.edges) edge(e.from, e.to ?? null)
  return {
    definitionId: definition.id,
    definitionVersion: version,
    schemaVersion: definition.schema_version,
    nodes,
    edges,
  }
}

export function mapDefinition(record: WorkflowDefinitionRecord): WorkflowDefinition {
  return {
    id: record.id,
    schemaVersion: record.schema_version,
    name: record.name,
    description: record.definition.description ?? undefined,
    version: record.version,
    revision: record.revision,
    source: record.source,
    readOnly: record.read_only,
    status: record.status,
    owner: { userId: record.owner.user_id, appId: record.owner.app_id },
    analysis: mapAnalysis(record.analysis),
    graph: mapGraph(record.definition, record.compiled.nodes, record.resolved_functions, record.version),
    resolvedFunctions: record.resolved_functions,
    createdAt: isoFromUnix(record.created_at),
    updatedAt: isoFromUnix(record.updated_at),
    tags: record.tags,
  }
}

export function mapRunSummary(run: SdkRunSummary): WorkflowRunSummary {
  return {
    runId: run.run_id,
    definitionId: run.workflow_id,
    definitionName: run.workflow_name,
    revision: run.revision,
    status: run.status,
    requestedBy: run.requested_by ?? undefined,
    owner: { userId: run.owner.user_id, appId: run.owner.app_id },
    startedAt: isoFromUnix(run.created_at),
    updatedAt: isoFromUnix(run.updated_at),
    progress: run.progress,
  }
}

export function mapRunDetail(graph: WorkflowRunGraph): WorkflowRunDetail {
  const pendingByNode: Record<string, string[]> = {}
  for (const [executionId, pending] of Object.entries(graph.pending_thunks ?? {})) {
    const nodeId = (pending as { node_id?: string }).node_id
    if (!nodeId) continue
    ;(pendingByNode[nodeId] ??= []).push(executionId)
  }
  const steps: RunStepView[] = Object.entries(graph.nodes ?? {}).map(([nodeId, node]) => ({
    nodeId,
    name: node.name,
    state: (graph.node_states[nodeId] ?? 'pending') as NodeRunState,
    attempt: graph.node_attempts?.[nodeId] ?? 0,
    source: graph.node_sources?.[nodeId],
    error: graph.node_errors?.[nodeId],
    output: graph.node_outputs?.[nodeId],
    pendingExecutionIds: pendingByNode[nodeId] ?? [],
  }))
  const order = graph.graph?.start_nodes ?? []
  steps.sort((a, b) => {
    const ia = order.indexOf(a.nodeId)
    const ib = order.indexOf(b.nodeId)
    if (ia !== -1 || ib !== -1) return (ia === -1 ? 1 : 0) - (ib === -1 ? 1 : 0)
    return 0
  })
  const runError = (graph.metrics?.run_error as RunStepView['error'] | undefined) ?? undefined
  return {
    runId: graph.run_id,
    definitionId: graph.workflow_id,
    definitionName: graph.workflow_name,
    revision: graph.revision,
    status: graph.status,
    terminal: isTerminalStatus(graph.status),
    requestedBy: graph.requested_by ?? undefined,
    executor: graph.executor,
    owner: { userId: graph.owner.user_id, appId: graph.owner.app_id },
    steps,
    humanWaitingNodes: graph.human_waiting_nodes ?? [],
    output: graph.output ?? {},
    runError,
    deadlineAt: graph.deadline_at ?? undefined,
    executionEpoch: graph.execution_epoch,
    createdAt: isoFromUnix(graph.created_at),
    updatedAt: isoFromUnix(graph.updated_at),
    seq: graph.seq,
  }
}

function summarizeSchema(schema: unknown): string | undefined {
  if (!schema || typeof schema !== 'object') return undefined
  const props = (schema as { properties?: Record<string, unknown> }).properties
  if (!props) return undefined
  const keys = Object.keys(props)
  return keys.length > 0 ? `{ ${keys.join(', ')} }` : undefined
}

export function mapFunction(entry: WorkflowFunctionEntry): ExecutorEntry {
  return {
    id: `/tool/${entry.registration.name}@${entry.registration.version}`,
    namespace: 'func',
    description: entry.registration.description || `${entry.registration.name} (${entry.fun_id})`,
    inputSummary: summarizeSchema(entry.object.params_schema),
    outputSummary: summarizeSchema(entry.object.result_schema),
  }
}
