// Builds and runs the real `aiworkspace` backend in standalone mode for the e2e suite:
// a temporary data folder, static test identities, the port given by AIWS_E2E_BACKEND_PORT.
// Objects live in a standalone nfs-server (port AIWS_E2E_NDM_PORT) that declares the service's
// buckets (`--bucket-specs`, the same as `buckyos_api::aiworkspace_bucket_decls`): the Desktop
// uploads into it through NDM and reads downloads from its `/bucket/v1` data plane.
import { execFileSync, spawn } from 'node:child_process'
import { mkdtempSync, mkdirSync, rmSync, writeFileSync, existsSync } from 'node:fs'
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
let nfsBinary = process.env.NFS_BIN
if (!nfsBinary) {
  execFileSync('cargo', ['build', '-p', 'nfs_server'], { cwd: workspace, stdio: 'inherit' })
  const metadata = JSON.parse(execFileSync('cargo', ['metadata', '--format-version', '1', '--no-deps'], { cwd: workspace, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 }))
  nfsBinary = join(metadata.target_directory, 'debug', 'nfs_server')
}
const ndmBase = `http://127.0.0.1:${process.env.AIWS_E2E_NDM_PORT}`
const store = join(root, 'named-store')
const exported = join(root, 'files')
mkdirSync(exported, { recursive: true })
const storeConfig = join(root, 'named-store.json')
writeFileSync(storeConfig, JSON.stringify({ stores: [{ path: store }] }))
const bucketSpecs = join(root, 'bucket-specs.json')
writeFileSync(bucketSpecs, JSON.stringify({
  aiworkspace: {
    storage_quota_mib: 16384,
    buckets: {
      assets: { kind: 'data', data_policy: 'app_data', scope: 'user' },
      objects: { kind: 'data', data_policy: 'app_data', scope: 'user' },
      exports: { kind: 'data', data_policy: 'temp', scope: 'user', default_ttl_days: 7 },
      library: { kind: 'data', data_policy: 'app_data', scope: 'user' },
    },
  },
}))
const nfs = spawn(nfsBinary, ['--data-dir', join(root, 'nfs'), '--export', `home=${exported}`, '--named-store', store, '--bucket-specs', bucketSpecs, '--listen', `127.0.0.1:${process.env.AIWS_E2E_NDM_PORT}`, '--log-level', 'warn'], { stdio: 'inherit' })
const auth = join(root, 'tokens.json')
// a Limited account (restricted session) and an External DID stand for the hybrid SSO visitors (entries.spec.ts)
writeFileSync(auth, JSON.stringify({
  tokens: {
    'tok-alice': { principal: 'alice', auth_method: 'password' },
    'tok-bob': { principal: 'bob', auth_method: 'password' },
    'tok-lim': { principal: 'acme-c1', restricted: true, auth_method: 'password' },
    'tok-ext': { principal: 'did:bns:carol', class: 'external', auth_method: 'did_federation' },
  },
  sso: { self_register_enabled: true, did_login_enabled: false },
}))
// wish runs talk to a scripted OpenAI-compatible model (许愿格 §16.1: controlled model returns)
// the port is chosen by playwright.aiworkspace.config.ts: the local executor spec reaches the same model from the browser
const llmPort = process.env.AIWS_E2E_LLM_PORT ?? await new Promise((resolvePort) => { const s = createServer().listen(0, '127.0.0.1', () => { const p = s.address().port; s.close(() => resolvePort(p)) }) })
const llm = startMockLlm(llmPort)
const wishConfig = join(root, 'wish.json')
writeFileSync(wishConfig, JSON.stringify({ provider: { type: 'openai', base_url: `http://127.0.0.1:${llmPort}/v1`, api_key_env: 'AIWS_E2E_LLM_KEY' }, analyze_model: 'scripted', execute_model: 'scripted', map_model: 'scripted', program_timeout_secs: 60 }))
const env = { ...process.env, AIWS_E2E_LLM_KEY: 'e2e' }
// a local run whose browser went away is interrupted within seconds (the browser heartbeats at a third of the lease)
env.AIWS_RUN_LEASE_MS ??= '6000'
delete env.AIWS_FAILPOINT
// workspace pages live on `aiws-<label>.localhost:<dev server port>` (T4-8): the dev server serves aiws.html there
const publicBase = process.env.AIWS_E2E_PUBLIC_BASE ? ['--public-base', process.env.AIWS_E2E_PUBLIC_BASE] : []
const child = spawn(binary, ['--data-dir', join(root, 'data'), '--listen', `127.0.0.1:${port}`, '--auth-file', auth, '--fixture-sources', '--wish-config', wishConfig, '--nfs-server', ndmBase, '--named-store-config', storeConfig, ...publicBase, '--log-level', 'warn'], { stdio: 'inherit', env })

let stopping = false
const stop = () => {
  if (stopping) return
  stopping = true
  child.kill('SIGKILL')
  nfs.kill('SIGKILL')
  llm.close()
  try { rmSync(root, { recursive: true, force: true }) } catch { /* best effort */ }
}
child.on('exit', (code) => { stop(); process.exit(stopping ? 0 : code ?? 1) })
for (const signal of ['SIGTERM', 'SIGINT', 'SIGHUP']) process.on(signal, () => { stop(); process.exit(0) })
process.on('exit', stop)
