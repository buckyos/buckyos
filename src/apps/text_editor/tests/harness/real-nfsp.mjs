import { mkdir, readFile, writeFile, rm } from 'node:fs/promises'
import { join } from 'node:path'
import { spawn, execFileSync } from 'node:child_process'
const root = process.env.TEXT_EDITOR_NFSP_ROOT
if (!root) throw new Error('Use playwright.nfsp.config.ts')
const home = join(root, 'home')
await mkdir(join(home, 'test-user/notes'), { recursive: true })
await writeFile(join(home, 'test-user/notes/demo.md'), '# Hello\r\nOriginal\r\n')
// The editor's buckets as its package declares them; the standalone server has no app specs of its own.
const app = JSON.parse(await readFile(new URL('../../dapp_meta/app.json', import.meta.url), 'utf8'))
const specs = join(root, 'bucket-specs.json')
await writeFile(specs, JSON.stringify({ 'text-editor.buckyos.bns.did': { buckets: app.service_config_tips.buckets, storage_quota_mib: app.service_config_tips.storage_quota_mib } }))
const target = JSON.parse(execFileSync('cargo', ['metadata', '--format-version', '1', '--no-deps'], { cwd: new URL('../../../../', import.meta.url), encoding: 'utf8' })).target_directory
const child = spawn(process.env.NFSP_SERVER_BIN ?? join(target, 'debug/nfs_server'), ['--listen', '127.0.0.1:3261', '--data-dir', join(root, 'db'), '--export', 'home=' + home, '--named-store', join(root, 'store'), '--bucket-specs', specs, '--scan-interval-secs', '1', '--debug-api', '--log-level', 'warn'], { stdio: 'inherit' })
child.on('error', error => { console.error(error); process.exit(1) })
for (const signal of ['SIGTERM', 'SIGINT']) process.on(signal, () => child.kill(signal))
child.on('exit', async code => { await rm(root, { recursive: true, force: true }); process.exit(code ?? 0) })
