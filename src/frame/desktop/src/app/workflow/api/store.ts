/* ── Workflow WebUI store backed by the workflow service RPC ── */

import { buckyos, WorkflowRequestError } from 'buckyos'
import type { WorkflowClient, WorkflowDefinition as DslDefinition } from 'buckyos'
import type {
  AnalysisReport,
  ExecutorEntry,
  WorkflowDefinition,
  WorkflowRunDetail,
  WorkflowRunSummary,
} from '../types'
import {
  isTerminalStatus,
  mapAnalysis,
  mapDefinition,
  mapFunction,
  mapRunDetail,
  mapRunSummary,
} from './mapping'

export interface DryRunOutcome {
  ok: boolean
  analysis: AnalysisReport
  revision?: string
  message?: string
}

const RUN_POLL_MS = 2000
const SCHEMA_VERSION = '0.2.0'

export class WorkflowStore {
  definitions: WorkflowDefinition[] = []
  runs: WorkflowRunSummary[] = []
  executors: ExecutorEntry[] = []
  runDetails = new Map<string, WorkflowRunDetail>()
  /** run id -> TaskMgr root task id (`wf-run-<run_id>` idempotency key). */
  rootTaskIds = new Map<string, string>()
  schemaVersion = SCHEMA_VERSION
  loading = false
  error: string | null = null
  private version = 0
  private listeners = new Set<() => void>()
  private pollers = new Map<string, ReturnType<typeof setInterval>>()
  private client: WorkflowClient | null = null

  getSnapshot = (): number => this.version

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }

  dispose(): void {
    for (const timer of this.pollers.values()) clearInterval(timer)
    this.pollers.clear()
    this.listeners.clear()
  }

  private emitChange(): void {
    this.version += 1
    this.listeners.forEach((listener) => listener())
  }

  private rpc(): WorkflowClient {
    if (!this.client) this.client = buckyos.getWorkflowClient()
    return this.client
  }

  async load(): Promise<void> {
    this.loading = true
    this.error = null
    this.emitChange()
    try {
      const [summaries, runs, functions] = await Promise.all([
        this.rpc().listDefinitions(),
        this.rpc().listRuns(),
        this.rpc().listFunctions(),
      ])
      const records = await Promise.all(summaries.map((s) => this.rpc().getDefinition(s.id)))
      this.definitions = records.map(mapDefinition)
      this.definitions.sort((a, b) => (a.source === 'system' ? 0 : 1) - (b.source === 'system' ? 0 : 1) || a.name.localeCompare(b.name))
      this.runs = runs.map(mapRunSummary).sort((a, b) => b.updatedAt.localeCompare(a.updatedAt))
      this.executors = functions.map(mapFunction)
    } catch (err) {
      this.error = err instanceof Error ? err.message : String(err)
    } finally {
      this.loading = false
      this.emitChange()
    }
  }

  async refreshRuns(): Promise<void> {
    try {
      const runs = await this.rpc().listRuns()
      this.runs = runs.map(mapRunSummary).sort((a, b) => b.updatedAt.localeCompare(a.updatedAt))
      this.emitChange()
    } catch (err) {
      this.error = err instanceof Error ? err.message : String(err)
      this.emitChange()
    }
  }

  listDefinitions(): WorkflowDefinition[] {
    return this.definitions
  }

  getDefinition(id: string): WorkflowDefinition | undefined {
    return this.definitions.find((d) => d.id === id)
  }

  listRuns(): WorkflowRunSummary[] {
    return this.runs
  }

  listRunsForDefinition(definitionId: string, limit = 50): WorkflowRunSummary[] {
    return this.runs.filter((r) => r.definitionId === definitionId).slice(0, limit)
  }

  getRunDetail(runId: string): WorkflowRunDetail | undefined {
    return this.runDetails.get(runId)
  }

  /** Loads the run graph and keeps polling while the run is not terminal. */
  async watchRun(runId: string): Promise<void> {
    await this.loadRunDetail(runId)
    if (this.pollers.has(runId)) return
    const timer = setInterval(() => {
      const detail = this.runDetails.get(runId)
      if (detail?.terminal) {
        this.unwatchRun(runId)
        return
      }
      void this.loadRunDetail(runId)
    }, RUN_POLL_MS)
    this.pollers.set(runId, timer)
  }

  unwatchRun(runId: string): void {
    const timer = this.pollers.get(runId)
    if (timer) clearInterval(timer)
    this.pollers.delete(runId)
  }

  getRootTaskId(runId: string): string | undefined {
    return this.rootTaskIds.get(runId)
  }

  private async resolveRootTask(runId: string): Promise<void> {
    if (this.rootTaskIds.has(runId)) return
    try {
      const page = await buckyos.getTaskManagerClient().listTasks({ idempotency_key: `wf-run-${runId}` })
      const root = page.tasks.find((t) => t.schema_id === 'workflow.run_tree/v1') ?? page.tasks[0]
      if (root) {
        this.rootTaskIds.set(runId, root.task_id)
        this.emitChange()
      }
    } catch {
      // TaskMgr unavailable: the run page falls back to a plain Task Center link.
    }
  }

  async loadRunDetail(runId: string): Promise<WorkflowRunDetail | undefined> {
    void this.resolveRootTask(runId)
    try {
      const graph = await this.rpc().getRunGraph(runId)
      const detail = mapRunDetail(graph)
      this.runDetails.set(runId, detail)
      const index = this.runs.findIndex((r) => r.runId === runId)
      if (index !== -1 && this.runs[index].status !== detail.status) {
        this.runs[index] = { ...this.runs[index], status: detail.status, updatedAt: detail.updatedAt }
      }
      this.emitChange()
      return detail
    } catch (err) {
      this.error = err instanceof Error ? err.message : String(err)
      this.emitChange()
      return undefined
    }
  }

  async cancelRun(runId: string, reason?: string): Promise<string | null> {
    try {
      await this.rpc().cancelRun({ run_id: runId, reason })
      await this.loadRunDetail(runId)
      await this.refreshRuns()
      return null
    } catch (err) {
      return err instanceof Error ? err.message : String(err)
    }
  }

  async dryRun(definition: DslDefinition): Promise<DryRunOutcome> {
    try {
      const result = await this.rpc().dryRun(definition)
      return { ok: true, analysis: mapAnalysis(result.analysis), revision: result.revision }
    } catch (err) {
      const detail = err instanceof WorkflowRequestError ? err.detail : undefined
      const analysis = detail && typeof detail === 'object' && 'errors' in (detail as object)
        ? mapAnalysis(detail as never)
        : mapAnalysis(undefined)
      return {
        ok: false,
        analysis,
        message: err instanceof Error ? err.message : String(err),
      }
    }
  }

  async submitDefinition(definition: DslDefinition): Promise<{ definitionId: string } | { error: string }> {
    try {
      const result = await this.rpc().submitDefinition({ definition, source: 'user_imported' })
      const mapped = mapDefinition(result.definition)
      const index = this.definitions.findIndex((d) => d.id === mapped.id)
      if (index === -1) this.definitions.unshift(mapped)
      else this.definitions[index] = mapped
      this.emitChange()
      return { definitionId: mapped.id }
    } catch (err) {
      return { error: err instanceof Error ? err.message : String(err) }
    }
  }

  async archiveDefinition(definitionId: string): Promise<string | null> {
    try {
      const status = await this.rpc().archiveDefinition(definitionId)
      const index = this.definitions.findIndex((d) => d.id === definitionId)
      if (index !== -1) this.definitions[index] = { ...this.definitions[index], status }
      this.emitChange()
      return null
    } catch (err) {
      return err instanceof Error ? err.message : String(err)
    }
  }

  activeRunCount(): number {
    return this.runs.filter((r) => !isTerminalStatus(r.status)).length
  }

  buildAiPrompt(): string {
    const exec = this.executors
      .map((e) => `- ${e.id} :: ${e.description} (in: ${e.inputSummary ?? '?'}; out: ${e.outputSummary ?? '?'})`)
      .join('\n')
    return [
      `You are generating a BuckyOS Workflow definition (DSL JSON).`,
      `schema_version: ${this.schemaVersion}`,
      ``,
      `Top-level fields: id, name, schema_version, trigger, steps, nodes?, edges, guards?, defs?.`,
      ``,
      `Step fields: id, name, type ("autonomous" | "human_confirm" | "human_required"), executor, input, input_schema?, output_schema, output_mode ("single" | "finite_seekable" | "finite_sequential"), idempotent, skippable, subject_ref?, prompt?, guards?.`,
      `Control nodes (in "nodes"):`,
      `  * {type:"branch", id, on, paths, max_iterations}  // paths is exhaustive enum -> nodeId map`,
      `  * {type:"parallel", id, branches[], join: "all" | "any" | "n_of_m", n?}`,
      `  * {type:"for_each", id, items, steps[], max_items, concurrency}  // upstream finite_sequential forces concurrency=1`,
      ``,
      `Edges: { from, to }; an edge without "to" marks a terminal node. branch.paths and parallel.branches imply edges; do not duplicate them.`,
      ``,
      `References (in step input) use:  \${node_id.output[.field[.sub]]}; \${trigger.output.<field>} reads the run input declared in trigger.input_schema.`,
      `No expressions, function calls, or string concatenation in references.`,
      ``,
      `Executors: "/tool/<name>@<version>" binds a published FunctionObject; "service::<svc>.<method>" calls a service adapter.`,
      `Published functions in this zone:`,
      exec,
      ``,
      `Output requirements: emit ONE valid JSON object (no commentary, no markdown fences). Conform exactly to schema_version ${this.schemaVersion}.`,
    ].join('\n')
  }
}
