// Runs the wish program `program/main.js` against this run's snapshot with the `aiws` program host
// and writes `output/.aiws/results.json`. The host's canonical run ("run_program", "re-run the
// program") and a person or model debugging in the shell use exactly this file:
//
//   deno run -A lib/run.js
//
// Exit code 0: the program finished; 1: it threw (the error is in results.json and on stderr).

import { createHost } from './aiws.js'

const root = new URL('..', import.meta.url).pathname
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
if (failed) {
  console.error(String(failed?.stack ?? failed))
  Deno.exit(1)
}
