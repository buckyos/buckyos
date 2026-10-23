import assert from 'node:assert/strict'
import { readFile, writeFile } from 'node:fs/promises'
import { spawn, execFile, type ChildProcessWithoutNullStreams } from 'node:child_process'
import { promisify } from 'node:util'
// @deno-types="../../../buckyos-websdk/dist/node.d.ts"
import { ExecutorClient } from '../../../buckyos-websdk/dist/node.mjs'
import type { ExecutorClient as ExecutorClientApi, ExecutorCommandStream, ExecutorAccess, ExecutorTarget, ExecutorOperationLocator } from '../../../buckyos-websdk/dist/executor_client.d.ts'

interface E2EConfig {
  control_url: string; owner_token: string; agent_token: string; agent_did: string
  ssh_key: string; known_hosts: string; vm_ip: string; remote_binary: string
  launcher_command?: string; report_file: string; platform?: string; vm_id?: number; reconnect_check?: boolean
}

const config: E2EConfig = JSON.parse(await readFile(process.argv[2], 'utf8'))
const admin: ExecutorClientApi = new ExecutorClient(config.control_url, config.owner_token)
const agent: ExecutorClientApi = new ExecutorClient(config.control_url, config.agent_token)
const sshArgs = ['-i', config.ssh_key, '-o', `UserKnownHostsFile=${config.known_hosts}`, '-o', 'BatchMode=yes', `agenttester@${config.vm_ip}`]
const execute = promisify(execFile)
const quote = (value: string) => `'${String(value).replaceAll("'", "'\\''")}'`
const delay = (ms: number) => new Promise(resolve => setTimeout(resolve, ms))
const report = { platform: config.platform ?? 'Linux x86_64 musl', vm_id: config.vm_id ?? 111, vm_ip: config.vm_ip, results: [] as string[] }
const processes = new Set<ChildProcessWithoutNullStreams>()
let created: ExecutorAccess | undefined
let stream: ExecutorCommandStream | undefined
const record = (name: string) => { report.results.push(name); process.stdout.write(`PASS ${name}\n`) }

async function ssh(command: string) {
  return (await execute('ssh', [...sshArgs, command], { timeout: 30000 })).stdout.trim()
}
async function until<T>(fn: () => Promise<T | false | undefined> | T | false | undefined, timeout = 20000): Promise<T> {
  const end = Date.now() + timeout
  while (Date.now() < end) { const value = await fn(); if (value) return value; await delay(250) }
  throw new Error('E2E condition timed out')
}
async function online(id: string) { return (await agent.listOnline()).executors.find(item => item.executor_id === id) }
async function target(id: string): Promise<ExecutorTarget> {
  const item = await until(() => online(id))
  return { executor_id: item.executor_id, generation: item.generation, profile_id: 'user', store_id: item.store_id }
}
function locator(selected: ExecutorTarget, name: string): ExecutorOperationLocator {
  return { ...selected, op_id: crypto.randomUUID(), run_id: 'pve-e2e', call_id: name }
}
async function launch(url: string, extra = '') {
  const child = spawn('ssh', [...sshArgs, `${config.launcher_command ?? quote(config.remote_binary)} ${extra} run --url-stdin`], { stdio: ['pipe', 'pipe', 'pipe'] })
  processes.add(child)
  child.once('exit', () => processes.delete(child))
  child.stdin.end(url + '\n')
  let output = ''
  child.stdout.on('data', bytes => { output += bytes.toString() })
  child.stderr.on('data', bytes => { output += bytes.toString() })
  child.once('exit', code => {
    if (code) process.stderr.write(`daemon exited ${code}: ${output.replaceAll(url, '[access URL redacted]')}\n`)
  })
  return child
}
async function terminal(client: ExecutorCommandStream, loc: ExecutorOperationLocator) {
  return until(async () => {
    const reply = await client.query(loc, undefined, 0, 500)
    assert.ok(reply.op)
    return !['starting', 'running'].includes(reply.op.state) && reply
  })
}
async function content(client: ExecutorCommandStream, loc: ExecutorOperationLocator, channel: 'content' | 'stdout' | 'stderr' = 'content') {
  const reply = await client.query(loc, channel)
  return Buffer.from(reply.chunk?.data_b64 ?? '', 'base64').toString()
}
async function stop(id: string) {
  await ssh(`${quote(config.remote_binary)} stop --executor ${quote(id)}`)
  await until(async () => !await online(id))
}

try {
  const identity = await ssh('id -u; test ! -e /opt/buckyos; command -v docker || true; command -v node || true')
  assert.equal(identity, '1000')
  const doctor = JSON.parse(await ssh(`${quote(config.remote_binary)} doctor --json`))
  assert.equal(doctor.capabilities.ui.state, 'unsupported')
  assert.equal(doctor.identity.uid, 1000)
  record('clean target, current ordinary user and Linux UI unsupported')
  created = await admin.createDevice({
    name: `PVE Agent Daemon E2E ${Date.now()}`,
    profiles: { user: { workdir_roots: ['/home/agenttester'], principals: [config.agent_did], ops: ['open_stream', 'describe', 'exec', 'read', 'write', 'edit', 'query', 'cancel'] } },
  })
  const id = created.device.executor_id
  assert.equal(await online(id), undefined)
  record('created device is invisible before launch')
  await launch(created.access_url)
  let selected = await target(id)
  stream = await agent.connect(selected, config.agent_token, 'pve-sdk-e2e', 'pve-e2e')
  const description = await stream.describe('/home/agenttester')
  assert.equal((description.info as { os: string }).os, 'linux')
  record('verified WSS, authenticated App login and online discovery')
  const command = locator(selected, 'exec')
  await writeFile(config.report_file + '.locator.json', JSON.stringify(command), { mode: 0o600 })
  await stream.exec(command, 'printf "%s:%s:%s" "$(id -u)" "$PWD" "$E2E_VALUE"', '/home/agenttester', { E2E_VALUE: 'passed' })
  assert.equal((await terminal(stream, command)).op?.state, 'succeeded')
  assert.equal(await content(stream, command, 'stdout'), '1000:/home/agenttester:passed')
  record('exec cwd, environment and uid')
  const path = '/home/agenttester/e2e-user-file.txt'
  const write = locator(selected, 'write')
  await stream.write(write, path, new TextEncoder().encode('hello world\n'))
  assert.equal((await terminal(stream, write)).op?.state, 'succeeded')
  const edit = locator(selected, 'edit')
  await stream.edit(edit, path, [{ old_string: 'world', new_string: 'executor' }])
  assert.equal((await terminal(stream, edit)).op?.state, 'succeeded')
  const read = locator(selected, 'read')
  await stream.read(read, path)
  assert.equal((await terminal(stream, read)).op?.state, 'succeeded')
  assert.equal(await content(stream, read), 'hello executor\n')
  record('write, edit, read and result query')
  const outside = locator(selected, 'forbidden-path')
  await stream.read(outside, '/etc/passwd')
  assert.equal((await terminal(stream, outside)).op?.state, 'failed')
  assert.equal(await content(stream, outside), '')
  record('file roots enforced')
  const long = locator(selected, 'cancel')
  await stream.exec(long, 'sleep 60', '/home/agenttester')
  await stream.cancel(long)
  assert.equal((await terminal(stream, long)).op?.state, 'cancelled')
  record('long command cancellation')
  if (config.reconnect_check) {
    const interrupted = locator(selected, 'transport-loss')
    const marker = `/home/agenttester/e2e-no-replay-${interrupted.op_id}`
    await stream.exec(interrupted, `sleep 2; printf once >> ${marker}`, '/home/agenttester')
    const endpoint = new URL(config.control_url)
    await execute('qm', ['guest', 'exec', String(config.vm_id ?? 111), '--', '/usr/bin/ss', '-K', 'dst', endpoint.hostname, 'dport', '=', `:${endpoint.port || 443}`], { timeout: 15000 })
    await until(async () => (await online(id))?.generation !== selected.generation && !!await online(id))
    const reconnected = await target(id)
    assert.equal(reconnected.store_id, selected.store_id)
    await assert.rejects(() => agent.openRoute(selected))
    stream.close()
    selected = reconnected
    stream = await agent.connect(selected, config.agent_token, 'pve-sdk-e2e', 'pve-e2e')
    assert.equal((await terminal(stream, interrupted)).op?.state, 'succeeded')
    assert.equal(await ssh(`cat ${quote(marker)}`), 'once')
    record('management transport loss reconnects with a new generation and no action replay')
  }
  const duplicate = await launch(created.access_url, '--data-dir /home/agenttester/duplicate-ledger')
  await until(() => duplicate.exitCode !== null)
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
  stream = await agent.connect(selected, config.agent_token, 'pve-sdk-e2e', 'pve-e2e')
  assert.equal((await stream.query(command)).op?.state, 'succeeded')
  assert.equal(await content(stream, command, 'stdout'), '1000:/home/agenttester:passed')
  assert.throws(() => stream!.exec(command, 'false', '/home/agenttester'))
  record('restart retains ledger; old generation rejects dispatch; previous result recovers')
  await admin.revokeAccess(id)
  await until(async () => !await online(id))
  await assert.rejects(() => agent.openRoute(selected))
  stream.close()
  record('revocation closes online route')
  const expired = await admin.issueAccess(id, Date.now() + 7000)
  await launch(expired.access_url)
  const expiringTarget = await target(id)
  const expiryStart = Date.now()
  await until(async () => !await online(id), 15000)
  await assert.rejects(() => agent.openRoute(expiringTarget))
  assert.ok(Date.now() - expiryStart < 12000)
  record('absolute URL expiry removes live executor')
  assert.equal(await ssh(`cat ${quote(path)}`), 'hello executor')
  record('stop and revocation preserve business files')
} finally {
  stream?.close()
  if (created) {
    try { await ssh(`${quote(config.remote_binary)} stop --executor ${quote(created.device.executor_id)}`) } catch {}
    await admin.deleteDevice(created.device.executor_id).catch(() => {})
  }
  for (const child of processes) child.kill('SIGTERM')
  await writeFile(config.report_file, JSON.stringify(report, null, 2) + '\n')
}
