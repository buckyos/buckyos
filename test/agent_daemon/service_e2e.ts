import assert from 'node:assert/strict'
import { readFile, writeFile } from 'node:fs/promises'
import { spawn, execFile } from 'node:child_process'
import { promisify } from 'node:util'
// @deno-types="../../../buckyos-websdk/dist/node.d.ts"
import { ExecutorClient } from '../../../buckyos-websdk/dist/node.mjs'
import type { ExecutorAccess } from '../../../buckyos-websdk/dist/executor_client.d.ts'

const config = JSON.parse(await readFile(process.argv[2], 'utf8')) as {
  control_url: string; owner_token: string; agent_token: string; agent_did: string
  ssh_key: string; known_hosts: string; vm_ip: string; remote_binary: string; report_file: string
}
const admin = new ExecutorClient(config.control_url, config.owner_token)
const agent = new ExecutorClient(config.control_url, config.agent_token)
const args = ['-i', config.ssh_key, '-o', `UserKnownHostsFile=${config.known_hosts}`, '-o', 'BatchMode=yes', `agenttester@${config.vm_ip}`]
const execute = promisify(execFile)
const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`
const script = config.remote_binary.replace(/\/[^/]+$/, '/service.sh')
const run = async (command: string) => (await execute('ssh', [...args, command], { timeout: 30000 })).stdout.trim()
const delay = (ms: number) => new Promise(resolve => setTimeout(resolve, ms))
const results: string[] = []
const record = (name: string) => { results.push(name); console.log(`PASS ${name}`) }
let created: ExecutorAccess | undefined

async function until(fn: () => Promise<boolean>, timeout = 20000) {
  const end = Date.now() + timeout
  while (Date.now() < end) { if (await fn()) return; await delay(250) }
  throw new Error('Service lifecycle condition timed out')
}
async function online() { return (await agent.listOnline()).executors.find(item => item.executor_id === created?.device.executor_id) }
async function saveUrl(url: string) {
  await new Promise<void>((resolve, reject) => {
    const child = spawn('ssh', [...args, 'umask 077; cat > /home/agenttester/e2e-service-url'], { stdio: ['pipe', 'ignore', 'ignore'] })
    child.once('error', reject)
    child.once('exit', code => code === 0 ? resolve() : reject(new Error('Unable to save private URL')))
    child.stdin.end(url + '\n')
  })
}

try {
  await run('systemctl --user show-environment >/dev/null; printf preserved > /home/agenttester/e2e-service-business-file')
  const digest = (await run(`sha256sum ${quote(config.remote_binary)}`)).split(' ')[0]
  const binaryArgs = `--binary ${quote(config.remote_binary)} --sha256 ${quote(digest)}`
  created = await admin.createDevice({ name: `PVE user service E2E ${Date.now()}`, profiles: { user: {
    workdir_roots: ['/home/agenttester'], principals: [config.agent_did], ops: ['open_stream', 'describe', 'exec', 'read', 'write', 'edit', 'query', 'cancel'],
  } } })
  await saveUrl(created.access_url)
  await run(`sh ${quote(script)} install ${binaryArgs} --url-file /home/agenttester/e2e-service-url`)
  await until(async () => !!await online())
  const initial = await online()
  assert.equal(await run('stat -c %a ~/.config/buckyos-agent-daemon/access-url'), '600')
  assert.equal(await run('systemctl --user show buckyos-agent-daemon.service -p Restart --value'), 'no')
  record('explicit user service installation, private credential and no restart policy')
  await run(`sh ${quote(script)} stop`)
  await until(async () => !await online())
  record('service stop removes route')
  await run(`sh ${quote(script)} upgrade ${binaryArgs}`)
  await until(async () => !!await online())
  assert.equal((await online())?.store_id, initial?.store_id)
  record('verified upgrade preserves executor ledger')
  const invalid = '/home/agenttester/e2e-broken-daemon'
  await run(`printf '%s\n' '#!/bin/sh' 'if [ "$1" = --version ]; then echo 0.0.0; exit 0; fi' 'exit 1' > ${quote(invalid)}; chmod 700 ${quote(invalid)}`)
  const invalidHash = (await run(`sha256sum ${quote(invalid)}`)).split(' ')[0]
  await assert.rejects(() => run(`sh ${quote(script)} upgrade --binary ${quote(invalid)} --sha256 ${quote(invalidHash)}`))
  await until(async () => !!await online())
  assert.equal((await run('sha256sum ~/.local/lib/buckyos-agent-daemon/buckyos_agent_daemon')).split(' ')[0], digest)
  record('failed upgrade rolls back to previous binary')
  const access = await admin.issueAccess(created.device.executor_id, Date.now() + 10000)
  await saveUrl(access.access_url)
  await run(`sh ${quote(script)} install ${binaryArgs} --url-file /home/agenttester/e2e-service-url`)
  await until(async () => !!await online())
  await until(async () => !await online(), 15000)
  await until(async () => await run('systemctl --user show buckyos-agent-daemon.service -p ActiveState --value') === 'inactive')
  assert.equal(await run('systemctl --user show buckyos-agent-daemon.service -p NRestarts --value'), '0')
  record('URL expiry stops installed service without a restart loop')
  await run(`sh ${quote(script)} uninstall`)
  assert.equal(await run('test ! -e ~/.config/systemd/user/buckyos-agent-daemon.service; test ! -e ~/.config/buckyos-agent-daemon/access-url; cat /home/agenttester/e2e-service-business-file'), 'preserved')
  record('uninstall removes service and credentials while preserving user files')
} finally {
  await run(`sh ${quote(script)} uninstall`).catch(() => {})
  await run('rm -f /home/agenttester/e2e-service-url /home/agenttester/e2e-broken-daemon').catch(() => {})
  if (created) await admin.deleteDevice(created.device.executor_id).catch(() => {})
  await writeFile(config.report_file.replace(/\.json$/, '-service.json'), JSON.stringify({ vm_ip: config.vm_ip, results }, null, 2) + '\n')
}
