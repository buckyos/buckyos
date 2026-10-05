// Builds and runs the real `aiworkspace` backend in standalone mode for the e2e suite:
// a temporary data folder, static test identities, the port given by AIWS_E2E_BACKEND_PORT.
import { execFileSync, spawn } from 'node:child_process'
import { mkdtempSync, rmSync, writeFileSync, existsSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

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
const env = { ...process.env }
delete env.AIWS_FAILPOINT
const child = spawn(binary, ['--data-dir', join(root, 'data'), '--listen', `127.0.0.1:${port}`, '--auth-file', auth, '--fixture-sources', '--log-level', 'warn'], { stdio: 'inherit', env })

let stopping = false
const stop = () => {
  if (stopping) return
  stopping = true
  child.kill('SIGKILL')
  try { rmSync(root, { recursive: true, force: true }) } catch { /* best effort */ }
}
child.on('exit', (code) => { stop(); process.exit(stopping ? 0 : code ?? 1) })
for (const signal of ['SIGTERM', 'SIGINT', 'SIGHUP']) process.on(signal, () => { stop(); process.exit(0) })
process.on('exit', stop)
