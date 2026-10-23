/* The local executor (内核架构改进 §9, D14, T3-4): this browser executes a wish run it started. It claims the
 * run, keeps the lease alive, hosts the model loop — the stage's prompts and tools come from the service,
 * every tool call goes back to it — runs the program in a Worker on the snapshot, and reports when done. The
 * service validates, plans and waits for the user's confirmation exactly as for any other executor.
 *
 * The claim token of a run is kept in this browser (localStorage) so a reload can claim it again: a run
 * whose lease lapsed is `interrupted` and resumes as a new attempt. */

import type { Json, WishClaim, WishStageBrief, WishToolAnswer } from '../api/types'
import { aiwsT } from '../i18n'
import { type ChatClient, type ChatMessage, type ChatTool } from './chat'
import { runProgram } from './programHost'

const CLAIMS_KEY = 'aiworkspace.claims'
const PROGRAM_TIMEOUT_MS = 120_000
const MAX_ITERATIONS = 60

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
  procFiles(runId: string, attempt: number): Promise<Record<string, { text?: string; base64?: string; skipped?: string }>>
}

function codeOf(error: unknown): string | undefined {
  return (error as { code?: string } | null)?.code
}

export class LocalExecutor {
  private readonly abort = new AbortController()
  private attempt = 0
  private files: Record<string, { text?: string; base64?: string; skipped?: string }> | null = null
  private phase = 'claimed'
  private readonly session: ExecutorSession
  private readonly chat: ChatClient
  readonly runId: string
  private readonly claimToken: string

  constructor(session: ExecutorSession, chat: ChatClient, runId: string, claimToken: string) {
    this.session = session
    this.chat = chat
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
      if (stage.kind === 'program') await this.rerun(stage)
      else await this.loop(stage)
      await this.session.procReport(this.runId, this.attempt, { finished: true })
      return 'finished'
    } catch (error) {
      // stopped on purpose (cancel, the page going away) or revoked by the service: nothing to report
      if (this.abort.signal.aborted || codeOf(error) === 'RUN_REVOKED' || (error instanceof DOMException && error.name === 'AbortError')) return 'stopped'
      const detail = error instanceof Error ? error.message : String(error)
      await this.session.procReport(this.runId, this.attempt, { error: { code: 'EXECUTOR_FAILED', detail } }).catch(() => undefined)
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

  private async program(source: string): Promise<WishToolAnswer> {
    this.phase = 'running_program'
    const files = await this.snapshot()
    const llmMap = async (req: Json) => {
      const answer = await this.session.procTool(this.runId, this.attempt, 'llm_map', req)
      return answer.status === 'ok' ? answer.map ?? null : { error: answer.text ?? 'llm.map failed' }
    }
    const out = await runProgram(source, files, llmMap, { timeoutMs: PROGRAM_TIMEOUT_MS, signal: this.abort.signal })
    return this.session.procProgram(this.runId, this.attempt, { source, results: out.results, log: out.log, files: out.files })
  }

  private async rerun(stage: WishStageBrief) {
    const answer = await this.program(stage.source ?? '')
    if (answer.status !== 'ok') throw new Error(answer.text ?? aiwsT('aiworkspace.wish.error.mockFailed'))
  }

  private async loop(stage: WishStageBrief) {
    const tools: ChatTool[] = stage.tools ?? []
    const messages: ChatMessage[] = [{ role: 'system', text: stage.system ?? '' }, { role: 'user', text: stage.user ?? '' }]
    const iterations = Math.min(stage.iterations ?? MAX_ITERATIONS, MAX_ITERATIONS)
    for (let i = 0; i < iterations; i++) {
      this.checkAborted()
      this.phase = 'waiting_model'
      const turn = await this.chat.complete(stage.model ?? '', messages, tools, this.abort.signal)
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
        messages.push({ role: 'tool', callId: call.id, text: answer.text ?? JSON.stringify(answer.map ?? null), isError: answer.status === 'error' })
      }
    }
    throw new Error(aiwsT('aiworkspace.wish.error.iterations', { count: iterations }))
  }
}
