/* The local executor (内核架构改进 §9, D14, T3-4; 浏览器优先; AI Native 第一期 §2): this browser executes a wish run
 * it started — analyze, code or run. It claims the run, keeps the lease alive, hosts the model loop of a model stage
 * (the stage's prompts and tools come from the service, every tool call goes back to it), runs the macro in a Worker on
 * the snapshot and answers its bridge with host.js (SQL through the service, AI with the user's own session, Zone
 * resources with the page's), and reports when done. The service validates, plans and waits for the user's confirmation
 * exactly as for any other executor; it calls no model.
 *
 * The claim token of a run is kept in this browser (localStorage) so a reload can claim it again: a run
 * whose lease lapsed is `interrupted` and resumes as a new attempt. */

import { createBridgeHost } from '../../../../../aiworkspace/aiws/host.js'
import type { Json, StagedAsset, WishBudget, WishClaim, WishStageBrief, WishStageFiles, WishToolAnswer } from '../api/types'
import { aiwsT } from '../i18n'
import { type ChatClient, type ChatMessage, type ChatTool } from './chat'
import { aiccCall, aiccTask, classified, type Infer } from './infer'
import { runProgram } from './programHost'

const CLAIMS_KEY = 'aiworkspace.claims'
const DEFAULT_ITERATIONS = 60
const DEFAULT_TIMEOUT_SECS = 120

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
  },
}

export interface ExecutorSession {
  procClaim(runId: string, claimToken: string): Promise<WishClaim>
  procHeartbeat(runId: string, attempt: number, progress?: Json): Promise<void>
  procReport(runId: string, attempt: number, body: Record<string, Json>): Promise<unknown>
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
  private budget: Partial<WishBudget> = {}
  private manifest: Json = null
  private readonly session: ExecutorSession
  private readonly chat: ChatClient
  private readonly infer: Infer
  readonly runId: string
  private readonly claimToken: string

  constructor(session: ExecutorSession, chat: ChatClient, infer: Infer, runId: string, claimToken: string) {
    this.session = session
    this.chat = chat
    this.infer = infer
    this.runId = runId
    this.claimToken = claimToken
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
      await this.session.procReport(this.runId, this.attempt, { finished: true })
      return 'finished'
    } catch (error) {
      // stopped on purpose (cancel, the page going away) or revoked by the service: nothing to report
      if (this.abort.signal.aborted || codeOf(error) === 'RUN_REVOKED' || (error instanceof DOMException && error.name === 'AbortError')) return 'stopped'
      const detail = error instanceof Error ? error.message : String(error)
      const code = codeOf(error)?.startsWith('AI_') || codeOf(error) === 'SNAPSHOT_TOO_LARGE' ? codeOf(error)! : 'EXECUTOR_FAILED'
      await this.session.procReport(this.runId, this.attempt, { error: { code, detail } }).catch(() => undefined)
      throw error
    } finally {
      window.clearInterval(heartbeat)
    }
  }

  cancel() { this.abort.abort() }

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

  private async loop(stage: WishStageBrief) {
    const tools: ChatTool[] = stage.tools ?? []
    const messages: ChatMessage[] = [{ role: 'system', text: stage.system ?? '' }, { role: 'user', text: stage.user ?? '' }]
    const iterations = stage.budget?.iterations ?? stage.iterations ?? DEFAULT_ITERATIONS
    for (let i = 0; i < iterations; i++) {
      this.checkAborted()
      this.phase = 'waiting_model'
      const turn = await this.chat.complete(stage.model ?? '', messages, tools, this.abort.signal).catch((error: unknown) => { throw classified(error) })
      messages.push({ role: 'assistant', text: turn.text, toolCalls: turn.toolCalls, raw: turn.raw })
      if (turn.toolCalls.length === 0) return
      for (const call of turn.toolCalls) {
        this.checkAborted()
        this.phase = call.name
        let answer: WishToolAnswer
        if (call.name === 'run_program') {
          answer = await this.program(String(call.args.source ?? ''))
        } else {
          answer = await this.session.procTool(this.runId, this.attempt, call.name, call.args)
        }
        messages.push({ role: 'tool', callId: call.id, text: answer.text ?? '', isError: answer.status === 'error' })
      }
    }
    throw new Error(aiwsT('aiworkspace.wish.error.iterations', { count: iterations }))
  }
}
