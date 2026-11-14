/* Runs a wish program and collects what it produced. The program is model output, so it runs with less than the
 * page (内核架构改进 D21): a hidden iframe with `sandbox="allow-scripts"` (an opaque origin: none of the page's
 * IndexedDB or Cache Storage) and a CSP without any network source (no request leaves, with or without the
 * page's cookie) starts the runner (program.worker.ts, `virtual:aiws-program-runner`) as a classic Worker from a
 * blob; the Worker inherits the CSP. Both are needed. The program lives in the Worker, not in the iframe's
 * script, which could navigate itself away. What goes
 * in is the snapshot files, the program source and the answers to its bridge calls — never a token, since the
 * sandbox's origin is `null` and messages to it are posted to `*`. What comes out is checked: only the iframe's
 * window, only a bridge call of a known kind and `done`, by shape. A timeout or a cancel removes the iframe, which
 * ends the Worker. */

import runner from 'virtual:aiws-program-runner'
import type { Json } from '../api/types'
import type { BridgeReply, RunMessage } from './program.worker'

export interface ProgramOutcome { results: Json | null; log: string; files: Record<string, Json> }

const SANDBOX_CSP = "default-src 'none'; script-src 'unsafe-inline' blob:; worker-src blob:"

// The iframe's script: start the runner from the first message, then relay both ways. A Worker that fails to start
// or throws outside the program ends the run with an error, as a `done`.
const SANDBOX_SCRIPT = `
let worker = null
const fail = (message) => parent.postMessage({ type: 'done', results: { error: message }, log: '', files: {} }, '*')
onmessage = (event) => {
  if (event.source !== parent) return
  if (worker) { worker.postMessage(event.data); return }
  try {
    worker = new Worker(URL.createObjectURL(new Blob([event.data.runner], { type: 'text/javascript' })), { name: 'aiworkspace-wish-program' })
  } catch (error) { fail(String(error)); return }
  worker.onmessage = (e) => parent.postMessage(e.data, '*')
  worker.onerror = (e) => { e.preventDefault(); fail(e.message || 'the program host failed to load') }
  worker.postMessage(event.data.run)
}`

const SANDBOX_DOC = `<!doctype html><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${SANDBOX_CSP}"><script>${SANDBOX_SCRIPT}</script>`

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && Object.getPrototypeOf(value) === Object.prototype
}

/** What a macro may ask its executor (aiws.js `bridge`). */
const BRIDGE_OPS = ['sql', 'ai.complete', 'llm.map', 'zone.fetch', 'ai.image', 'ai.video']
/** Bridge calls whose wait the program's time limit does not count: a generated picture or clip has its own. */
const MEDIA_OPS = new Set(['ai.image', 'ai.video'])

type Out =
  | { type: 'bridge'; id: number; op: string; req: Json }
  | { type: 'done'; results: Json | null; log: string; files: Record<string, Json> }

/** A message of the sandbox, if it is one of the two it may send. */
function parseOut(data: unknown): Out | null {
  if (!isRecord(data)) return null
  if (data.type === 'bridge') {
    return Number.isSafeInteger(data.id) && typeof data.op === 'string' && BRIDGE_OPS.includes(data.op) && isRecord(data.req) ? { type: 'bridge', id: data.id as number, op: data.op, req: data.req as Json } : null
  }
  if (data.type === 'done') {
    const { results, log, files } = data
    if (!(results === null || isRecord(results)) || typeof log !== 'string' || !isRecord(files)) return null
    const ok = Object.values(files).every((f) => typeof f === 'string' || (isRecord(f) && Object.keys(f).length === 1 && typeof f.base64 === 'string'))
    return ok ? { type: 'done', results: results as Json | null, log, files: files as Record<string, Json> } : null
  }
  return null
}

export function runProgram(
  source: string,
  files: Record<string, { text?: string; base64?: string; skipped?: string }>,
  bridge: (op: string, req: Json) => Promise<Json>,
  options: { timeoutMs: number; signal?: AbortSignal },
): Promise<ProgramOutcome> {
  return new Promise((resolve, reject) => {
    const iframe = document.createElement('iframe')
    iframe.setAttribute('sandbox', 'allow-scripts')
    iframe.hidden = true
    iframe.title = 'aiworkspace-wish-program'
    iframe.srcdoc = SANDBOX_DOC
    const post = (msg: unknown) => iframe.contentWindow?.postMessage(msg, '*')
    let settled = false
    // the time the program itself took: waits for generated media are not counted
    const started = Date.now()
    let media = 0
    let mediaSince = 0
    let mediaMs = 0
    const used = () => Date.now() - started - mediaMs - (media ? Date.now() - mediaSince : 0)
    const finish = (fn: () => void) => {
      if (settled) return
      settled = true
      window.clearInterval(timer)
      options.signal?.removeEventListener('abort', onAbort)
      window.removeEventListener('message', onMessage)
      iframe.remove()
      fn()
    }
    const onAbort = () => finish(() => reject(new DOMException('aborted', 'AbortError')))
    const onMessage = (event: MessageEvent) => {
      if (event.source !== iframe.contentWindow || settled) return
      const msg = parseOut(event.data)
      if (!msg) return
      if (msg.type === 'done') finish(() => resolve({ results: msg.results, log: msg.log, files: msg.files }))
      else {
        const waiting = MEDIA_OPS.has(msg.op)
        if (waiting && media++ === 0) mediaSince = Date.now()
        bridge(msg.op, msg.req)
          .then((body) => post({ type: 'bridge', id: msg.id, body } satisfies BridgeReply))
          .catch((error: unknown) => post({ type: 'bridge', id: msg.id, body: { error: error instanceof Error ? error.message : String(error) } } satisfies BridgeReply))
          .finally(() => { if (waiting && --media === 0) mediaMs += Date.now() - mediaSince })
      }
    }
    const timer = window.setInterval(() => {
      if (used() > options.timeoutMs) finish(() => resolve({ results: { error: `The program timed out (${Math.round(options.timeoutMs / 1000)} s)` }, log: '', files: {} }))
    }, 1000)
    options.signal?.addEventListener('abort', onAbort)
    window.addEventListener('message', onMessage)
    iframe.addEventListener('load', () => post({ runner, run: { type: 'run', source, files } satisfies RunMessage }), { once: true })
    document.body.append(iframe)
  })
}
