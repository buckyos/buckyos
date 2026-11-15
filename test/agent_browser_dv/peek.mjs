#!/usr/bin/env node
// What a session of the Agent did, read from its directory on this host (the committed worklog comes through kRPC
// only when a Turn closed; this also shows a Turn that is still open): every tool call with its result, from the
// newest snapshot of each run.
//
//   node peek.mjs <sid> [--full] [--calls] [--grep <string>]
//
//   --full   results in full (default: the first 1500 characters)
//   --calls  the commands only
//   --grep   only say how often a string occurs in everything the runs hold (prompts, calls, results)
//
// Needs to run on the node that holds the AgentRoot: $BUCKYOS_ROOT/data/home/<owner>/agents/<agent_id>/sessions/<sid>.
import { existsSync, readdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { AGENT_ID, OWNER } from './lib.mjs'

const [sid, ...options] = process.argv.slice(2)
const full = options.includes('--full')
const callsOnly = options.includes('--calls')
const needle = options.includes('--grep') ? options[options.indexOf('--grep') + 1] : null
const root = join(process.env.BUCKYOS_ROOT ?? '/opt/buckyos', 'data/home', OWNER, 'agents', AGENT_ID, 'sessions', sid, '.opendan_agent_session')
const runs = join(root, 'runs')
if (!existsSync(runs)) { console.error(`no runs under ${root}`); process.exit(1) }
const clip = (text, n = 1500) => (full || text.length <= n ? text : `${text.slice(0, n)}\n…(+${text.length - n} characters)`)
const textOf = (content) => (typeof content === 'string' ? content : Array.isArray(content) ? content.map((part) => part.text ?? part.content ?? (part.type === 'text' ? '' : JSON.stringify(part))).join('') : JSON.stringify(content))

let hits = 0
for (const run of readdirSync(runs).sort()) {
  const dir = join(runs, run, 'snapshots')
  if (!existsSync(dir)) continue
  const last = readdirSync(dir).sort().at(-1)
  if (!last) continue
  const raw = readFileSync(join(dir, last), 'utf8')
  if (needle) { hits += raw.split(needle).length - 1; continue }
  const snapshot = JSON.parse(raw)
  const usage = snapshot.state?.usage ?? {}
  console.log(`===== run ${run} (snapshot ${last}) tokens in=${usage.input_tokens} out=${usage.output_tokens} cached=${usage.cache_read_input_tokens}`)
  for (const message of snapshot.state?.accumulated ?? []) {
    if (message.role === 'system') continue
    const parts = Array.isArray(message.content) ? message.content : [{ type: 'text', text: String(message.content ?? '') }]
    for (const part of parts) {
      if (['provider_state', 'reasoning', 'thinking'].includes(part.type)) continue
      if (part.type === 'tool_use' || part.type === 'tool_call' || part.type === 'function_call') {
        const input = part.input ?? part.arguments ?? part.args ?? {}
        const shown = typeof input === 'string' ? input : input.command ?? JSON.stringify(input)
        console.log(`\n$ [${part.name}] ${clip(String(shown), 3000)}${input.intent ? `\n  # ${input.intent}` : ''}`)
      } else if (part.type === 'tool_result' || part.type === 'function_call_output') {
        if (!callsOnly) console.log(clip(textOf(part.content ?? part.output ?? '')))
      } else if (!callsOnly || message.role === 'assistant') {
        const text = textOf(part.text ?? part)
        if (text.trim()) console.log(`\n[${message.role}] ${clip(text)}`)
      }
    }
    for (const call of message.tool_calls ?? []) {
      const args = call.function?.arguments ?? call.arguments ?? ''
      let shown = args
      try { const parsed = JSON.parse(args); shown = parsed.command ?? args } catch { /* keep the text */ }
      console.log(`\n$ [${call.function?.name ?? call.name}] ${clip(String(shown), 3000)}`)
    }
  }
}
if (needle) { console.log(`${hits} occurrence(s) in the runs of ${sid}`); process.exit(hits ? 3 : 0) }
