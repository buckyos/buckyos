/* The program runner of the local executor (内核架构改进 T3-4, D21): a wish program runs in this Worker on
 * the stage snapshot the main thread fetched, through the same program host the service uses (aiws.js). It
 * sees files, not the Workspace: no session, no token, no kRPC. `aiws.llm.map` goes to the main thread,
 * which asks the service under the run's attribution. Everything the program writes under output/ and
 * what the host collected go back as the run's output. */

import { createHost } from '../../../../../aiworkspace/aiws/aiws.js'

type FileIn = { text?: string; base64?: string; skipped?: string }
export type RunMessage = { type: 'run'; source: string; files: Record<string, FileIn> }
export type MapReply = { type: 'llm_map'; id: number; body: unknown }
export type WorkerOut =
  | { type: 'llm_map'; id: number; req: unknown }
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

let mapSeq = 0
const mapWaiters = new Map<number, (body: unknown) => void>()
;(globalThis as unknown as { __aiwsLlmMap: (req: unknown) => Promise<unknown> }).__aiwsLlmMap = (req) => new Promise((resolve) => {
  const id = ++mapSeq
  mapWaiters.set(id, resolve)
  postMessage({ type: 'llm_map', id, req } satisfies WorkerOut)
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
    const mod = await import(/* @vite-ignore */ url) as { default?: unknown; main?: unknown }
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
  if (failed) log.push(String((failed as { stack?: string })?.stack ?? failed))
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

onmessage = (event: MessageEvent<RunMessage | MapReply>) => {
  const msg = event.data
  if (msg.type === 'run') void run(msg)
  else if (msg.type === 'llm_map') { mapWaiters.get(msg.id)?.(msg.body); mapWaiters.delete(msg.id) }
}
