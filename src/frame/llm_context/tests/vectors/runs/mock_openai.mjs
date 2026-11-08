// A scripted OpenAI-compatible Chat Completions endpoint for the xllm run
// fixtures and the Rust ↔ TS conformance runs. It answers by call number per
// scenario, never by matching prompt text:
//
//   POST /s/<scenario>/v1/chat/completions   reply #n of script[<scenario>]
//   POST /v1/chat/completions                 scenario `default`
//
// The script file is read on every request (a driver can change it between
// steps): { "<scenario>": [step, ...] }, a step one of
//   { "content": "text" }
//   { "tool_calls": [{ "id": "call_1", "name": "shell", "arguments": { ... } }] }
//   { "status": 503, "body": { "error": { ... } } }    an HTTP error
//   { "hang": true }                                    never answers
//   { "delay_ms": 200, "then": step }
// An exhausted script answers 400 (a permanent provider error).
// Every request is appended to the log file as one JSON line
// { scenario, n, body }.
//
// Runs under Node and Deno (`node:http`):
//   node mock_openai.mjs --script script.json [--log requests.jsonl] [--port 0]
//   deno run --allow-net --allow-read --allow-write mock_openai.mjs --script …
// and prints `listening <port>` once it accepts connections.
import { createServer } from 'node:http'
import { appendFileSync, readFileSync } from 'node:fs'

function reply(step, n) {
  if (step.tool_calls) {
    return {
      role: 'assistant',
      content: step.content ?? null,
      tool_calls: step.tool_calls.map((c, i) => ({
        id: c.id ?? `call_${n}_${i + 1}`,
        type: 'function',
        function: { name: c.name, arguments: JSON.stringify(c.arguments ?? {}) },
      })),
    }
  }
  return { role: 'assistant', content: step.content ?? '' }
}

export function startMockOpenai({ script, log, port = 0, host = '127.0.0.1' }) {
  const counters = new Map()
  const server = createServer((req, res) => {
    let raw = ''
    req.on('data', c => { raw += c })
    req.on('end', async () => {
      const m = /^\/(?:s\/([^/]+)\/)?v1\/chat\/completions$/.exec(req.url ?? '')
      if (req.method !== 'POST' || !m) {
        res.writeHead(404, { 'content-type': 'application/json' })
        res.end(JSON.stringify({ error: { message: `no route ${req.method} ${req.url}` } }))
        return
      }
      const scenario = m[1] ?? 'default'
      const n = (counters.get(scenario) ?? 0) + 1
      counters.set(scenario, n)
      let body = null
      try {
        body = JSON.parse(raw)
      } catch {
        body = raw
      }
      if (log) appendFileSync(log, JSON.stringify({ scenario, n, body }) + '\n')
      const scripts = typeof script === 'string' ? JSON.parse(readFileSync(script, 'utf8')) : script
      let step = (scripts[scenario] ?? [])[n - 1]
      while (step && step.delay_ms !== undefined) {
        await new Promise(r => setTimeout(r, step.delay_ms))
        step = step.then
      }
      if (!step) {
        res.writeHead(400, { 'content-type': 'application/json' })
        res.end(JSON.stringify({ error: { message: `mock script ${scenario} has no reply #${n}`, code: 'mock_exhausted' } }))
        return
      }
      if (step.hang) return
      if (step.status !== undefined) {
        res.writeHead(step.status, { 'content-type': 'application/json' })
        res.end(JSON.stringify(step.body ?? { error: { message: `mock status ${step.status}` } }))
        return
      }
      res.writeHead(200, { 'content-type': 'application/json' })
      res.end(JSON.stringify({
        id: `mock-${scenario}-${n}`,
        object: 'chat.completion',
        model: body?.model ?? 'mock-model',
        choices: [{ index: 0, message: reply(step, n), finish_reason: step.tool_calls ? 'tool_calls' : 'stop' }],
        usage: { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 },
      }))
    })
  })
  return new Promise(resolve => {
    server.listen(port, host, () => resolve({ server, port: server.address().port, close: () => new Promise(r => server.close(r)) }))
  })
}

const isMain = (() => {
  try {
    if (typeof Deno !== 'undefined') return import.meta.main
    return process.argv[1] && new URL(import.meta.url).pathname === process.argv[1]
  } catch {
    return false
  }
})()

if (isMain) {
  const args = typeof Deno !== 'undefined' ? Deno.args : process.argv.slice(2)
  const opt = name => {
    const i = args.indexOf(`--${name}`)
    return i >= 0 ? args[i + 1] : undefined
  }
  const script = opt('script')
  if (!script) {
    console.error('usage: mock_openai.mjs --script <file.json> [--log <file.jsonl>] [--port <n>]')
    if (typeof Deno !== 'undefined') Deno.exit(2)
    else process.exit(2)
  }
  const { port } = await startMockOpenai({ script, log: opt('log'), port: Number(opt('port') ?? 0) })
  console.log(`listening ${port}`)
}
