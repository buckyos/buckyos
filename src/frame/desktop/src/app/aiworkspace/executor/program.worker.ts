/* The program runner of the local executor (内核架构改进 T3-4, D21; AI Native 第一期 §2.6): a wish macro runs in this
 * Worker on the stage snapshot the main thread fetched, through the program host every executor uses (aiws.js). The
 * Worker is a classic one inside programHost.ts's sandbox (an opaque origin without network): it sees files, not the
 * Workspace — no session, no cookie, none of the page's storage, no network. The macro's bridge calls (`aiws.sql`,
 * `aiws.ai.complete`, `aiws.llm.map`, `aiws.zone.fetch`) go to the main thread, which answers them with host.js.
 * Everything the macro writes under output/ and what the host collected go back as the run's output. Built into one
 * script text by vite.config.ts (`virtual:aiws-program-runner`). */

import { createHost } from '../../../../../aiworkspace/aiws/aiws.js'

type FileIn = { text?: string; base64?: string; skipped?: string }
export type RunMessage = { type: 'run'; source: string; files: Record<string, FileIn> }
export type BridgeReply = { type: 'bridge'; id: number; body: unknown }
export type WorkerOut =
  | { type: 'bridge'; id: number; op: string; req: unknown }
  | { type: 'done'; results: unknown | null; log: string; files: Record<string, string | { base64: string }> }

const enc = new TextEncoder()
const dec = new TextDecoder()
const files = new Map<string, Uint8Array>()
const written = new Set<string>()
const log: string[] = []

function norm(path: string): string {
  return String(path).replace(/\\/g, '/').replace(/^(\.\/|\/)+/, '').replace(/\/+/g, '/')
}

function must(path: string): Uint8Array {
  const f = files.get(norm(path))
  if (!f) throw new Error(`No such file: ${norm(path)}`)
  return f
}

function fromBase64(b64: string): Uint8Array {
  const bin = atob(b64)
  const out = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i)
  return out
}

function toBase64(bytes: Uint8Array): string {
  let bin = ''
  for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000))
  return btoa(bin)
}

/** What the program host expects of `Deno`, over the in-memory snapshot. */
const deno = {
  readTextFileSync: (path: string) => dec.decode(must(path)),
  readFileSync: (path: string) => must(path),
  writeTextFileSync: (path: string, text: string) => { files.set(norm(path), enc.encode(text)); written.add(norm(path)) },
  writeFileSync: (path: string, bytes: Uint8Array) => { files.set(norm(path), bytes); written.add(norm(path)) },
  mkdirSync: () => {},
  env: { get: () => undefined },
  exit: (code: number) => { throw new Error(`Deno.exit(${code})`) },
}
;(globalThis as unknown as { Deno: unknown }).Deno = deno

// the sandbox refuses every request: a failing fetch says so, in words the model can act on
const sandboxFetch = globalThis.fetch
globalThis.fetch = (input, init) => sandboxFetch(input, init).catch((error: unknown) => {
  throw new TypeError(`fetch(${input instanceof Request ? input.url : String(input)}) failed (${String(error)}): a wish macro reaches nothing directly; read its declared inputs (aiws.input, aiws.sql) or a resource of the Zone (aiws.zone.fetch)`)
})

let bridgeSeq = 0
const bridgeWaiters = new Map<number, (body: unknown) => void>()
;(globalThis as unknown as { __aiwsBridge: (op: string, req: unknown) => Promise<unknown> }).__aiwsBridge = (op, req) => new Promise((resolve) => {
  const id = ++bridgeSeq
  bridgeWaiters.set(id, resolve)
  postMessage({ type: 'bridge', id, op, req } satisfies WorkerOut)
})

for (const level of ['log', 'info', 'warn', 'error', 'debug'] as const) {
  console[level] = (...args: unknown[]) => { log.push(args.map((a) => (typeof a === 'string' ? a : JSON.stringify(a))).join(' ')) }
}

async function run(msg: RunMessage) {
  for (const [path, f] of Object.entries(msg.files)) {
    if (typeof f.text === 'string') files.set(norm(path), enc.encode(f.text))
    else if (typeof f.base64 === 'string') files.set(norm(path), fromBase64(f.base64))
  }
  const host = createHost('.')
  let failed: unknown = null
  let url: string | null = null
  try {
    url = URL.createObjectURL(new Blob([msg.source], { type: 'text/javascript' }))
    const mod = await import(/* @vite-ignore */ url).catch((error: unknown) => {
      throw error instanceof TypeError ? new TypeError(`program/main.js did not load (${String(error)}): a wish program cannot import modules, from the network or from files; use the JavaScript built-ins and the aiws argument of main`) : error
    }) as { default?: unknown; main?: unknown }
    const main = mod.default ?? mod.main
    if (typeof main !== 'function') throw new Error('program/main.js must `export default async function main(aiws) { … }`')
    await (main as (aiws: unknown) => Promise<void>)(host.aiws)
  } catch (error) {
    failed = error
  } finally {
    if (url) URL.revokeObjectURL(url)
  }
  const results = host.flush(failed)
  const summary = host.summary()
  if (summary) log.push(summary)
  if (results.error) log.push(results.error)
  const outputs: Record<string, string | { base64: string }> = {}
  for (const path of written) {
    if (!path.startsWith('output/') || path.startsWith('output/.aiws/')) continue
    const bytes = files.get(path)!
    let text: string | null = null
    try { text = bytes.includes(0) ? null : new TextDecoder('utf-8', { fatal: true }).decode(bytes) } catch { text = null }
    outputs[path] = text ?? { base64: toBase64(bytes) }
  }
  postMessage({ type: 'done', results, log: log.join('\n'), files: outputs } satisfies WorkerOut)
}

onmessage = (event: MessageEvent<RunMessage | BridgeReply>) => {
  const msg = event.data
  if (msg.type === 'run') void run(msg)
  else if (msg.type === 'bridge') { bridgeWaiters.get(msg.id)?.(msg.body); bridgeWaiters.delete(msg.id) }
}
