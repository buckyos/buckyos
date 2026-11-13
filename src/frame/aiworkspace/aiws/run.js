// Runs the wish macro `program/main.js` against this run's snapshot with the `aiws` program host and writes
// `output/.aiws/results.json`. Every executor runs a macro exactly this way:
//
//   deno run --allow-read=<dir> --allow-write=<dir>/output lib/run.js --bridge
//
// With `--bridge` the program's bridge calls (`aiws.sql`, `aiws.ai.complete`, `aiws.llm.map`, `aiws.zone.fetch`) go
// to the process that started it, over stdio: each call is one line on stdout, `\x1eaiws {"id":n,"op":"sql","req":{…}}`,
// and its answer one line on stdin, `{"id":n,"body":{…}}`. The program's own console output goes to stderr. Without
// `--bridge` (a person debugging), those calls fail and only the snapshot can be read.
//
// Exit code 0: the macro finished; 1: it threw (the error is in results.json and on stderr).

import { createHost } from './aiws.js'
import { fileURLToPath } from 'node:url'

const BRIDGE = '\x1eaiws '

if (Deno.args.includes('--bridge')) {
  const enc = new TextEncoder()
  const waiters = new Map()
  let seq = 0
  const say = (line) => Deno.stdout.writeSync(enc.encode(line + '\n'))
  for (const level of ['log', 'info', 'warn', 'error', 'debug']) {
    console[level] = (...args) => Deno.stderr.writeSync(enc.encode(args.map((a) => (typeof a === 'string' ? a : JSON.stringify(a))).join(' ') + '\n'))
  }
  globalThis.__aiwsBridge = (op, req) => new Promise((resolve) => {
    const id = ++seq
    waiters.set(id, resolve)
    say(BRIDGE + JSON.stringify({ id, op, req }))
  })
  ;(async () => {
    const dec = new TextDecoder()
    let buf = ''
    for await (const chunk of Deno.stdin.readable) {
      buf += dec.decode(chunk, { stream: true })
      let i
      while ((i = buf.indexOf('\n')) >= 0) {
        const line = buf.slice(0, i).trim()
        buf = buf.slice(i + 1)
        if (!line) continue
        let msg
        try { msg = JSON.parse(line) } catch { continue }
        const w = waiters.get(msg.id)
        if (w) {
          waiters.delete(msg.id)
          w(msg.body)
        }
      }
    }
    for (const w of waiters.values()) w({ error: 'the executor closed the bridge' })
    waiters.clear()
  })()
}

const root = fileURLToPath(new URL('..', import.meta.url))
const host = createHost(root)
let failed = null
try {
  const url = new URL('../program/main.js', import.meta.url)
  const mod = await import(url.href)
  const main = mod.default ?? mod.main
  if (typeof main !== 'function') throw new Error('program/main.js must `export default async function main(aiws) { … }`')
  await main(host.aiws)
} catch (e) {
  failed = e
}
host.flush(failed)
const summary = host.summary()
if (summary) console.log(summary)
if (failed) console.error(String(failed?.stack ?? failed))
Deno.exit(failed ? 1 : 0)
