// Builds and runs the real `aiworkspace` backend in standalone mode for the e2e suite:
// a temporary data folder, static test identities, the port given by AIWS_E2E_BACKEND_PORT.
import { execFileSync, spawn } from 'node:child_process'
import { mkdtempSync, rmSync, writeFileSync, existsSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { createServer } from 'node:net'
import { startMockLlm } from './mock-llm.mjs'

const here = dirname(fileURLToPath(import.meta.url))
const workspace = resolve(here, '../../../..') // buckyos/src
const port = process.env.AIWS_E2E_BACKEND_PORT
if (!port) throw new Error('AIWS_E2E_BACKEND_PORT is not set (run through playwright.aiworkspace.config.ts)')

let binary = process.env.AIWS_BIN
if (!binary) {
  execFileSync('cargo', ['build', '-p', 'aiworkspace'], { cwd: workspace, stdio: 'inherit' })
  const metadata = JSON.parse(execFileSync('cargo', ['metadata', '--format-version', '1', '--no-deps'], { cwd: workspace, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 }))
  binary = join(metadata.target_directory, 'debug', 'aiworkspace')
}
if (!existsSync(binary)) throw new Error(`aiworkspace binary not found: ${binary}`)

const root = mkdtempSync(join(tmpdir(), 'aiws-e2e-'))
const auth = join(root, 'tokens.json')
writeFileSync(auth, JSON.stringify({ tokens: { 'tok-alice': { principal: 'alice' }, 'tok-bob': { principal: 'bob' } } }))
// wish runs talk to a scripted OpenAI-compatible model (许愿格 §16.1: controlled model returns)
const llmPort = await new Promise((resolvePort) => { const s = createServer().listen(0, '127.0.0.1', () => { const p = s.address().port; s.close(() => resolvePort(p)) }) })
const llm = startMockLlm(llmPort)
const wishConfig = join(root, 'wish.json')
writeFileSync(wishConfig, JSON.stringify({ provider: { type: 'openai', base_url: `http://127.0.0.1:${llmPort}/v1`, api_key_env: 'AIWS_E2E_LLM_KEY' }, analyze_model: 'scripted', execute_model: 'scripted', map_model: 'scripted', program_timeout_secs: 60 }))
const env = { ...process.env, AIWS_E2E_LLM_KEY: 'e2e' }
delete env.AIWS_FAILPOINT
const child = spawn(binary, ['--data-dir', join(root, 'data'), '--listen', `127.0.0.1:${port}`, '--auth-file', auth, '--fixture-sources', '--wish-config', wishConfig, '--log-level', 'warn'], { stdio: 'inherit', env })

let stopping = false
const stop = () => {
  if (stopping) return
  stopping = true
  child.kill('SIGKILL')
  llm.close()
  try { rmSync(root, { recursive: true, force: true }) } catch { /* best effort */ }
}
child.on('exit', (code) => { stop(); process.exit(stopping ? 0 : code ?? 1) })
for (const signal of ['SIGTERM', 'SIGINT', 'SIGHUP']) process.on(signal, () => { stop(); process.exit(0) })
process.on('exit', stop)
