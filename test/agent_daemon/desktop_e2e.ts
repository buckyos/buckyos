// Desktop (macOS / Windows) acceptance for the standalone Agent Daemon.
// Shares the Linux flow of e2e.ts (access lifecycle, command/file ops) and adds
// the Computer protocol: inspect, observe (PNG via the ledger content channel),
// actions with feedback, and pause/resume. The daemon is launched inside the
// logged-in desktop session through a slot launcher (mac_launcher.sh in a
// Terminal window, or an interactive Task Scheduler task on Windows); every
// other remote call goes through SSH (macOS) or the PVE guest agent (Windows).
import assert from 'node:assert/strict'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { spawn } from 'node:child_process'
import path from 'node:path'
// @deno-types="../../../buckyos-websdk/dist/node.d.ts"
import { ExecutorClient } from '../../../buckyos-websdk/dist/node.mjs'
import type { ExecutorClient as ExecutorClientApi, ExecutorCommandStream, ExecutorAccess, ExecutorTarget, ExecutorOperationLocator, ExecutorReply } from '../../../buckyos-websdk/dist/executor_client.d.ts'

interface E2EConfig {
  control_url: string; owner_token: string; agent_token: string; agent_did: string
  platform: 'macos' | 'windows'; vm_id: number; report_dir: string
  remote_binary: string; home: string
  vm_ip?: string; ssh_key?: string; known_hosts?: string; ssh_user?: string
  win_remote?: string
}

const config: E2EConfig = JSON.parse(await readFile(process.argv[2], 'utf8'))
function execute(command: string, args: string[], input?: string, timeout = 180000): Promise<string> {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { stdio: ['pipe', 'pipe', 'pipe'] })
    let stdout = ''; let stderr = ''
    const timer = setTimeout(() => { child.kill('SIGKILL'); reject(new Error(`${command} timed out`)) }, timeout)
    child.stdout.on('data', bytes => { stdout += bytes.toString() })
    child.stderr.on('data', bytes => { stderr += bytes.toString() })
    child.once('error', error => { clearTimeout(timer); reject(error) })
    child.once('close', code => { clearTimeout(timer); code === 0 ? resolve(stdout) : reject(new Error(`${command} ${args[0]} exited ${code}: ${stderr.slice(0, 500)}`)) })
    child.stdin.end(input ?? '')
  })
}
const delay = (ms: number) => new Promise(resolve => setTimeout(resolve, ms))
const quote = (value: string) => `'${String(value).replaceAll("'", "'\\''")}'`
const psQuote = (value: string) => `'${String(value).replaceAll("'", "''")}'`
const win = config.platform === 'windows'
const join = (...parts: string[]) => parts.join(win ? '\\' : '/')
const home = config.home
const report = { platform: config.platform, vm_id: config.vm_id, started_at: new Date().toISOString(), results: [] as string[], notes: [] as string[] }
const record = (name: string) => { report.results.push(name); process.stdout.write(`PASS ${name}\n`) }
const note = (text: string) => { report.notes.push(text); process.stdout.write(`NOTE ${text}\n`) }

interface Remote {
  run(command: string): Promise<string>
  write(remotePath: string, text: string): Promise<void>
  launch(slot: string, extra: string[]): Promise<void>
  exited(slot: string): Promise<number | null>
  cli(args: string): Promise<string>
}

function macRemote(): Remote {
  const sshArgs = ['-i', config.ssh_key!, '-o', `UserKnownHostsFile=${config.known_hosts}`, '-o', 'BatchMode=yes', `${config.ssh_user ?? 'buckyos'}@${config.vm_ip}`]
  const ssh = (command: string, input?: string) => execute('ssh', [...sshArgs, command], input, 60000)
  const base = `${home}/bk-e2e`
  return {
    run: command => ssh(command),
    write: (remotePath, text) => ssh(`umask 077; cat > ${quote(remotePath)}`, text).then(() => {}),
    async launch(slot, extra) {
      const spec = `binary=${quote(config.remote_binary)}\nurl_file=${quote(`${base}/url-${slot}`)}\nextra=${quote(extra.join(' '))}\n`
      await ssh(`umask 077; cat > ${quote(`${base}/launch-${slot}.env`)}`, spec)
    },
    async exited(slot) {
      const text = await ssh(`cat ${quote(`${base}/daemon-${slot}.exit`)} 2>/dev/null || true`)
      return text.trim() === '' ? null : Number(text.trim())
    },
    cli: args => ssh(`${quote(config.remote_binary)} ${args}`),
  }
}

function windowsRemote(): Remote {
  const helper = config.win_remote ?? path.join(path.dirname(new URL(import.meta.url).pathname), 'win_remote.py')
  const call = (args: string[], input?: string) => execute('python3', ['-I', helper, String(config.vm_id), ...args], input)
  const base = 'C:\\bk\\e2e'
  return {
    run: command => call(['user', command]),
    write: (remotePath, text) => call(['write', remotePath], text).then(() => {}),
    launch: (slot, extra) => call(['launch', slot, config.remote_binary, `${base}\\url-${slot}`, ...extra]).then(() => {}),
    async exited(slot) { const text = (await call(['exited', slot])).trim(); return text === '' ? null : Number(text) },
    cli: args => call(['user', `& ${psQuote(config.remote_binary)} ${args}`]),
  }
}

const remote = win ? windowsRemote() : macRemote()
const urlBase = win ? 'C:\\bk\\e2e' : `${home}/bk-e2e`
const admin: ExecutorClientApi = new ExecutorClient(config.control_url, config.owner_token)
const agent: ExecutorClientApi = new ExecutorClient(config.control_url, config.agent_token)
let created: ExecutorAccess | undefined
let stream: ExecutorCommandStream | undefined
let slot = 0
const slots: string[] = []

async function until<T>(fn: () => Promise<T | false | undefined | null> | T | false | undefined | null, timeout = 30000, what = 'condition'): Promise<T> {
  const end = Date.now() + timeout
  while (Date.now() < end) { const value = await fn(); if (value) return value; await delay(500) }
  throw new Error(`E2E ${what} timed out`)
}
async function online(id: string) { return (await agent.listOnline()).executors.find(item => item.executor_id === id) }
async function target(id: string): Promise<ExecutorTarget> {
  const item = await until(() => online(id), 90000, 'executor online')
  return { executor_id: item.executor_id, generation: item.generation, profile_id: 'user', store_id: item.store_id }
}
function locator(selected: ExecutorTarget, name: string): ExecutorOperationLocator {
  return { ...selected, op_id: crypto.randomUUID(), run_id: 'desktop-e2e', call_id: name }
}
async function launch(url: string, extra: string[] = []) {
  const name = `s${++slot}`
  slots.push(name)
  await remote.write(`${urlBase}${win ? '\\' : '/'}url-${name}`, url + '\n')
  await remote.launch(name, extra)
  return name
}
async function terminal(client: ExecutorCommandStream, loc: ExecutorOperationLocator, timeout = 60000) {
  return until(async () => {
    const reply = await client.query(loc, undefined, 0, 500)
    assert.ok(reply.op)
    return !['starting', 'running'].includes(reply.op.state) && reply
  }, timeout, `operation ${loc.call_id}`)
}
async function content(client: ExecutorCommandStream, loc: ExecutorOperationLocator, channel: 'content' | 'stdout' | 'stderr' = 'content') {
  const chunks: Buffer[] = []
  let offset = 0
  for (;;) {
    const reply = await client.query(loc, channel, offset)
    const chunk = reply.chunk
    if (!chunk) break
    const bytes = Buffer.from(chunk.data_b64, 'base64')
    chunks.push(bytes); offset += bytes.length
    if (chunk.eof || bytes.length === 0) break
  }
  return Buffer.concat(chunks)
}
function computerOf(reply: ExecutorReply): Record<string, unknown> {
  const op = reply.op as Record<string, unknown>
  const result = (op.result ?? op) as Record<string, unknown>
  const computer = (result.computer ?? op.computer) as Record<string, unknown> | undefined
  assert.ok(computer, `no computer result in ${JSON.stringify(op).slice(0, 400)}`)
  return computer
}
async function computer(client: ExecutorCommandStream, selected: ExecutorTarget, name: string, request: Record<string, unknown>) {
  const loc = locator(selected, name)
  await client.request('computer', { store_id: loc.store_id, op_id: loc.op_id, run_id: loc.run_id, call_id: loc.call_id, request })
  const reply = await terminal(client, loc)
  return { loc, reply, state: reply.op!.state as string }
}
function pngSize(bytes: Buffer) {
  assert.equal(bytes.subarray(0, 8).toString('hex'), '89504e470d0a1a0a', 'not a PNG')
  return { width: bytes.readUInt32BE(16), height: bytes.readUInt32BE(20) }
}
async function stop(id: string) {
  await remote.cli(`stop --executor ${id}`)
  await until(async () => !await online(id), 30000, 'executor offline after stop')
}

await mkdir(config.report_dir, { recursive: true })
try {
  const doctor = JSON.parse(await remote.cli('doctor --json'))
  await writeFile(path.join(config.report_dir, 'doctor.json'), JSON.stringify(doctor, null, 2))
  assert.equal(doctor.capabilities.ui.supported, true)
  assert.equal(doctor.capabilities.ui.backend, win ? 'windows_desktop' : 'macos_desktop')
  assert.equal(doctor.identity.mode, 'current_user')
  note(`doctor: ui.state=${doctor.capabilities.ui.state} capture.ready=${doctor.capabilities.capture.ready} input.ready=${doctor.capabilities.input.ready}`)
  record('clean desktop target reports the platform desktop backend as current user')
  created = await admin.createDevice({
    name: `${config.platform} desktop E2E ${Date.now()}`,
    profiles: { user: { workdir_roots: [home], principals: [config.agent_did], ops: ['open_stream', 'describe', 'exec', 'read', 'write', 'edit', 'mkdir', 'stat', 'computer_inspect', 'computer_observe', 'computer_act', 'query', 'cancel'] } },
  })
  const id = created.device.executor_id
  assert.equal(await online(id), undefined)
  record('created device is invisible before launch')
  await launch(created.access_url)
  let selected = await target(id)
  stream = await agent.connect(selected, config.agent_token, 'desktop-sdk-e2e', 'desktop-e2e')
  const description = await stream.describe(home)
  assert.equal((description.info as { os: string }).os, win ? 'windows' : 'macos')
  await writeFile(path.join(config.report_dir, 'describe.json'), JSON.stringify(description, null, 2))
  record('verified production WSS entry, authenticated App login and online discovery')

  const command = locator(selected, 'exec')
  const script = win ? 'Write-Output "$env:USERNAME|$((Get-Location).Path)|$env:E2E_VALUE"' : 'printf "%s|%s|%s" "$(id -un)" "$PWD" "$E2E_VALUE"'
  await stream.exec(command, script, home, { E2E_VALUE: 'passed' })
  assert.equal((await terminal(stream, command)).op?.state, 'succeeded')
  const expected = `${win ? 'buckyos' : (await remote.run('id -un')).trim()}|${home}|passed`
  assert.equal((await content(stream, command, 'stdout')).toString().trim(), expected)
  record('exec cwd, environment and OS identity')

  const file = join(home, 'desktop-e2e-file.txt')
  const write = locator(selected, 'write')
  await stream.write(write, file, new TextEncoder().encode('hello world\n'))
  assert.equal((await terminal(stream, write)).op?.state, 'succeeded')
  const edit = locator(selected, 'edit')
  await stream.edit(edit, file, [{ old_string: 'world', new_string: 'executor' }])
  assert.equal((await terminal(stream, edit)).op?.state, 'succeeded')
  const read = locator(selected, 'read')
  await stream.read(read, file)
  assert.equal((await terminal(stream, read)).op?.state, 'succeeded')
  assert.equal((await content(stream, read)).toString(), 'hello executor\n')
  record('write, edit, read and result query')

  const outside = locator(selected, 'forbidden-path')
  await stream.read(outside, win ? 'C:\\Windows\\win.ini' : '/etc/passwd')
  assert.equal((await terminal(stream, outside)).op?.state, 'failed')
  record('file roots enforced')

  const long = locator(selected, 'cancel')
  await stream.exec(long, win ? 'Start-Sleep -Seconds 60' : 'sleep 60', home)
  await until(async () => (await stream!.query(long)).op?.state === 'running', 20000, 'long command running')
  await stream.cancel(long)
  assert.equal((await terminal(stream, long)).op?.state, 'cancelled')
  record('long command cancellation')

  const binding = `user:${doctor.capabilities.ui.surface_id}`
  note(`binding: ${binding}`)
  const inspect = await computer(stream, selected, 'inspect', { command: 'inspect', binding_id: binding })
  await writeFile(path.join(config.report_dir, 'inspect.json'), JSON.stringify(inspect.reply, null, 2))
  assert.equal(inspect.state, 'succeeded', JSON.stringify(inspect.reply).slice(0, 600))
  const inspected = computerOf(inspect.reply)
  const capabilities = inspected.capabilities as { capture: { ready: boolean; reason?: string }; input: { ready: boolean; reason?: string } }
  note(`inspect: capture.ready=${capabilities.capture.ready} (${capabilities.capture.reason ?? ''}) input.ready=${capabilities.input.ready} (${capabilities.input.reason ?? ''})`)
  assert.equal((inspected.binding as { binding_id: string }).binding_id, binding)
  record('computer inspect returns binding and live capabilities')
  assert.equal(capabilities.capture.ready, true, 'screen capture is not ready on the target desktop')
  assert.equal(capabilities.input.ready, true, 'desktop input is not ready on the target desktop')

  const observe = await computer(stream, selected, 'observe', { command: 'observe', binding_id: binding, mode: 'visual' })
  assert.equal(observe.state, 'succeeded', JSON.stringify(observe.reply).slice(0, 600))
  const observation = computerOf(observe.reply).observation as { observation_id: string; screenshot: { artifact_ref: string; width: number; height: number; mime_type: string } }
  assert.ok(observation.screenshot.artifact_ref.startsWith('executor-content:'))
  const png = await content(stream, observe.loc)
  const size = pngSize(png)
  assert.deepEqual(size, { width: observation.screenshot.width, height: observation.screenshot.height })
  await writeFile(path.join(config.report_dir, 'observe.png'), png)
  note(`observe: ${size.width}x${size.height} PNG, ${png.length} bytes`)
  record('computer observe delivers a real PNG through the ledger content channel')

  const centre = { x: Math.floor(size.width / 2), y: Math.floor(size.height / 2) }
  const click = await computer(stream, selected, 'click', { command: 'click', binding_id: binding, target: { kind: 'point', observation_id: observation.observation_id, ...centre, space: 'screenshot' }, feedback: 'after' })
  assert.equal(click.state, 'succeeded', JSON.stringify(click.reply).slice(0, 600))
  const clicked = computerOf(click.reply)
  assert.equal((clicked.feedback as { state: string }).state, 'included')
  const afterClick = clicked.observation as { observation_id: string }
  assert.ok(afterClick.observation_id && afterClick.observation_id !== observation.observation_id)
  await writeFile(path.join(config.report_dir, 'click.png'), await content(stream, click.loc))
  record('point click on the desktop with screenshot feedback returns a fresh observation')

  const focusWait = await computer(stream, selected, 'wait-for-focus', { command: 'wait_for', binding_id: binding, target: { kind: 'focus', observation_id: afterClick.observation_id }, condition: { kind: 'delay', duration_ms: 300 }, feedback: 'after' })
  const focusError = ((focusWait.reply.op as { result?: { error?: { message?: string } } }).result?.error?.message) ?? ''
  if (focusWait.state === 'succeeded') record('focus-targeted wait with feedback on the focused desktop')
  else { assert.match(focusError, /focus_unverifiable/, JSON.stringify(focusWait.reply).slice(0, 600)); note(`focus target on the bare desktop -> ${focusWait.state}: ${focusError}`); record('focus target without a verifiable focused element is refused, not guessed') }

  if (win) {
    const open = locator(selected, 'open-notepad')
    await stream.exec(open, 'Start-Process notepad; Start-Sleep -Seconds 3', home)
    assert.equal((await terminal(stream, open)).op?.state, 'succeeded')
    const editorObserve = await computer(stream, selected, 'observe-editor', { command: 'observe', binding_id: binding, mode: 'visual' })
    assert.equal(editorObserve.state, 'succeeded')
    const editorObservation = computerOf(editorObserve.reply).observation as { observation_id: string }
    const before = await content(stream, editorObserve.loc)
    const typed = await computer(stream, selected, 'type-editor', { command: 'type', binding_id: binding, target: { kind: 'focus', observation_id: editorObservation.observation_id }, text: 'buckyos desktop e2e', feedback: 'after' })
    assert.equal(typed.state, 'succeeded', JSON.stringify(typed.reply).slice(0, 600))
    const typedResult = computerOf(typed.reply)
    assert.equal((typedResult.feedback as { state: string }).state, 'included')
    const afterTyping = await content(stream, typed.loc)
    assert.notEqual(Buffer.compare(before, afterTyping), 0, 'screenshot did not change after typing')
    await writeFile(path.join(config.report_dir, 'typed.png'), afterTyping)
    const close = locator(selected, 'close-notepad')
    await stream.exec(close, 'Stop-Process -Name notepad -Force', home)
    assert.equal((await terminal(stream, close)).op?.state, 'succeeded')
    record('focus-targeted Unicode typing into Notepad with screenshot feedback')
  }

  const stale = await computer(stream, selected, 'stale-observation', { command: 'click', binding_id: binding, target: { kind: 'point', observation_id: observation.observation_id, x: 1, y: 1, space: 'screenshot' } })
  assert.notEqual(stale.state, 'succeeded')
  note(`stale observation click -> ${stale.state}: ${JSON.stringify((stale.reply.op as { result?: { error?: unknown } }).result?.error ?? '').slice(0, 200)}`)
  record('an action against a superseded observation is not dispatched')

  await remote.cli(`pause --executor ${id}`)
  const paused = await until(async () => {
    const fresh = await computer(stream!, selected, 'paused-observe', { command: 'observe', binding_id: binding, mode: 'visual' })
    const obs = fresh.state === 'succeeded' ? (computerOf(fresh.reply).observation as { observation_id: string }).observation_id : afterClick.observation_id
    const attempt = await computer(stream!, selected, 'paused-action', { command: 'click', binding_id: binding, target: { kind: 'point', observation_id: obs, ...centre, space: 'screenshot' } })
    return attempt.state !== 'succeeded' && attempt
  }, 30000, 'paused action rejected')
  note(`paused action -> ${paused.state}: ${JSON.stringify((paused.reply.op as { result?: { error?: unknown } }).result?.error ?? '').slice(0, 200)}`)
  await remote.cli(`resume --executor ${id}`)
  const resumed = await until(async () => {
    const fresh = await computer(stream!, selected, 'resumed-observe', { command: 'observe', binding_id: binding, mode: 'visual' })
    if (fresh.state !== 'succeeded') return false
    const obs = (computerOf(fresh.reply).observation as { observation_id: string }).observation_id
    const attempt = await computer(stream!, selected, 'resumed-action', { command: 'click', binding_id: binding, target: { kind: 'point', observation_id: obs, ...centre, space: 'screenshot' } })
    return attempt.state === 'succeeded' && { attempt, obs }
  }, 30000, 'action after resume')
  const resumedObservation = { observation_id: resumed.obs }
  record('local pause blocks desktop input and resume restores it')

  const duplicate = await launch(created.access_url, ['--data-dir', join(home, 'duplicate-ledger')])
  await until(() => remote.exited(duplicate).then(code => code !== null), 60000, 'duplicate instance exit')
  assert.equal((await online(id))?.generation, selected.generation)
  record('second independent instance refused')

  stream.close()
  await stop(id)
  record('normal stop immediately removes online target')

  await launch(created.access_url)
  const previous = selected
  selected = await target(id)
  assert.notEqual(selected.generation, previous.generation)
  assert.equal(selected.store_id, previous.store_id)
  await assert.rejects(() => agent.openRoute(previous))
  stream = await agent.connect(selected, config.agent_token, 'desktop-sdk-e2e', 'desktop-e2e')
  assert.equal((await stream.query(command)).op?.state, 'succeeded')
  assert.equal((await content(stream, command, 'stdout')).toString().trim(), expected)
  assert.equal(pngSize(await content(stream, observe.loc)).width, size.width)
  assert.throws(() => stream!.exec(command, 'false', home))
  const replay = await computer(stream, selected, 'old-observation-after-restart', { command: 'click', binding_id: binding, target: { kind: 'point', observation_id: resumedObservation.observation_id, ...centre, space: 'screenshot' } })
  assert.notEqual(replay.state, 'succeeded')
  record('restart retains ledger and screenshots; old generation and old observations reject dispatch')

  await admin.revokeAccess(id)
  await until(async () => !await online(id), 30000, 'offline after revocation')
  await assert.rejects(() => agent.openRoute(selected))
  stream.close()
  record('revocation closes online route')

  // Desktop launches go through a session launcher (Terminal loop / Task
  // Scheduler), so the URL must outlive the launch latency; expiry is still
  // absolute and measured from issue time.
  const issuedAt = Date.now()
  const expired = await admin.issueAccess(id, issuedAt + 45000)
  await launch(expired.access_url)
  const expiring = await target(id)
  note(`expiring executor online ${Date.now() - issuedAt} ms after issue`)
  await until(async () => !await online(id), 60000, 'expiry offline')
  await assert.rejects(() => agent.openRoute(expiring))
  assert.ok(Date.now() - issuedAt < 60000)
  record('absolute URL expiry removes live executor')

  const kept = win ? await remote.run(`Get-Content -Raw ${psQuote(file)}`) : await remote.run(`cat ${quote(file)}`)
  assert.equal(kept.trim(), 'hello executor')
  record('stop and revocation preserve business files')
} finally {
  stream?.close()
  if (created) {
    try { await remote.cli(`stop --executor ${created.device.executor_id}`) } catch {}
    await admin.deleteDevice(created.device.executor_id).catch(() => {})
  }
  report.notes.push(`slots: ${slots.join(', ')}`)
  await writeFile(path.join(config.report_dir, 'report.json'), JSON.stringify(report, null, 2) + '\n')
}
