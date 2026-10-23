import { test } from 'node:test'
import assert from 'node:assert/strict'
import { NfspError } from 'buckyos/nfsp'
import { NfspDocumentStore, SaveError, SaveConflict, errorCategory } from '../../src/fs/documentStore.ts'
import { saveDocument } from '../../src/fs/saveFlow.ts'
const bytes = new TextEncoder().encode('new text')
const nfsError = (code: string, status = 409) => new NfspError({ code, message: code }, status)
/** A fake NFSP client over an in-memory tree; `hooks` inject failures per method. */
function client(paths: Record<string, 'dir' | string>, hooks: Partial<Record<string, (args: any) => unknown>> = {}) {
  const tree = new Map(Object.entries(paths)); const calls: string[] = []
  const node = (path: string) => { const v = tree.get(path); if (v === undefined) throw nfsError('NOT_FOUND', 404); return v }
  const info = (path: string) => { const v = node(path); return { kind: v === 'dir' ? 'dir' : 'file', ref: { type: 'live', node_id: path }, node_id: path, name: path.split('/').pop(), size: v === 'dir' ? 0 : new TextEncoder().encode(v).length, etag: `e-${v === 'dir' ? 0 : v.length}`, capabilities: {} } }
  const run = async (name: string, args: any, real: () => unknown) => { calls.push(name); return hooks[name] ? hooks[name]!(args) ?? real() : real() }
  let staged: Uint8Array | undefined
  const fake: any = {
    sessionId: 's', hello: async () => ({}),
    resolve: (path: string) => run('resolve', path, () => info(path)),
    mkdir: (path: string) => run('mkdir', path, () => {
      let at = ''
      for (const part of path.slice('cyfs://'.length).split('/').filter(Boolean)) { at += '/' + part; const v = tree.get('cyfs://' + at); if (v !== undefined && v !== 'dir') throw nfsError('NAMESPACE_CONFLICT'); tree.set('cyfs://' + at, 'dir') }
      return { ref: { type: 'live', node_id: path }, existed: false }
    }),
    openWrite: (target: any) => run('openWrite', target, () => ({ fb_handle: 'fb', lease: { lease_id: 'l' }, target: { exists: 'ref' in target } })),
    uploadContent: (_fb: string, content: Uint8Array) => run('uploadContent', content, () => { staged = content }),
    commitFile: (parent: string, name: string) => run('commitFile', { parent, name }, () => { tree.set(`${parent}/${name}`, new TextDecoder().decode(staged)); return { ref: { type: 'live', node_id: `${parent}/${name}` } } }),
    abortWrite: (id: string) => run('abortWrite', id, () => ({})),
    readFile: (path: string) => run('readFile', path, () => new Response(node(path) as string, { headers: { etag: `W/"e-${(node(path) as string).length}"` } })),
  }
  return { fake, tree, calls }
}
const target = 'cyfs:///home/alice/new/deeper/file.md'
test('a formal save creates the missing folders, then writes', async () => {
  const c = client({ 'cyfs:///home/alice': 'dir' })
  const saved = await new NfspDocumentStore(c.fake).write(target, bytes, null, { createParents: true })
  assert.equal(c.tree.get(target), 'new text'); assert.equal(c.tree.get('cyfs:///home/alice/new/deeper'), 'dir')
  assert.equal(saved.etag, 'e-8')
  assert.ok(!c.calls.includes('abortWrite'), 'a committed lease is released by the server, not aborted')
})
test('without createParents a missing folder fails before anything is written', async () => {
  const c = client({ 'cyfs:///home/alice': 'dir' })
  await assert.rejects(new NfspDocumentStore(c.fake).write(target, bytes, null), (e: unknown) => e instanceof SaveError && e.stage === 'prepare' && e.category === 'not-found')
  assert.ok(!c.calls.includes('mkdir') && !c.calls.includes('openWrite'))
})
test('a file in the way is named as such, and nothing already created is removed', async () => {
  const c = client({ 'cyfs:///home/alice': 'dir', 'cyfs:///home/alice/new': 'plain file' })
  await assert.rejects(new NfspDocumentStore(c.fake).write(target, bytes, null, { createParents: true }), (e: unknown) => e instanceof SaveError && e.stage === 'prepare' && e.category === 'not-dir' && e.path === 'cyfs:///home/alice/new/deeper')
  assert.equal(c.tree.get('cyfs:///home/alice/new'), 'plain file')
})
test('a folder created concurrently by someone else is fine', async () => {
  const c = client({ 'cyfs:///home/alice': 'dir' }, { mkdir: () => { c.tree.set('cyfs:///home/alice/new', 'dir'); c.tree.set('cyfs:///home/alice/new/deeper', 'dir'); throw nfsError('INTERNAL', 500) } })
  await new NfspDocumentStore(c.fake).write(target, bytes, null, { createParents: true })
  assert.equal(c.tree.get(target), 'new text')
})
test('after a commit, failing to read the new etag back does not fail the save', async () => {
  let resolves = 0
  const c = client({ 'cyfs:///home/alice/new/deeper': 'dir' }, { resolve: (path: string) => { if (path === target && c.tree.has(target) && resolves++ < 5) throw new TypeError('Failed to fetch') } })
  const saved = await new NfspDocumentStore(c.fake).write(target, bytes, null)
  assert.deepEqual(saved, { source: { kind: 'cyfs-path', path: target }, etag: '', size: bytes.length })
})
test('a lost commit answer is checked against the disk instead of being redone', async () => {
  const applied = client({ 'cyfs:///home/alice/new/deeper': 'dir' })
  applied.fake.commitFile = async (parent: string, name: string) => { applied.tree.set(`${parent}/${name}`, 'new text'); throw new TypeError('Failed to fetch') }
  assert.equal((await new NfspDocumentStore(applied.fake).write(target, bytes, null)).etag, 'e-8')
  const lost = client({ 'cyfs:///home/alice/new/deeper': 'dir' }, { commitFile: () => { throw nfsError('INTERNAL', 504) } })
  await assert.rejects(new NfspDocumentStore(lost.fake).write(target, bytes, null), (e: unknown) => e instanceof SaveError && e.stage === 'commit' && e.unknown && e.category === 'network')
  assert.ok(lost.calls.includes('abortWrite'))
})
test('a refused commit is a known failure, and a failing abort does not hide it', async () => {
  const c = client({ 'cyfs:///home/alice/new/deeper': 'dir' }, { commitFile: () => { throw nfsError('PERMISSION_DENIED', 403) }, abortWrite: () => { throw new TypeError('Failed to fetch') } })
  await assert.rejects(new NfspDocumentStore(c.fake).write(target, bytes, null), (e: unknown) => e instanceof SaveError && e.stage === 'commit' && !e.unknown && e.category === 'permission')
  const busy = client({ 'cyfs:///home/alice/new/deeper': 'dir' }, { openWrite: () => { throw nfsError('LEASE_CONFLICT', 423) } })
  await assert.rejects(new NfspDocumentStore(busy.fake).write(target, bytes, null), (e: unknown) => e instanceof SaveConflict && e.kind === 'lease-busy')
  assert.equal(errorCategory(new TypeError('Failed to fetch')), 'network')
})
test('a retried save whose earlier answer was lost adopts the disk instead of reporting a conflict', async () => {
  const source = { kind: 'cyfs-path' as const, path: target }
  const doc = { source, name: 'file.md', text: 'new text', base: { etag: 'before', size: 3 }, readOnly: false, encoding: 'utf-8', bom: false, eol: '\n' as const, mixedEol: false }
  const disk = { source, etag: 'after', size: 8, bytes }
  let writes = 0
  const services: any = { origin: { device: 't' }, flush: async () => {}, archive: async () => ({}), annotate: async () => {}, store: { stat: async () => disk, read: async () => disk, write: async () => { writes++; return disk } } }
  assert.deepEqual(await saveDocument(doc, services), { source, etag: 'after', size: 8 })
  assert.equal(writes, 0)
})
