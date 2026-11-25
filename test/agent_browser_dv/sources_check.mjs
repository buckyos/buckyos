#!/usr/bin/env node
// The reading-site sources that need the browser, end to end and without an Agent: starts the test site
// (site.mjs, port 18741 — the address the two packages under sources/ are written for) and browser_stub.mjs,
// then runs `site try` on both packages through every state: drawn pages published; the members' column
// refused while signed out (needs_login) and published once the stub's browser signed in; both failing as
// `program` after the site changed its markup a little (2) and completely (3).
//
//   node sources_check.mjs            exit code 0 when every step is as expected
//
// Needs deno and node on PATH, the node's ext-core (playwright-core, Chromium; see browser_stub.mjs for the
// environment that says where) and the Docker bridge address 172.17.0.1 on this host.
import { spawn } from 'node:child_process'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const tool = join(here, '../../src/tools/buckyos-agent')
const out = mkdtempSync(join(tmpdir(), 'ab-dv-sources-'))
const SITE = 'http://172.17.0.1:18741'
const STUB = 'http://127.0.0.1:18750/kapi/stub'
const PASSWORD = 'Pw-dvc-83Kq7Lm2'
const sleep = (ms) => new Promise((done) => setTimeout(done, ms))

const children = []
function start(file, args) {
  const child = spawn(process.execPath, [join(here, file), ...args], { stdio: ['ignore', 'pipe', 'inherit'] })
  children.push(child)
  return new Promise((ready, failed) => {
    child.stdout.once('data', ready)
    child.once('exit', (code) => failed(new Error(`${file} ended with ${code} before it was ready (is its port in use?)`)))
  })
}

function run(command, args, env = {}) {
  return new Promise((done) => {
    const child = spawn(command, args, { env: { ...process.env, ...env }, stdio: ['ignore', 'pipe', 'pipe'] })
    let stdout = ''
    child.stdout.on('data', (chunk) => { stdout += chunk })
    child.stderr.on('data', () => {})
    child.on('exit', (code) => done({ code, stdout }))
  })
}

async function siteTry(source) {
  const { code, stdout } = await run('deno', [
    'run', '-A', '--config', join(tool, 'deno.json'), join(tool, 'aicc-tool.ts'),
    'site', 'try', join(here, 'sources', source), '--no-translate', '--out', join(out, source),
  ], { OPENDAN_BROWSER_RPC: STUB, BUCKYOS_APPCLIENT_SESSION_TOKEN: 'x' })
  const result = JSON.parse(stdout.trim().split('\n').at(-1))
  const status = result.detail?.status ?? {}
  return { code, title: result.title, published: result.detail?.metrics?.published ?? 0, state: status.state, error: status.error?.kind ?? null, login_url: status.login_url ?? null, summary: result.summary.split('\n')[0] }
}

let failed = 0
function expect(what, got, want) {
  const ok = Object.entries(want).every(([key, value]) => got[key] === value)
  if (!ok) failed += 1
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}: ${got.summary}${ok ? '' : `\n     wanted ${JSON.stringify(want)}, got ${JSON.stringify(got)}`}`)
}

try {
  await start('site.mjs', ['18741', join(out, 'site')])
  await start('browser_stub.mjs', ['--port', '18750', '--profile', join(out, 'profile')])
  const markup = (v) => fetch(`${SITE}/__markup?v=${v}`).then((r) => r.text())
  const published = { code: 0, title: 'site try => done', published: 3, state: 'ok', error: null }
  const broken = { code: 2, title: 'site try => program', published: 0, state: 'failed', error: 'program' }

  expect('drawn pages are read in the browser', await siteTry('dv-live-news'), published)
  expect('signed out, the members\' column asks to sign in', await siteTry('dv-members'), { code: 2, title: 'site try => auth', state: 'needs_login', error: 'auth', login_url: `${SITE}/members` })
  const signedIn = await run(process.execPath, [join(here, 'browser_stub.mjs'), 'run', join(here, 'sign_in.mjs'), '--rpc', STUB, '--arg', `url=${SITE}/login`, '--arg', 'username=dvuser', '--arg', `password=${PASSWORD}`])
  expect('the owner signs in', { code: signedIn.code, summary: signedIn.stdout.trim() }, { code: 0 })
  expect('signed in, the members\' column is read with the browser\'s cookies', await siteTry('dv-members'), published)
  for (const version of [2, 3]) {
    await markup(version)
    const how = version === 2 ? 'a little' : 'completely'
    expect(`the site changed ${how}: the drawn pages`, await siteTry('dv-live-news'), broken)
    expect(`the site changed ${how}: the members' column`, await siteTry('dv-members'), broken)
  }
  await markup(1)
  expect('the site is as it was: the drawn pages again', await siteTry('dv-live-news'), published)
  await fetch(`${SITE}/__reset`)
  expect('the session ended: the members\' column asks again', await siteTry('dv-members'), { code: 2, state: 'needs_login', error: 'auth' })
} catch (e) {
  failed += 1
  console.log(`FAIL ${e.message}`)
} finally {
  for (const child of children) { child.removeAllListeners('exit'); child.kill() }
  await sleep(300)
  rmSync(out, { recursive: true, force: true })
}
console.log(failed ? `${failed} step(s) failed` : 'all steps as expected')
process.exit(failed ? 1 : 0)
