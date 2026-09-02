/**
 * End-to-end verification of nfs_server's main logic, driven exclusively
 * through the TS client (src/api/nfsp_client.ts) — mirrors the server's
 * tests/integration.rs, exercised over real HTTP.
 *
 * Run (Node ≥ 23, type stripping; the script must be on the same machine as
 * the server because bypass-write tests touch the export root directly):
 *
 *   cargo build -p nfs_server
 *   mkdir -p /tmp/nfsp-test/home /tmp/nfsp-test/data
 *   nfs_server --listen 127.0.0.1:3261 --data-dir /tmp/nfsp-test/data \
 *              --export home=/tmp/nfsp-test/home --scan-interval-secs 0 --debug-api &
 *   NFSP_BASE=http://127.0.0.1:3261 NFSP_HOME=/tmp/nfsp-test/home \
 *              node test/test_nfs_server/verify_nfsp.ts
 *
 * Expects a FRESH server (empty export root + data dir) per run.
 */
import * as fs from 'node:fs'
import * as path from 'node:path'
import { createHash } from 'node:crypto'
import {
  NfspClient,
  NfspError,
  liveRef,
  type WireRef,
  type WatchEvent,
} from '../../src/frame/desktop/src/api/nfsp_client.ts'

const BASE = process.env.NFSP_BASE ?? 'http://127.0.0.1:3261'
const HOME = process.env.NFSP_HOME
if (!HOME) throw new Error('NFSP_HOME not set')

let passed = 0
const failures: string[] = []

function assert(cond: unknown, msg: string): asserts cond {
  if (!cond) throw new Error(`assert failed: ${msg}`)
}
function assertEq(actual: unknown, expected: unknown, msg: string) {
  const a = JSON.stringify(actual)
  const b = JSON.stringify(expected)
  if (a !== b) throw new Error(`${msg}: expected ${b}, got ${a}`)
}
async function expectErr(code: string, fn: () => Promise<unknown>, httpStatus?: number): Promise<NfspError> {
  try {
    await fn()
  } catch (e) {
    if (e instanceof NfspError) {
      if (e.code !== code) throw new Error(`expected error ${code}, got ${e.code}: ${e.message}`)
      if (httpStatus !== undefined && e.httpStatus !== httpStatus)
        throw new Error(`expected HTTP ${httpStatus} for ${code}, got ${e.httpStatus}`)
      return e
    }
    throw e
  }
  throw new Error(`expected error ${code}, but the call succeeded`)
}
async function test(name: string, fn: () => Promise<void>) {
  try {
    await fn()
    passed++
    console.log(`PASS ${name}`)
  } catch (e) {
    failures.push(`${name}: ${e instanceof Error ? e.message : e}`)
    console.log(`FAIL ${name}: ${e instanceof Error ? e.stack : e}`)
  }
}
const sha256hex = (b: Uint8Array) => createHash('sha256').update(b).digest('hex')
const hp = (...seg: string[]) => path.join(HOME!, ...seg)

const c = new NfspClient({ baseUrl: BASE })

// ---------------------------------------------------------------------------

await test('hello: version negotiation, features, limits, realms', async () => {
  const r = await c.hello()
  assertEq(r.version, 'nfsp/0', 'protocol version')
  assert(r.session.length > 0, 'session issued')
  for (const f of ['view', 'collection', 'reference-binding', 'watch.sse', 'search.name'])
    assert(r.features.includes(f), `feature ${f} advertised`)
  assert(!r.features.includes('repr'), 'unimplemented repr not advertised')
  assert(r.limits.max_batch > 0 && r.limits.max_list > 0, 'limits present')
  assertEq(r.realms[0], { id: 'dfs', writable: true }, 'dfs realm')
})

await test('browse: resolve/list/stat, want groups, error paths', async () => {
  fs.mkdirSync(hp('docs'))
  fs.writeFileSync(hp('a.txt'), 'hello')
  fs.writeFileSync(hp('docs/b.md'), '# b')

  const root = await c.resolve('/')
  assertEq(root.kind, 'dir', 'root kind')
  assert(root.capabilities.list, 'root listable')
  assert(root.flags!.includes('read_only'), 'root read-only')

  const rootList = await c.list('/', undefined, ['base', 'ident'])
  assertEq(rootList.entries.length, 1, 'one export root')
  assertEq(rootList.entries[0].name, 'home', 'export root name')
  assertEq(rootList.entries[0].binding, 'native', 'binding native')

  const home = await c.list('/home', undefined, ['base', 'ident'])
  assertEq(home.entries.map((e) => e.name), ['a.txt', 'docs'], 'byte-ordered names')
  assertEq(home.entries[0].target.kind, 'file', 'a.txt is file')
  assertEq(home.entries[0].target.attrs!.size, 5, 'a.txt size')
  assert(home.watch_token!.startsWith('w_'), 'watch token issued')

  // stat by ref round-trips; access urls present.
  const info = await c.stat(home.entries[0].target.ref, { want: ['base', 'ident', 'access'] })
  assertEq(info.kind, 'file', 'stat by ref')
  assert(info.etag!.includes('-'), 'etag shape')
  assert(info.access_urls!.some((u) => u.kind === 'fs' && u.url === 'cyfs:///home/a.txt'), 'fs url')
  assert(info.access_urls!.some((u) => u.kind === 'read'), 'read url')

  await expectErr('NOT_FOUND', () => c.resolve('/nope'), 404)
  await expectErr('NOT_A_CONTAINER', () => c.list('/home/a.txt'), 400)
  await expectErr('INVALID_ARGUMENT', () => c.resolve('/home/../etc'))
  await expectErr('PERMISSION_DENIED', () =>
    new NfspClient({ baseUrl: BASE }).list('/'))  // no hello → no session
})

await test('list: pagination, D10 cursor stability, filters', async () => {
  fs.mkdirSync(hp('pg'))
  for (let i = 0; i < 25; i++) fs.writeFileSync(hp('pg', `f${String(i).padStart(2, '0')}.txt`), 'x')
  const p1 = await c.list('/home/pg', { limit: 10 })
  assertEq(p1.entries.length, 10, 'page 1 size')
  assertEq(p1.truncated, true, 'truncated')

  // Concurrent change between pages: cursor never resets.
  fs.writeFileSync(hp('pg', 'f00-inserted.txt'), 'x')
  await c.mkdir((await c.resolve('/home/pg')).ref as WireRef, 'zz-dir')
  const p2 = await c.list('/home/pg', { limit: 10, cursor: p1.next_cursor })
  assertEq(p2.revision_changed, true, 'revision_changed flagged')
  assertEq(p2.entries[0].name, 'f10.txt', 'cursor continues, no re-emit')

  const filtered = await c.list('/home/pg', { filter: { name_glob: 'f1?.txt' }, limit: 100 })
  assertEq(filtered.entries.length, 10, 'glob filter')
  const dirsOnly = await c.list('/home/pg', { filter: { kind: ['dir'] }, limit: 100 })
  assertEq(dirsOnly.entries.map((e) => e.name), ['zz-dir'], 'kind filter')
})

await test('batch: shared cursor walk/stat/list, abort & continue', async () => {
  fs.mkdirSync(hp('photos/2026'), { recursive: true })
  fs.writeFileSync(hp('photos/2026/cover.jpg'), 'jpg')
  const r = await c.batch('/home', [
    { m: 'walk', args: { names: ['photos', '2026'] } },
    { m: 'list', args: { limit: 200 }, want: ['base', 'ident'] },
    { m: 'stat', args: { name: 'cover.jpg' }, want: ['base', 'access'] },
  ])
  assertEq(r.completed, 3, 'all ops completed')
  const [w, l, s] = r.results as { ok: true; result: any }[]
  assertEq(w.result.kind, 'dir', 'walk lands on dir')
  assertEq(l.result.entries[0].name, 'cover.jpg', 'list at cursor')
  assertEq(s.result.name, 'cover.jpg', 'stat child')

  const abort = await c.batch('/home', [{ m: 'walk', args: { name: 'missing' } }, { m: 'list' }])
  assertEq(abort.completed, 0, 'abort default')
  assertEq(abort.results.length, 1, 'stopped at first error')
  assertEq((abort.results[0] as any).error.code, 'NOT_FOUND', 'walk error surfaced')

  const cont = await c.batch('/home', [{ m: 'stat', args: { name: 'missing' } }, { m: 'list' }], 'continue')
  assertEq(cont.completed, 1, 'continue mode')
  assertEq(cont.results.length, 2, 'both results present')
})

await test('upload → commit → read roundtrip, probe, 秒传, NEED_PULL', async () => {
  const content = new TextEncoder().encode('The quick brown fox jumps over the lazy dog')
  const parentRef = (await c.resolve('/home')).ref as WireRef
  const committed = await c.uploadFile(parentRef, 'fox.txt', content)
  assertEq(committed.obj.sha256, sha256hex(content), 'server hash matches')
  const nodeId = (committed.ref as { node_id: string }).node_id

  // Full read.
  const full = await c.readFile(nodeId)
  assertEq(full.status, 200, 'read 200')
  assertEq(Array.from(new Uint8Array(await full.arrayBuffer())), Array.from(content), 'content roundtrip')
  const etag = full.headers.get('etag')!

  // Range read.
  const part = await c.readFile(nodeId, { range: { start: 4, end: 8 } })
  assertEq(part.status, 206, 'partial content')
  assertEq(part.headers.get('content-range'), `bytes 4-8/${content.length}`, 'content-range')
  assertEq(await part.text(), 'quick', 'range bytes')

  // Conditional read.
  const cond = await c.readFile(nodeId, { ifNoneMatch: etag })
  assertEq(cond.status, 304, 'etag 304')

  // Download disposition url.
  const dl = await fetch(c.readUrl(nodeId, { download: true, name: '狐.txt' }))
  assert(dl.headers.get('content-disposition')!.startsWith("attachment; filename*=UTF-8''"), 'disposition')

  // probe: known content present, unknown missing.
  const probe = await c.probe([
    { hash: committed.obj.sha256, size: content.length },
    { hash: '0'.repeat(64), size: 1 },
  ])
  assertEq(probe.missing.length, 1, 'one missing')
  assertEq(probe.missing[0].hash, '0'.repeat(64), 'unknown hash missing')

  // 秒传 (dedup by hash only).
  const dedup = await c.commitFile('/home', 'fox-copy.txt', { hash: committed.obj.sha256 })
  assertEq(dedup.obj.sha256, committed.obj.sha256, 'dedup hash')
  assertEq(fs.readFileSync(hp('fox-copy.txt')).toString(), new TextDecoder().decode(content), 'dedup content on disk')

  // Unknown hash → NEED_PULL with obj_id.
  const err = await expectErr('NEED_PULL', () =>
    c.commitFile('/home', 'ghost.txt', { hash: '1'.repeat(64) }), 409)
  assert(String(err.details.obj_id).startsWith('sha256:'), 'NEED_PULL carries obj_id')
})

await test('revision CAS, seq exactly-once replay, mkdir -p', async () => {
  const listing = await c.list('/home')
  const rev = listing.container.revision!
  const homeRef = listing.container.ref as WireRef

  const mk = await c.mkdir(homeRef, 'd1', { expectedRevision: rev })
  assertEq(mk.existed, false, 'CAS mkdir')
  await expectErr('REV_MISMATCH', () => c.mkdir(homeRef, 'd2', { expectedRevision: rev }), 409)

  // Exactly-once: same seq replayed returns the cached result verbatim.
  // Runs on a throwaway session so the raw seq doesn't disturb the main
  // client's replay window.
  const cr = new NfspClient({ baseUrl: BASE })
  await cr.hello()
  const body = { session: cr.sessionId, seq: 1, at: { realm: 'dfs', path: '/home' }, args: { name: 'd3' } }
  const send = async () => (await (await fetch(`${BASE}/nfs/v1/mkdir`, {
    method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body),
  })).json()) as any
  const first = await send()
  assertEq(first.result.existed, false, 'first mkdir executes')
  const replay = await send()
  assertEq(replay.result.existed, false, 'replay returns cached result, not re-execution')

  // A seq far ahead advances the window; an old seq then falls out of it.
  await (await fetch(`${BASE}/nfs/v1/mkdir`, {
    method: 'POST', headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ ...body, seq: 500, args: { name: 'd3b' } }),
  })).json()
  const stale = await (await fetch(`${BASE}/nfs/v1/mkdir`, {
    method: 'POST', headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ ...body, seq: 2, args: { name: 'd3c' } }),
  })).json() as any
  assertEq(stale.error.code, 'SEQ_OUT_OF_WINDOW', 'old seq rejected once window moved')

  // Write without seq rejected (raw call path).
  const noSeq = await (await fetch(`${BASE}/nfs/v1/mkdir`, {
    method: 'POST', headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ session: cr.sessionId, at: { realm: 'dfs', path: '/home' }, args: { name: 'd4' } }),
  })).json() as any
  assertEq(noSeq.error.code, 'INVALID_ARGUMENT', 'seq required for writes')
  await cr.bye()

  // Path form = mkdir -p, idempotent.
  const deep = await c.mkdir('/home/x/y/z')
  assertEq(deep.existed, false, 'mkdir -p creates')
  assert(fs.statSync(hp('x/y/z')).isDirectory(), 'on disk')
  const again = await c.mkdir('/home/x/y/z')
  assertEq(again.existed, true, 'idempotent')
})

await test('lease conflict + bypass-write detection at commit', async () => {
  const parentRef = (await c.resolve('/home')).ref as WireRef
  await c.uploadFile(parentRef, 'doc.txt', new TextEncoder().encode('v1 content'))

  const ow = await c.openWrite({ parentRef, name: 'doc.txt' })
  assert(ow.lease.ttl_ms > 0, 'lease ttl')
  assertEq(ow.target.exists, true, 'target exists')

  // A second session conflicts on the same target.
  const c2 = new NfspClient({ baseUrl: BASE })
  await c2.hello()
  const err = await expectErr('LEASE_CONFLICT', () => c2.openWrite({ parentRef, name: 'doc.txt' }), 423)
  assert(typeof err.details.holder_session === 'string', 'holder_session reported')
  await c2.bye()

  // Bypass edit between open_write and commit → explicit TARGET_MISMATCH.
  fs.writeFileSync(hp('doc.txt'), 'bypass writer changed me!')
  await c.uploadContent(ow.fb_handle, new TextEncoder().encode('v2 content'))
  const mism = await expectErr('TARGET_MISMATCH', () =>
    c.commitFile('/home', 'doc.txt', { fbHandle: ow.fb_handle, leaseId: ow.lease.lease_id }), 409)
  assertEq(mism.details.reason, 'bypass_modified', 'reason surfaced')
  assertEq(fs.readFileSync(hp('doc.txt')).toString(), 'bypass writer changed me!', 'bypass content not clobbered')
})

await test('tus resume: HEAD offset + chunked continuation', async () => {
  const parentRef = (await c.resolve('/home')).ref as WireRef
  const content = new TextEncoder().encode('0123456789abcdef')
  const ow = await c.openWrite({ parentRef, name: 'resume.bin', size: content.length })
  await c.uploadChunk(ow.fb_handle, 0, content.subarray(0, 7))
  assertEq(await c.uploadOffset(ow.fb_handle), 7, 'offset after first chunk')
  // uploadContent resumes from the server-side offset.
  await c.uploadContent(ow.fb_handle, content)
  const done = await c.commitFile('/home', 'resume.bin', { fbHandle: ow.fb_handle, leaseId: ow.lease.lease_id })
  assertEq(done.obj.size, content.length, 'full size committed')
  assertEq(fs.readFileSync(hp('resume.bin')).toString(), '0123456789abcdef', 'content intact')
})

await test('move/delete/bind_ref/unlink, anchors survive, stale honesty', async () => {
  fs.mkdirSync(hp('src'))
  fs.mkdirSync(hp('dst'))
  const srcRef = (await c.resolve('/home/src')).ref as WireRef
  const dstRef = (await c.resolve('/home/dst')).ref as WireRef
  await c.uploadFile(srcRef, 'file.txt', new TextEncoder().encode('content!'))

  // Anchor via set_meta, then watch identity survive a protocol move.
  await c.setMeta((await c.resolve('/home/src/file.txt')).ref, [{ ns: 'user', key: 'rating', value: 5 }])
  const before = await c.resolve('/home/src/file.txt', ['ident'])
  const stableId = before.node_id!
  assert(stableId.startsWith('n_'), 'anchored node id')

  await c.move({ parentRef: srcRef, name: 'file.txt' }, { parentRef: dstRef, name: 'renamed.txt' })
  assert(fs.existsSync(hp('dst/renamed.txt')), 'moved on disk')
  const after = await c.stat(liveRef(stableId, 1), { want: ['base'] })
  assertEq(after.name, 'renamed.txt', 'stable ref follows move')
  const meta = await c.getMeta(liveRef(stableId, 1))
  assertEq(meta.records[0].value, 5, 'meta survives move')

  // bind_ref + list surfaces reference binding with canonical_path.
  const bind = await c.bindRef(srcRef, 'link-to-file', liveRef(stableId, 1))
  assert(bind.entry_ref.startsWith('be_'), 'binding entry_ref')
  const listing = await c.list('/home/src')
  assertEq(listing.entries.length, 1, 'one entry')
  assertEq(listing.entries[0].binding, 'reference', 'reference binding')
  assertEq(listing.entries[0].canonical_path, 'cyfs:///home/dst/renamed.txt', 'canonical path')

  // delete refuses reference entries; unlink removes only the entry.
  const delErr = await expectErr('INVALID_ARGUMENT', () => c.delete('/home/src', 'link-to-file'))
  assert(delErr.message.includes('unlink'), 'points at unlink')
  await c.unlink(bind.entry_ref)
  assert(fs.existsSync(hp('dst/renamed.txt')), 'target untouched by unlink')
  assertEq((await c.list('/home/src')).entries.length, 0, 'entry gone')

  // Destroying the target turns the stable ref STALE (410).
  await c.delete('/home/dst', 'renamed.txt')
  await expectErr('STALE', () => c.stat(liveRef(stableId, 1)), 410)

  // Non-empty delete guard + recursive.
  fs.mkdirSync(hp('full'))
  fs.writeFileSync(hp('full/x'), 'x')
  await expectErr('NOT_EMPTY', () => c.delete('/home', 'full'), 409)
  await c.delete('/home', 'full', { recursive: true })
  assert(!fs.existsSync(hp('full')), 'recursively deleted')

  // Moving a dir into its own subtree is rejected.
  fs.mkdirSync(hp('cyc/inner'), { recursive: true })
  const homeRef = (await c.resolve('/home')).ref as WireRef
  const innerRef = (await c.resolve('/home/cyc/inner')).ref as WireRef
  const cycErr = await expectErr('INVALID_ARGUMENT', () =>
    c.move({ parentRef: homeRef, name: 'cyc' }, { parentRef: innerRef, name: 'cyc2' }))
  assert(cycErr.message.includes('itself'), 'cycle message')
})

await test('binding vs native shadow conflict', async () => {
  fs.mkdirSync(hp('shadow'))
  fs.writeFileSync(hp('target.txt'), 't')
  const dRef = (await c.resolve('/home/shadow')).ref as WireRef
  const tRef = (await c.resolve('/home/target.txt')).ref as WireRef
  await c.bindRef(dRef, 'item', tRef)
  // Bypass writer creates a same-name native file → conflict surfaced.
  fs.writeFileSync(hp('shadow/item'), 'native')
  const listing = await c.list('/home/shadow')
  assertEq(listing.entries.length, 1, 'native wins listing')
  assertEq(listing.entries[0].binding, 'native', 'native binding shown')
  assertEq(listing.conflicts![0].name, 'item', 'conflict name')
  assertEq(listing.conflicts![0].reason, 'native_shadow', 'conflict reason')
  assert(listing.conflicts![0].entry_ref!.startsWith('be_'), 'conflict keeps entry_ref for unlink')
  // Binding over an existing native name refused up front.
  await expectErr('NAMESPACE_CONFLICT', () => c.bindRef(dRef, 'item', tRef), 409)
})

await test('collection lifecycle: patch ops, groups, stale members', async () => {
  fs.writeFileSync(hp('one.pdf'), '1')
  fs.writeFileSync(hp('two.pdf'), '2')
  const one = (await c.resolve('/home/one.pdf')).ref as WireRef
  const two = (await c.resolve('/home/two.pdf')).ref as WireRef

  const col = await c.createCollection('Reading List', 'reading')
  assertEq(col.kind, 'collection', 'kind')
  assertEq(col.capabilities.remove_semantics, 'unlink', 'unlink semantics')
  const cref = col.ref as WireRef
  const rev = col.revision!

  const patched = await c.collectionPatch(cref, [
    { add_ref: { target_ref: one } },
    { add_ref: { target_ref: two, position: 0, name: 'second-first' } },
    { create_group: { name: 'papers' } },
  ], { expectedRevision: rev })
  assert(patched.revision !== rev, 'revision advanced')
  await expectErr('REV_MISMATCH', () =>
    c.collectionPatch(cref, [{ create_group: { name: 'x' } }], { expectedRevision: rev }))

  const listing = await c.list({ uri: 'collection://reading' })
  assertEq(listing.container.kind, 'collection', 'listed as collection')
  assertEq(listing.entries[0].name, 'second-first', 'manual order, position 0')
  assertEq(listing.entries[0].canonical_path, 'cyfs:///home/two.pdf', 'member canonical path')
  const group = listing.entries.find((e) => e.target.kind === 'group')!
  assertEq(group.binding, 'member', 'group binding')

  // Add into the group; the group lists as its own container.
  await c.collectionPatch(cref, [
    { add_ref: { target_ref: two, parent_entry_ref: group.entry_ref! } },
  ])
  const groupListing = await c.list(group.target.ref as WireRef)
  assertEq(groupListing.container.kind, 'group', 'group container')
  assertEq(groupListing.entries.length, 1, 'group member added')

  // open_collection resolves the same node.
  const opened = await c.openCollection('reading')
  assertEq(opened.ref, cref, 'open_collection ref')

  // Deleting the native target leaves stale members (never silently dropped).
  await c.delete('/home', 'two.pdf')
  const staleList = await c.list({ uri: 'collection://reading' })
  assert(staleList.entries.some((e) => (e.target as any).target_state === 'stale'), 'stale surfaced')

  // rename_group then remove_entry.
  await c.collectionPatch(cref, [
    { rename_group: { entry_ref: group.entry_ref!, name: 'archive' } },
    { remove_entry: { entry_ref: group.entry_ref! } },
  ])
  const finalList = await c.list({ uri: 'collection://reading' })
  assert(finalList.entries.every((e) => e.binding !== 'member'), 'group removed')
})

await test('view: debug seed, read-only overlay, groups, provenance', async () => {
  fs.mkdirSync(hp('vphotos'))
  fs.writeFileSync(hp('vphotos/a.jpg'), 'a')
  fs.writeFileSync(hp('vphotos/b.jpg'), 'b')
  // Seed through the debug door (v1 has no AI generator).
  const seed = await (await fetch(`${BASE}/nfs/v1/debug/create_view`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      view_id: 'topic/hokkaido',
      title: '北海道之行',
      groups: [{ label: 'Day 1', by: 'time' }],
      members: [
        { path: 'home/vphotos/a.jpg', group: 'Day 1', provenance: { why: '同一行程', matched_by: 'story.im', score: 0.9 } },
        { path: 'home/vphotos/b.jpg' },
      ],
    }),
  })).json() as any
  assertEq(seed.ok, true, 'debug create_view')

  const view = await c.openView('topic/hokkaido')
  assertEq(view.kind, 'view', 'view kind')
  assertEq(view.title, '北海道之行', 'title')
  assertEq(view.capabilities.accepts_content, false, 'read-only caps')
  assert(view.flags!.includes('read_only'), 'read_only flag')

  const listing = await c.list({ uri: 'view://topic/hokkaido' })
  assertEq(listing.entries.length, 2, 'groups + ungrouped')
  assertEq(listing.entries[0].target.kind, 'group', 'group first')
  assertEq(listing.entries[0].context!.count, 1, 'group count')
  assertEq(listing.entries[1].binding, 'derived', 'derived binding')
  assertEq(listing.entries[1].canonical_path, 'cyfs:///home/vphotos/b.jpg', 'canonical path')

  const members = await c.list(listing.entries[0].target.ref as WireRef)
  assertEq(members.entries.length, 1, 'group member')
  assertEq((members.entries[0].context!.provenance as any).why, '同一行程', 'provenance kept')

  const byUri = await c.resolve('view://topic/hokkaido')
  assertEq(byUri.ref, view.ref, 'uri and open_view agree')
})

await test('meta: lazy anchoring, user-ns only', async () => {
  fs.writeFileSync(hp('m.txt'), 'm')
  const before = await c.resolve('/home/m.txt', ['ident'])
  assert(before.node_id!.startsWith('nh_'), 'browse leaves node unanchored (handle)')
  const empty = await c.getMeta('/home/m.txt')
  assertEq(empty.records.length, 0, 'no meta yet')

  await c.setMeta('/home/m.txt', [{ ns: 'user', key: 'note', value: 'important' }])
  const after = await c.resolve('/home/m.txt', ['ident', 'meta'])
  assert(after.node_id!.startsWith('n_'), 'set_meta anchors lazily')
  assertEq(after.meta_summary!.user, 1, 'meta summary')

  const err = await expectErr('PERMISSION_DENIED', () =>
    c.setMeta('/home/m.txt', [{ ns: 'ai.vision.v1', key: 'caption', value: 'x' }]), 403)
  assertEq(err.details.required_op, 'meta.write.ai.vision.v1', 'required_op explanation')
})

await test('search: name mode, cursor, scope', async () => {
  fs.mkdirSync(hp('deep/nest'), { recursive: true })
  fs.writeFileSync(hp('hokkaido-1.jpg'), 'x')
  fs.writeFileSync(hp('deep/nest/hokkaido-2.jpg'), 'x')
  fs.writeFileSync(hp('unrelated.txt'), 'x')

  const r = await c.search('hokkaido', { limit: 1 })
  assertEq(r.hits.length, 1, 'limited to 1')
  assertEq(r.hits[0].match_source, 'name', 'match source')
  assert(typeof r.hits[0].explain?.matcher === 'string', 'explain present')
  assertEq(r.sources[0].mode, 'name', 'source mode')
  assertEq(r.sources[0].state, 'ok', 'source state')
  assert(r.next_cursor, 'cursor for more')

  const r2 = await c.search('hokkaido', { limit: 10, cursor: r.next_cursor })
  assert(r2.hits.length >= 1, 'second page has hits')
  assert(r2.hits[0].canonical_path !== r.hits[0].canonical_path, 'no duplicate hit')

  const scoped = await c.search('hokkaido', { scope: '/home/deep' })
  assertEq(scoped.hits.length, 1, 'scoped search')
})

await test('grant/revoke record lifecycle', async () => {
  fs.mkdirSync(hp('public'))
  const g = await c.grant('/home/public', { ops: ['read', 'list'], ttl: 3600 })
  assert(g.token.length > 30, 'token issued')
  assertEq(g.subtree, 'cyfs:///home/public', 'subtree canonicalized')
  assert(g.expires_at! > Date.now() / 1000 - 60, 'expiry set')
  await c.revoke(g.cap_id)
  await expectErr('NOT_FOUND', () => c.revoke('cap_missing'), 404)
})

await test('watch SSE: resync-first, token filtering, container_changed', async () => {
  fs.mkdirSync(hp('observed'))
  const listing = await c.list('/home/observed')
  const token = listing.watch_token!
  const homeRef = (await c.resolve('/home')).ref as WireRef
  const obsRef = listing.container.ref as WireRef

  const stream = c.watch({ tokens: [token] })
  const next = async (): Promise<WatchEvent> => {
    const race = await Promise.race([
      stream.next(),
      new Promise<never>((_, rej) => setTimeout(() => rej(new Error('watch timeout')), 5000)),
    ])
    assert(!race.done, 'stream stayed open')
    return race.value
  }

  const first = await next()
  assertEq(first.event, 'resync', 'first event is resync (D11)')
  assertEq(first.data.reason, 'connect', 'connect reason')

  await c.mkdir(obsRef, 'newdir')
  const ev = await next()
  assertEq(ev.event, 'container_changed', 'change event')
  assertEq(ev.data.reason, 'entries_changed', 'reason')
  assert((ev.data as any).revision, 'revision carried')

  // Unwatched container changes must not leak through the token filter.
  await c.mkdir(homeRef, 'elsewhere')
  await c.mkdir(obsRef, 'marker')
  const marker = await next()
  assertEq(marker.event, 'container_changed', 'marker event')
  assert(!JSON.stringify(marker.data).includes('elsewhere'), 'no leak from unwatched dir')
  stream.close()
})

await test('reconciler: bypass rename/rebind/delete via debug reconcile', async () => {
  fs.mkdirSync(hp('watched'))
  const wRef = (await c.resolve('/home/watched')).ref as WireRef
  await c.uploadFile(wRef, 'tracked.txt', new TextEncoder().encode('original'))
  await c.setMeta('/home/watched/tracked.txt', [{ ns: 'user', key: 'k', value: 1 }])
  const stableId = (await c.resolve('/home/watched/tracked.txt', ['ident'])).node_id!
  assert(stableId.startsWith('n_'), 'anchored')

  const reconcile = async () => {
    const v = await (await fetch(`${BASE}/nfs/v1/debug/reconcile`, { method: 'POST' })).json() as any
    assertEq(v.ok, true, 'reconcile ok')
    return v.result
  }
  await reconcile() // baseline

  // 1) Bypass rename → anchor follows.
  fs.renameSync(hp('watched/tracked.txt'), hp('watched/moved.txt'))
  const rep1 = await reconcile()
  assertEq(rep1.renamed, 1, 'rename followed')
  assertEq((await c.stat(liveRef(stableId, 1), { want: ['base'] })).name, 'moved.txt', 'ref survives rename')
  assertEq((await c.getMeta(liveRef(stableId, 1))).records.length, 1, 'meta reachable')

  // 2) Bypass delete+recreate (new inode, same path) → rebind.
  fs.rmSync(hp('watched/moved.txt'))
  fs.writeFileSync(hp('watched/moved.txt'), 'overwritten by editor')
  const rep2 = await reconcile()
  assert((rep2.rebound ?? 0) >= 1 || (rep2.touched ?? 0) >= 1, `rebound/touched: ${JSON.stringify(rep2)}`)
  assertEq((await c.stat(liveRef(stableId, 1), { want: ['base'] })).size, 21, 'size after rebind')

  // 3) Bypass delete → honest stale.
  fs.rmSync(hp('watched/moved.txt'))
  const rep3 = await reconcile()
  assertEq(rep3.staled, 1, 'staled')
  await expectErr('STALE', () => c.stat(liveRef(stableId, 1)), 410)
})

await test('session lifecycle: unknown method, critical ext, bye', async () => {
  await expectErr('UNSUPPORTED', () => c.call('frobnicate', {}))
  // Critical unknown extension rejected; non-critical ignored.
  const critical = await (await fetch(`${BASE}/nfs/v1/list`, {
    method: 'POST', headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ session: c.sessionId, at: { realm: 'dfs', path: '/' }, ext: [{ id: 'x.future', critical: true }] }),
  })).json() as any
  assertEq(critical.error.code, 'UNSUPPORTED_EXT', 'critical ext rejected')
  const soft = await (await fetch(`${BASE}/nfs/v1/list`, {
    method: 'POST', headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ session: c.sessionId, at: { realm: 'dfs', path: '/' }, ext: [{ id: 'x.future', critical: false }] }),
  })).json() as any
  assertEq(soft.ok, true, 'non-critical ext ignored')

  await c.bye()
  await expectErr('PERMISSION_DENIED', () => c.list('/'))
})

// ---------------------------------------------------------------------------

console.log(`\n${passed} passed, ${failures.length} failed`)
if (failures.length > 0) {
  for (const f of failures) console.log(`  FAIL ${f}`)
  process.exit(1)
}
