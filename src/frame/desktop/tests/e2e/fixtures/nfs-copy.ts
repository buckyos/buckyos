import { test as base, expect } from '@playwright/test'
import { spawn, type ChildProcess } from 'node:child_process'
import { copyFile, mkdtemp, mkdir, chmod, chown, writeFile, readFile, rm, open } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'

const target = resolve('../..', process.env.CARGO_TARGET_DIR ?? 'target')
const endpoint = 'http://127.0.0.1:3262'
const uid = process.getuid?.() === 0 ? 65534 : undefined
export class CopyServices {
  root = ''
  token = ''
  otherToken = ''
  session = ''
  nfs?: ChildProcess
  task?: ChildProcess
  logs: string[] = []
  async start() {
    this.root = await mkdtemp(join(uid === undefined ? tmpdir() : '/tmp', 'buckyos-copy-e2e-'))
    await chmod(this.root, 0o755)
    for (const directory of ['task', 'nfs', 'home', 'public', 'home/Documents', 'home/Pictures', 'home/Dest/a/b']) {
      const path = join(this.root, directory)
      await mkdir(path, { recursive: true }); await chmod(path, 0o777)
      if (uid !== undefined) await chown(path, uid, uid)
    }
    await this.directory('Dest/a/b')
    await this.write('Documents/notes.txt', Buffer.from('Real NFSP copy — 文件内容\n'))
    for (const binary of ['nfs_server', 'examples/nfs_copy_fixture']) {
      const destination = join(this.root, binary.replace('examples/', ''))
      await copyFile(join(target, 'debug', binary), destination)
      await chmod(destination, 0o755)
    }
    this.startTask()
    await expect.poll(async () => readFile(join(this.root, 'task/user-token'), 'utf8').catch(() => ''), { timeout: 30000 }).not.toBe('')
    this.token = await readFile(join(this.root, 'task/user-token'), 'utf8')
    this.otherToken = await readFile(join(this.root, 'task/other-token'), 'utf8')
    await this.restartNfs()
  }
  startProcess(binary: string, args: string[], env: Record<string, string> = {}) {
    const child = spawn(join(this.root, binary.replace('examples/', '')), args, { uid, gid: uid, env: { ...process.env, ...env }, stdio: ['ignore', 'pipe', 'pipe'] })
    child.stdout?.on('data', (chunk) => this.logs.push(String(chunk)))
    child.stderr?.on('data', (chunk) => this.logs.push(String(chunk)))
    return child
  }
  startTask() { this.task = this.startProcess('examples/nfs_copy_fixture', [join(this.root, 'task'), '3382']) }
  async stopProcess(child?: ChildProcess) {
    if (!child || child.exitCode !== null || child.signalCode !== null) return
    const exited = new Promise<void>((resolve) => child.once('exit', () => resolve()))
    child.kill('SIGKILL'); await exited
  }
  async restartNfs(env: Record<string, string> = {}) {
    await this.stopProcess(this.nfs)
    this.nfs = this.startProcess('nfs_server', ['--listen', '127.0.0.1:3262', '--data-dir', join(this.root, 'nfs'), '--export', `home=${join(this.root, 'home')}`, '--export', `public=${join(this.root, 'public')}`, '--debug-api', '--copy-test-config', join(this.root, 'task/nfs-copy-test.json')], env)
    await expect.poll(async () => {
      try { this.session = (await this.call('hello', {})).session; return !!this.session } catch { return false }
    }, { timeout: 30000 }).toBe(true)
  }
  async restartTasks() {
    await this.stopProcess(this.task); this.startTask()
    await expect.poll(async () => { try { await this.call('copy_list', {}); return true } catch { return false } }, { timeout: 30000 }).toBe(true)
  }
  async call(method: string, args: Record<string, unknown> = {}, at?: string, token = this.token): Promise<any> {
    const response = await fetch(`${endpoint}/nfs/v1/${method}`, { method: 'POST', headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${token}` }, body: JSON.stringify({ session: this.session || undefined, args, at: at ? { path: at } : undefined }) })
    const result = await response.json() as any
    if (!result.ok) throw new Error(JSON.stringify(result.error))
    return result.result
  }
  path(relative: string) { return join(this.root, 'home', relative) }
  async directory(relative: string) {
    const parts = relative.split('/')
    for (let i = 1; i <= parts.length; i++) { const path = this.path(parts.slice(0, i).join('/')); await mkdir(path, { recursive: true }); await chmod(path, 0o777) }
  }
  async write(relative: string, data: Buffer | string) {
    await this.directory(relative.split('/').slice(0, -1).join('/'))
    await writeFile(this.path(relative), data); await chmod(this.path(relative), 0o666)
    if (uid !== undefined) await chown(this.path(relative), uid, uid)
  }
  async large(relative: string, megabytes = 128) {
    await this.write(relative, '')
    const file = await open(this.path(relative), 'w')
    const block = Buffer.alloc(1024 * 1024)
    for (let i = 0; i < block.length; i++) block[i] = (i * 31) % 251
    for (let i = 0; i < megabytes; i++) await file.write(block)
    await file.close()
  }
  async input(sources: string[], destination: string) {
    return { sources: await Promise.all(sources.map(async (path) => ({ source_ref: (await this.call('stat', {}, `/home/${path}`)).copy_ref, source_path: `/home/${path}`, name: path.split('/').pop()! }))), destination_ref: (await this.call('stat', {}, `/home/${destination}`)).copy_ref, conflict: 'ask', retry_of: null }
  }
  async submit(input: object, key = crypto.randomUUID()) { return (await this.call('copy_submit', { input, idempotency_key: key })).task_id as string }
  async terminal(taskId: string) {
    await expect.poll(async () => (await this.call('copy_get', { task_id: taskId })).task.phase, { timeout: 120000 }).toBe('Terminal')
    return this.call('copy_get', { task_id: taskId })
  }
  async items(taskId: string) {
    const items = []
    let after = 0
    do { const view = await this.call('copy_get', { task_id: taskId, after }); items.push(...view.items); after = view.next } while (after)
    return items
  }
  async close() {
    await this.stopProcess(this.nfs); await this.stopProcess(this.task)
    await writeFile(resolve('test-results/copy-service.log'), this.logs.join('')).catch(() => {})
    await rm(this.root, { recursive: true, force: true })
  }
}
export const test = base.extend<{}, { services: CopyServices | null }>({
  services: [async ({}, use) => {
    if (!process.env.FB_COPY_AUTO) { await use(null); return }
    const services = new CopyServices()
    try { await services.start(); await use(services) } finally { await services.close() }
  }, { scope: 'worker', auto: true }],
  context: async ({ context, services }, use) => {
    if (services) await context.setExtraHTTPHeaders({ Authorization: `Bearer ${services.token}` })
    await use(context)
  },
})
export { expect }
