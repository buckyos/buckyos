/* Runs a wish program in a Worker (program.worker.ts) and collects what it produced. The Worker holds
 * no token: inputs come in as files, `aiws.llm.map` requests come back here and are answered by the
 * caller (the service, under the run's attribution). */

import type { Json } from '../api/types'
import type { MapReply, RunMessage, WorkerOut } from './program.worker'

export interface ProgramOutcome { results: Json | null; log: string; files: Record<string, Json> }

export function runProgram(
  source: string,
  files: Record<string, { text?: string; base64?: string; skipped?: string }>,
  llmMap: (req: Json) => Promise<Json>,
  options: { timeoutMs: number; signal?: AbortSignal },
): Promise<ProgramOutcome> {
  return new Promise((resolve, reject) => {
    const worker = new Worker(new URL('./program.worker.ts', import.meta.url), { type: 'module', name: 'aiworkspace-wish-program' })
    let settled = false
    const finish = (fn: () => void) => { if (settled) return; settled = true; window.clearTimeout(timer); options.signal?.removeEventListener('abort', onAbort); worker.terminate(); fn() }
    const onAbort = () => finish(() => reject(new DOMException('aborted', 'AbortError')))
    const timer = window.setTimeout(() => finish(() => resolve({ results: { error: `The program timed out (${Math.round(options.timeoutMs / 1000)} s)` }, log: '', files: {} })), options.timeoutMs)
    options.signal?.addEventListener('abort', onAbort)
    worker.onerror = (event) => { event.preventDefault(); finish(() => resolve({ results: { error: event.message || 'the program host failed to load' }, log: '', files: {} })) }
    worker.onmessage = (event: MessageEvent<WorkerOut>) => {
      const msg = event.data
      if (msg.type === 'done') finish(() => resolve({ results: msg.results as Json, log: msg.log, files: msg.files as Record<string, Json> }))
      else if (msg.type === 'llm_map') {
        llmMap(msg.req as Json)
          .then((body) => worker.postMessage({ type: 'llm_map', id: msg.id, body } satisfies MapReply))
          .catch((error: unknown) => worker.postMessage({ type: 'llm_map', id: msg.id, body: { error: error instanceof Error ? error.message : String(error) } } satisfies MapReply))
      }
    }
    worker.postMessage({ type: 'run', source, files } satisfies RunMessage)
  })
}
