#!/usr/bin/env node
// A stand-in for the Agent Service's `browser.background_run` (Agent Browser Protocol section 7.1), to run the
// source packages of a reading site that need the browser on a developer's machine, without an Agent:
// it starts a headless Chromium with a profile of its own and answers the kRPC call by running
// `agent-browser background-run` on it, one run at a time, as the Agent Service does.
//
//   node browser_stub.mjs [--port 18750] [--cdp-port 18751] [--profile <dir>] [--agent-browser <agent-browser.mjs>]
//       serve; then, in another shell:
//       OPENDAN_BROWSER_RPC=http://127.0.0.1:18750/kapi/stub BUCKYOS_APPCLIENT_SESSION_TOKEN=x site try <package> --no-translate
//   node browser_stub.mjs run <script.mjs> [--rpc <url>] [--url <url>] [--arg k=v]...
//       one browser.background_run through a stub that is serving (sign_in.mjs plays the owner signing in)
//
// Environment: AB_DV_CHROMIUM (default /opt/buckyos/tools/bin/chromium), NODE_PATH (where playwright-core is;
// default the node's ext-core). The token is not checked. Nothing here is the Agent Service: no lease, no tab
// groups, no idle stop.
import { spawn } from 'node:child_process'
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:http'
import { tmpdir } from 'node:os'
import { dirname, isAbsolute, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const argv = process.argv.slice(2)
const option = (name, fallback) => { const i = argv.indexOf(name); return i >= 0 ? argv[i + 1] : fallback }
const sleep = (ms) => new Promise((done) => setTimeout(done, ms))

if (argv[0] === 'run') {
  const script = argv[1]
  if (!script || script.startsWith('--')) { console.error('usage: browser_stub.mjs run <script.mjs> [--rpc <url>] [--url <url>] [--arg k=v]...'); process.exit(2) }
  const args = {}
  argv.forEach((word, i) => { if (word === '--arg') { const [key, ...value] = argv[i + 1].split('='); args[key] = value.join('=') } })
  const url = option('--url')
  const response = await fetch(option('--rpc', 'http://127.0.0.1:18750/kapi/stub'), {
    method: 'POST', headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ method: 'browser.background_run', params: { script_path: resolve(script), ...(url ? { url } : {}), args }, sys: [Date.now(), 'x'] }),
  })
  const answer = await response.json()
  console.log(JSON.stringify(answer.error === undefined ? answer.result : { error: answer.error }))
  process.exit(answer.error === undefined ? 0 : 1)
}

const port = Number(option('--port', 18750))
const cdpPort = Number(option('--cdp-port', 18751))
const profile = resolve(option('--profile', mkdtempSync(join(tmpdir(), 'ab-dv-profile-'))))
const agentBrowser = resolve(option('--agent-browser', join(here, '../../publish/exttool/ext-core/browser/agent-browser.mjs')))
const chromium = process.env.AB_DV_CHROMIUM ?? '/opt/buckyos/tools/bin/chromium'
const nodePath = process.env.NODE_PATH ?? '/opt/buckyos/tools/store/ext-core/playwright/node_modules'
const cdp = `http://127.0.0.1:${cdpPort}`
const work = mkdtempSync(join(tmpdir(), 'ab-dv-job-'))
mkdirSync(profile, { recursive: true })

const browser = spawn(chromium, [
  '--headless=new', `--remote-debugging-port=${cdpPort}`, `--user-data-dir=${profile}`,
  '--no-first-run', '--no-default-browser-check', '--disable-gpu', '--no-sandbox', 'about:blank',
], { stdio: ['ignore', 'ignore', 'pipe'] })
let browserLog = ''
browser.stderr.on('data', (chunk) => { browserLog = (browserLog + chunk).slice(-4000) })
browser.on('exit', (code) => { console.error(`the browser exited (${code})\n${browserLog}`); process.exit(1) })
const stop = () => { browser.removeAllListeners('exit'); browser.kill(); rmSync(work, { recursive: true, force: true }); process.exit(0) }
process.on('SIGINT', stop)
process.on('SIGTERM', stop)

for (let tries = 0; ; tries += 1) {
  if (await fetch(`${cdp}/json/version`).then((r) => r.ok, () => false)) break
  if (tries === 100) { console.error(`the browser does not answer on ${cdp}\n${browserLog}`); browser.kill(); process.exit(1) }
  await sleep(100)
}

const failure = (code, message, kind = 'other') => ({ error: JSON.stringify({ kind, code, message }) })

let runs = 0
async function backgroundRun(params) {
  const { script_path: script, url, args, timeout_s: timeout = 60 } = params ?? {}
  if (typeof script !== 'string' || !isAbsolute(script) || !script.endsWith('.mjs') || !existsSync(script)) {
    return failure('invalid_argument', `script_path must be the absolute path of an existing .mjs file: ${script}`, 'invalid_argument')
  }
  if (!Number.isInteger(timeout) || timeout < 1 || timeout > 300) return failure('invalid_argument', 'timeout_s must be 1 to 300', 'invalid_argument')
  const out = join(work, `out-${runs += 1}.json`)
  const command = [agentBrowser, 'background-run', '--cdp', cdp, '--script', script, '--out', out, '--timeout', String(timeout)]
  if (url) command.push(`--url=${url}`)
  if (args !== undefined && args !== null) {
    writeFileSync(`${out}.args`, JSON.stringify(args))
    command.push('--args', `${out}.args`)
  }
  const child = spawn(process.execPath, command, { cwd: work, env: { ...process.env, NODE_PATH: nodePath }, stdio: ['ignore', 'ignore', 'pipe'] })
  let stderr = ''
  child.stderr.on('data', (chunk) => { stderr += chunk })
  const killer = setTimeout(() => child.kill('SIGKILL'), (timeout + 10) * 1000)
  const code = await new Promise((done) => child.on('exit', done))
  clearTimeout(killer)
  if (code === 0) return { result: { data: JSON.parse(readFileSync(out, 'utf8')) } }
  // What the Agent Service passes on: the last line the command wrote.
  return failure('other', stderr.trim().split('\n').at(-1) || `background-run ended with ${code}`)
}

let queue = Promise.resolve()
const server = createServer(async (req, res) => {
  const chunks = []
  for await (const chunk of req) chunks.push(chunk)
  let call
  try { call = JSON.parse(Buffer.concat(chunks).toString('utf8')) } catch { call = {} }
  const answer = call.method === 'browser.background_run'
    ? await (queue = queue.then(() => backgroundRun(call.params), () => backgroundRun(call.params)))
    : failure('other', `this stand-in only answers browser.background_run, not ${call.method}`)
  console.error(`${call.method} ${call.params?.script_path ?? ''} ${call.params?.url ?? ''} => ${answer.error ?? 'ok'}`)
  res.writeHead(200, { 'content-type': 'application/json' })
  res.end(JSON.stringify({ ...answer, sys: [call.sys?.[0] ?? 0] }))
})
server.listen(port, '127.0.0.1', () => console.log(`browser.background_run at http://127.0.0.1:${port}/kapi/stub (browser ${cdp}, profile ${profile})`))
