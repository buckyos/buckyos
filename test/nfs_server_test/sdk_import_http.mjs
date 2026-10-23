import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { spawn } from 'node:child_process'
import { mkdtemp, mkdir, rm } from 'node:fs/promises'
import { createServer } from 'node:net'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { setTimeout as delay } from 'node:timers/promises'

// No Zone configuration or installed service is changed. The child server has
// AuthDisabled and a new temporary named store; ordinary-token policy is covered
// separately by nfs_server's Rust auth tests.
const sdk = process.env.BUCKYOS_SDK_MODULE
  ? pathToFileURL(process.env.BUCKYOS_SDK_MODULE).href
  : new URL('../../../buckyos-websdk/dist/node.mjs', import.meta.url).href
const { ndm } = await import(sdk)
const scratch = await mkdtemp(join(tmpdir(), 'ndm-sdk-http-'))
await mkdir(join(scratch, 'home'))
const reservation = createServer()
await new Promise(resolve => reservation.listen(0, '127.0.0.1', resolve))
const port = reservation.address().port
await new Promise(resolve => reservation.close(resolve))
const binary = process.env.NFS_SERVER_BIN || join(process.env.CARGO_TARGET_DIR || fileURLToPath(new URL('../../src/target', import.meta.url)), 'debug', 'nfs_server')
let serverError = ''
const server = spawn(binary, ['--data-dir', join(scratch, 'db'), '--export', `home=${join(scratch, 'home')}`, '--named-store', join(scratch, 'store'), '--listen', `127.0.0.1:${port}`, '--scan-interval-secs', '0', '--log-level', 'warn'], { stdio: ['ignore', 'ignore', 'pipe'] })
server.stderr.on('data', data => { serverError = (serverError + data).slice(-4000) })
server.on('error', error => { serverError = String(error) })


const endpoint = `http://127.0.0.1:${port}`
const requests = []
const fetcher = async (input, init) => { requests.push({ url: String(input), method: init?.method ?? 'GET' }); return fetch(input, init) }
const options = { endpoint, sessionToken: null, autoStartUpload: false, fetcher }
try {
let ready = false
for (let attempt = 0; attempt < 100; attempt++) {
  if (server.exitCode !== null || serverError.includes('ENOENT')) throw Error(`nfs_server failed: ${serverError}`)
  try { await fetch(endpoint); ready = true; break } catch { await delay(100) }
}
assert(ready, `nfs_server did not start: ${serverError}`)
const reports = []
const digest = data => createHash('sha256').update(data).digest('hex')

function measuredFile(bytes, name) {
  let readBytes = 0
  const file = new File([bytes], name, { type: 'application/octet-stream', lastModified: 0 })
  const slice = file.slice.bind(file)
  file.slice = (...args) => {
    const blob = slice(...args)
    const read = blob.arrayBuffer.bind(blob)
    blob.arrayBuffer = async () => { readBytes += blob.size; return read() }
    return blob
  }
  const read = file.arrayBuffer.bind(file)
  file.arrayBuffer = async () => { readBytes += file.size; return read() }
  return { file, readBytes: () => readBytes }
}

async function bytesOf(objId, innerPath) {
  const response = await fetch(`${endpoint}/ndm/proxy/v1/read/object/open`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ obj_id: objId, ...(innerPath ? { inner_path: innerPath } : {}) }) })
  assert.equal(response.status, 200, await (response.ok ? Promise.resolve('') : response.text()))
  return new Uint8Array(await response.arrayBuffer())
}
async function importOne(bytes, name) {
  const measured = measuredFile(bytes, name)
  const snapshot = await ndm.importFiles([measured.file], { ...options, mode: 'single_file' })
  const status = await ndm.ensureUploaded(snapshot.sessionId, options)
  assert.equal(status.objectStatus, 'ready')
  assert.equal((await ndm.isObjectStored({ obj_id: snapshot.items[0].objectId }, options)).stored, true)
  assert.equal(digest(await bytesOf(snapshot.items[0].objectId)), digest(bytes))
  const progress = await ndm.getUploadProgress(snapshot.sessionId)
  const result = { name, size: bytes.length, objId: snapshot.items[0].objectId, readBytes: measured.readBytes(), transmittedBytes: progress.transmittedBytes }
  reports.push(result)
  ndm.releaseImportSession(snapshot.sessionId)
  return result
}

await importOne(new Uint8Array(), 'empty.txt')
const small = Uint8Array.from({ length: 48 * 1024 }, (_, i) => i % 251)
const smallFirst = await importOne(small, 'small.bin')
assert.equal(smallFirst.transmittedBytes, small.length)
const smallAgain = await importOne(small, 'small.bin')
assert.equal(smallFirst.objId, smallAgain.objId)
assert.equal(smallAgain.transmittedBytes, 0)
assert.equal(smallAgain.readBytes, 3 * 4096)
const large = Uint8Array.from({ length: 33 * 1024 * 1024 }, (_, i) => (i * 7 + Math.floor(i / 65536)) % 253)
const largeFirst = await importOne(large, 'large.bin')
assert.equal(largeFirst.transmittedBytes, large.length)
const largeAgain = await importOne(large, 'large-renamed.bin')
assert.equal(largeAgain.transmittedBytes, 0)
assert.equal(largeAgain.readBytes, 3 * 4096, 'QCID hit must read only its three samples')
assert(requests.some(request => new URL(request.url).searchParams.get('quick_hash')?.startsWith('qcid:')), 'No QCID lookup was made')
const tree = { kind: 'dir', name: '资料 folder', children: [
  { kind: 'file', file: new File([small], 'same.bin') },
  { kind: 'dir', name: 'nested', children: [{ kind: 'file', file: new File([small], 'same.bin') }] },
  { kind: 'dir', name: '空目录', children: [] },
  { kind: 'file', file: new File(['new bytes'], 'new.txt') },
] }
const folder = await ndm.importDirectory(tree, options)
assert.equal((await ndm.ensureUploaded(folder.sessionId, options)).objectStatus, 'ready')
const root = folder.items[0].objectId
const dir = JSON.parse((await ndm.getObject({ obj_id: root }, options)).obj_data)
assert.deepEqual(Object.keys(dir.body).sort(), ['nested', 'new.txt', 'same.bin', '空目录'].sort())
assert.equal(digest(await bytesOf(root, 'nested/same.bin')), digest(small))
assert.equal(new TextDecoder().decode(await bytesOf(root, 'new.txt')), 'new bytes')
assert.equal((await ndm.isObjectStored({ obj_id: root }, options)).stored, true)
reports.push({ name: tree.name, objId: root, transmittedBytes: (await ndm.getUploadProgress(folder.sessionId)).transmittedBytes })
ndm.releaseImportSession(folder.sessionId)
console.log(JSON.stringify({ endpoint, mode: 'isolated AuthDisabled HTTP', results: reports, qcidLookups: requests.filter(request => new URL(request.url).searchParams.get('quick_hash')?.startsWith('qcid:')).length }, null, 2))
} finally {
  server.kill('SIGTERM')
  if (server.exitCode === null && !serverError.includes('ENOENT')) {
    const timeout = setTimeout(() => server.kill('SIGKILL'), 5000)
    await new Promise(resolve => server.once('exit', resolve))
    clearTimeout(timeout)
  }
  await rm(scratch, { recursive: true, force: true })
}
