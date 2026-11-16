/* The local executor (内核架构改进 §9, D14, T3-4; 浏览器优先; AI Native 第一期 §2, 余下任务 A): this browser executes a
 * wish run it started — analyze, code or run. It claims the run, keeps the lease alive, runs a model stage on xllm
 * (`XllmTask.prepareHosted` with the stage's system text and no xllm sections, the stage's tools as the page tools of
 * the site runtime `aiws.wish`, every call going back to the service), runs the macro in a Worker on the snapshot and
 * answers its bridge with host.js (SQL through the service, AI with the user's own session, Zone resources with the
 * page's), and reports when done, with what the stage's model loop used. The service validates, plans and writes the
 * stage's outcome exactly as for any other executor; it calls no model.
 *
 * The claim token of a run is kept in this browser (localStorage) so a reload can claim it again: a run whose lease
 * lapsed is `interrupted` and resumes as a new attempt. The service opens a new attempt's stage afresh (余下任务 §7.1
 * 4: nothing the previous attempt staged is kept), so its loop starts from the beginning; what the earlier attempts
 * used in this browser is kept beside the claim and reported with the stage. */

import { MemoryRunStore, XllmRun, XllmTask, type AiResponse, type JsonValue, type LlmClient, type LlmInferenceRequest, type RunEvent, type RunOutcome } from 'buckyos/xllm'
import { createBridgeHost } from '../../../../../aiworkspace/aiws/host.js'
import type { Json, StagedAsset, WishBudget, WishClaim, WishStageBrief, WishStageFiles, WishToolAnswer } from '../api/types'
import { aiwsT } from '../i18n'
import { aiccCall, aiccTask, refusalCode, stageLlm, type Infer } from './infer'
import { runProgram } from './programHost'
import { wishRuntime } from './wishRuntime'

const CLAIMS_KEY = 'aiworkspace.claims'
const USAGE_KEY = 'aiworkspace.claims.usage'
const DEFAULT_ITERATIONS = 60
const DEFAULT_TIMEOUT_SECS = 120
/** The loop's own limits (余下任务 §7.1): the brief has no stage duration, the lease and the iterations bound it. */
const STAGE_TIMEOUT_SECS = 21600
const LLM_TIMEOUT_SECS = 900
const CONTEXT_WINDOW = 200000
/** A model call that failed for a passing reason (the provider, a timeout) is tried again this many times. */
const TRANSIENT_RETRIES = 2
const TRANSIENT_KINDS = ['provider_transient', 'provider_timeout']

/** Claim tokens of the runs this browser started, by run id. */
export const claims = {
  all(): Record<string, string> {
    try { return JSON.parse(window.localStorage.getItem(CLAIMS_KEY) ?? '{}') as Record<string, string> } catch { return {} }
  },
  get(runId: string): string | null { return this.all()[runId] ?? null },
  set(runId: string, token: string) {
    const all = this.all()
    all[runId] = token
    const ids = Object.keys(all)
    for (const id of ids.slice(0, Math.max(0, ids.length - 50))) delete all[id]
    try { window.localStorage.setItem(CLAIMS_KEY, JSON.stringify(all)) } catch { /* a private window: the run cannot be resumed after a reload */ }
  },
  forget(runId: string) {
    const all = this.all()
    delete all[runId]
    try { window.localStorage.setItem(CLAIMS_KEY, JSON.stringify(all)) } catch { /* nothing to forget */ }
    claimUsage.set(runId, null)
  },
}

/** What the earlier attempts of a run used in this browser, beside its claim. */
const claimUsage = {
  all(): Record<string, RunUsage> {
    try { return JSON.parse(window.localStorage.getItem(USAGE_KEY) ?? '{}') as Record<string, RunUsage> } catch { return {} }
  },
  get(runId: string): RunUsage | null { return this.all()[runId] ?? null },
  set(runId: string, usage: RunUsage | null) {
    const all = this.all()
    if (usage) all[runId] = usage
    else delete all[runId]
    const ids = Object.keys(all)
    for (const id of ids.slice(0, Math.max(0, ids.length - 50))) delete all[id]
    try { window.localStorage.setItem(USAGE_KEY, JSON.stringify(all)) } catch { /* a private window: only this attempt is counted */ }
  },
}

/** The stage's model, counting what its turns used (every call: the loop's and a compaction's). */
class CountingLlm implements LlmClient {
  private readonly inner: LlmClient
  private readonly onTurn: (response: AiResponse, model: string) => void

  constructor(inner: LlmClient, onTurn: (response: AiResponse, model: string) => void) {
    this.inner = inner
    this.onTurn = onTurn
  }

  async infer(req: LlmInferenceRequest): Promise<AiResponse> {
    const response = await this.inner.infer(req)
    const model = (response.extra as { model?: unknown } | undefined)?.model
    this.onTurn(response, typeof model === 'string' && model ? model : req.model_alias)
    return response
  }
}

/** What a model stage used (`proc.report { usage }`): the model of its last turn (the stage's logical model when the
 * provider did not say), the tokens of all its turns, how many turns. A macro's own AI calls are counted by the program
 * host (`results.ai`). */
export type RunUsage = { model?: string; input_tokens?: number; output_tokens?: number; calls?: number }
/** How an executor ends a run: done (with what the model said when it stopped: a coding that ends without `finish` fails
 * of it), or failed with the reason; either with what the stage used. */
export type RunReport = { finished: true; summary?: string; usage?: RunUsage } | { error: { code: string; detail: string }; usage?: RunUsage }

export interface ExecutorSession {
  procClaim(runId: string, claimToken: string): Promise<WishClaim>
  procHeartbeat(runId: string, attempt: number, progress?: Json): Promise<void>
  procReport(runId: string, attempt: number, body: RunReport): Promise<unknown>
  procTool(runId: string, attempt: number, name: string, args: Json): Promise<WishToolAnswer>
  procProgram(runId: string, attempt: number, body: { source: string; results: Json | null; log: string; files: Json }): Promise<WishToolAnswer>
  procFiles(runId: string, attempt: number): Promise<WishStageFiles>
  importObject(objId: string): Promise<StagedAsset>
}

/** The Zone a macro may reach from this page (第一期 §2.6): this page's host and the hosts under it. */
function zoneHosts(): string[] {
  return [window.location.hostname]
}

async function zoneFetch(url: string, init: { method: string; headers: Record<string, string>; body: string | null }) {
  const res = await fetch(url, { method: init.method, headers: init.headers, body: init.body ?? undefined, credentials: 'include' })
  const bytes = new Uint8Array(await res.arrayBuffer())
  const headers: Record<string, string> = {}
  res.headers.forEach((v, k) => { headers[k] = v })
  let text: string | null = null
  try { text = new TextDecoder('utf-8', { fatal: true }).decode(bytes) } catch { text = null }
  if (text !== null) return { status: res.status, headers, text }
  let bin = ''
  for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000))
  return { status: res.status, headers, base64: btoa(bin) }
}

function codeOf(error: unknown): string | undefined {
  return (error as { code?: string } | null)?.code
}

export class LocalExecutor {
  private readonly abort = new AbortController()
  private attempt = 0
  private files: WishStageFiles | null = null
  private phase = 'claimed'
  /** What the model said when it stopped. */
  private said = ''
  private budget: Partial<WishBudget> = {}
  private manifest: Json = null
  private usage: Required<Omit<RunUsage, 'model'>> & Pick<RunUsage, 'model'> = { input_tokens: 0, output_tokens: 0, calls: 0 }
  /** A failure of the loop's infrastructure (a call that never reached the service): it ends the run, not the model. */
  private fatal: unknown = null
  private xllm: XllmRun | null = null
  private readonly session: ExecutorSession
  private readonly infer: Infer
  readonly runId: string
  private readonly claimToken: string

  constructor(session: ExecutorSession, infer: Infer, runId: string, claimToken: string) {
    this.session = session
    this.infer = infer
    this.runId = runId
    this.claimToken = claimToken
    this.abort.signal.addEventListener('abort', () => this.xllm?.interrupter().interrupt('stopped by the page'))
  }

  /** Runs to the report (`finished`), or stops without one when cancelled, revoked or the page goes away
   * (`stopped`: the claim stays usable for a resume); rejects with the executor's own failure (the service
   * already knows: it was reported). */
  async run(): Promise<'finished' | 'stopped'> {
    const claim = await this.session.procClaim(this.runId, this.claimToken)
    this.attempt = claim.attempt
    const heartbeat = window.setInterval(() => {
      this.session.procHeartbeat(this.runId, this.attempt, { phase: this.phase }).catch((error: unknown) => {
        // a lapsed lease or a cancel: the service revoked this attempt; stop quietly
        if (codeOf(error) === 'RUN_REVOKED') this.abort.abort()
      })
    }, Math.max(1000, Math.floor((claim.lease_ms || 90_000) / 3)))
    try {
      const stage = claim.stage
      if (!stage) throw new Error(aiwsT('aiworkspace.wish.error.noStage'))
      this.budget = stage.budget ?? {}
      this.manifest = stage.manifest ?? null
      // running the stored macro: the service judges the run (a failure ends it with its own code), then plans it
      if (stage.kind === 'program') await this.program(stage.source ?? '')
      else await this.loop(stage)
      await this.session.procReport(this.runId, this.attempt, { finished: true, ...(this.said ? { summary: this.said } : {}), ...this.used() })
      claimUsage.set(this.runId, null)
      return 'finished'
    } catch (error) {
      // stopped on purpose (cancel, the page going away) or revoked by the service: nothing to report
      if (this.abort.signal.aborted || codeOf(error) === 'RUN_REVOKED' || (error instanceof DOMException && error.name === 'AbortError')) return 'stopped'
      const detail = error instanceof Error ? error.message : String(error)
      const code = codeOf(error)?.startsWith('AI_') || codeOf(error) === 'SNAPSHOT_TOO_LARGE' ? codeOf(error)! : 'EXECUTOR_FAILED'
      claimUsage.set(this.runId, null)
      await this.session.procReport(this.runId, this.attempt, { error: { code, detail }, ...this.used() }).catch(() => undefined)
      throw error
    } finally {
      window.clearInterval(heartbeat)
    }
  }

  cancel() { this.abort.abort() }

  /** The report's `usage` when the stage's model was called (running a stored macro calls none). */
  private used(): { usage?: RunUsage } {
    return this.usage.calls > 0 ? { usage: { ...this.usage } } : {}
  }

  private checkAborted() {
    if (this.abort.signal.aborted) throw new DOMException('aborted', 'AbortError')
  }

  private async snapshot() {
    this.files ??= await this.session.procFiles(this.runId, this.attempt)
    return this.files
  }

  /** Run `source` on the snapshot in the sandbox, answering its bridge, and hand the run in. */
  private async program(source: string): Promise<WishToolAnswer> {
    this.phase = 'running_program'
    const snapshot = await this.snapshot()
    const budget = { ...this.budget, ...(snapshot.stage?.budget ?? {}) }
    const manifest = (this.manifest ?? snapshot.stage?.manifest ?? null) as { resources?: { timeout_secs?: number } } | null
    const host = createBridgeHost({
      tool: (name, args) => this.session.procTool(this.runId, this.attempt, name, args as Json) as Promise<{ status?: string; text?: string; code?: string }>,
      infer: this.infer,
      zoneFetch,
      aicc: aiccCall,
      task: aiccTask,
      importAsset: (objId) => this.session.importObject(objId),
      zoneHosts: zoneHosts(),
      zoneOrigin: window.location.origin,
      budget: budget as Record<string, unknown>,
      manifest: manifest && typeof manifest === 'object' ? manifest : null,
    })
    const cap = Number(budget.program_timeout_secs) || DEFAULT_TIMEOUT_SECS
    const declared = Number(manifest?.resources?.timeout_secs) || cap
    const out = await runProgram(source, snapshot.files, (op, req) => host.handle(op, req) as Promise<Json>, { timeoutMs: Math.min(cap, declared) * 1000, signal: this.abort.signal })
    return this.session.procProgram(this.runId, this.attempt, { source, results: out.results, log: out.log, files: out.files })
  }

  /** The stage's model loop on xllm (余下任务 §2.1): the brief's system text as the host's, no xllm sections (the
   * stage's prompts carry their own rules and reply contract), its user text as the input, its tools as the page tools. */
  private async loop(stage: WishStageBrief) {
    const tools = stage.tools ?? []
    this.usage = { input_tokens: 0, output_tokens: 0, calls: 0, ...(claimUsage.get(this.runId) ?? {}) }
    const llm = new CountingLlm(stageLlm(), (response, model) => {
      this.usage.calls += 1
      this.usage.input_tokens += response.usage?.input_tokens ?? 0
      this.usage.output_tokens += response.usage?.output_tokens ?? 0
      this.usage.model = model
      claimUsage.set(this.runId, { ...this.usage })
    })
    const runtime = wishRuntime(
      tools,
      (name, args) => name === 'run_program' ? this.program(String(args.source ?? '')) : this.session.procTool(this.runId, this.attempt, name, args),
      (error) => { this.fatal ??= error; this.xllm?.interrupter().interrupt('the service could not be reached') },
    )
    const observer = { onEvent: (_: string, event: RunEvent) => {
      if (event.kind === 'llm_started') this.phase = 'waiting_model'
      else if (event.kind === 'tool_started') this.phase = event.name
    } }
    // the run lives as long as this attempt: a new attempt starts its loop again (§7.1 4)
    const deps = { llm, runtime, store: new MemoryRunStore(), observer, hostPromptSections: 'NONE' as const }
    const llmContext: JsonValue = {
      provider: { type: 'buckyos' },
      ...(stage.model ? { model: stage.model } : {}),
      loop_model: 'function_call',
      max_tool_iterations: stage.budget?.iterations ?? stage.iterations ?? DEFAULT_ITERATIONS,
      timeout: STAGE_TIMEOUT_SECS,
      llm_timeout: LLM_TIMEOUT_SECS,
      context_window: CONTEXT_WINDOW,
      tools: { enabled: true, tools: tools.map((t) => ({ name: t.name })) },
    }
    this.checkAborted()
    // the stage's text is the service's, verbatim: never an xllm template
    const task = await XllmTask.prepareHosted(llmContext, 'aiws.wish', (stage.system ?? '').replaceAll('{{', '\\{{'), deps)
    this.xllm = await XllmRun.start(task, deps, { input: stage.user ?? '' })
    if (this.abort.signal.aborted) this.xllm.interrupter().interrupt('stopped by the page')
    let outcome: RunOutcome = await this.xllm.execute()
    for (let retry = 0; retry < TRANSIENT_RETRIES && outcome.status === 'paused' && TRANSIENT_KINDS.includes(outcome.record.last_error?.kind ?? '') && !this.fatal; retry++) {
      await new Promise((resolve) => window.setTimeout(resolve, 2000 * (retry + 1)))
      outcome = await this.xllm.execute()
    }
    if (this.fatal) throw this.fatal
    this.checkAborted()
    const record = outcome.record
    if (outcome.status === 'completed') {
      this.said = (record.result?.raw ?? '').trim().slice(0, 2000)
      return
    }
    if (outcome.status === 'interrupted') throw new DOMException('aborted', 'AbortError')
    if (outcome.status === 'limit_reached') throw new Error(record.limit_reason ?? aiwsT('aiworkspace.wish.error.iterations', { count: record.config.limits.max_tool_iterations }))
    const failure = record.last_error
    const error = new Error([failure?.message, failure?.condition].filter(Boolean).join(' — ') || outcome.status) as Error & { code?: string }
    const refused = refusalCode(failure?.kind)
    if (refused) error.code = refused
    throw error
  }
}
