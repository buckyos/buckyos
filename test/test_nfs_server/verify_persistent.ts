/**
 * Persistent-directory verification of nfs_server, driven through the TS
 * client (src/frame/desktop/src/api/nfsp_client.ts). Unlike verify_nfsp.ts this suite runs
 * against a STABLE work dir (/tmp/nfs_server_test by default via run.sh)
 * that is never deleted between runs:
 *
 *   - each run creates its work under /home/runs/<tag> and records what it
 *     built in $NFSP_HOME/persist-manifest.json;
 *   - the next run first verifies the previous run's files, collection and
 *     view all survived the server restart (anchors, filedb, handles);
 *   - covers fs (mkdir/upload/move/delete/list/read), collection and view;
 *   - covers a large file (multi-chunk tus upload + resume + range reads +
 *     秒传 dedup);
 *   - covers "objid via nfs+path, then manual read from the named store":
 *     stat(path, want:[ident]) → obj_id (sha256:*), then bypass NFSP and
 *     resolve the hash through filedb's content_index (v1's stand-in for
 *     the NamedStore, see nfs_server README「后续接入」) to read + verify
 *     the bytes straight from disk.
 *
 * Run (normally via test/test_nfs_server/run.sh persistent):
 *
 *   nfs_server --listen 127.0.0.1:3263 --data-dir /tmp/nfs_server_test/persistent/data \
 *              --export home=/tmp/nfs_server_test/persistent/home \
 *              --scan-interval-secs 0 --debug-api &
 *   NFSP_BASE=http://127.0.0.1:3263 \
 *   NFSP_HOME=/tmp/nfs_server_test/persistent/home \
 *   NFSP_DATA=/tmp/nfs_server_test/persistent/data \
 *              node test/test_nfs_server/verify_persistent.ts
 */
import * as fs from 'node:fs'
import * as path from 'node:path'
import { createHash } from 'node:crypto'
import { NfspClient, NfspError, type WireRef } from '../../src/frame/desktop/src/api/nfsp_client.ts'

const BASE = process.env.NFSP_BASE ?? 'http://127.0.0.1:3263'
const HOME = process.env.NFSP_HOME
const DATA = process.env.NFSP_DATA
if (!HOME) throw new Error('NFSP_HOME not set')
if (!DATA) throw new Error('NFSP_DATA not set')

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
async function expectErr(code: string, fn: () => Promise<unknown>): Promise<NfspError> {
  try {
    await fn()
  } catch (e) {
    if (e instanceof NfspError) {
      if (e.code !== code) throw new Error(`expected error ${code}, got ${e.code}: ${e.message}`)
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

/** Deterministic pseudo-random bytes (xorshift32) — reproducible large files. */
function prandBytes(size: number, seed: number): Uint8Array {
  const out = new Uint8Array(size)
  let x = seed >>> 0 || 0x9e3779b9
  for (let i = 0; i < size; i += 4) {
    x ^= x << 13; x >>>= 0
    x ^= x >> 17
    x ^= x << 5; x >>>= 0
    out[i] = x & 0xff
    if (i + 1 < size) out[i + 1] = (x >> 8) & 0xff
    if (i + 2 < size) out[i + 2] = (x >> 16) & 0xff
    if (i + 3 < size) out[i + 3] = (x >> 24) & 0xff
  }
  return out
}

/**
 * Manual named-store read, bypassing the NFSP data plane entirely: obj_id
 * (sha256:<hex>) → filedb content_index (v1's NamedStore stand-in) → bytes
 * from the export tree on disk. Returns the verified bytes.
 */
async function namedStoreRead(objId: string): Promise<Uint8Array> {
  assert(objId.startsWith('sha256:'), `obj_id has sha256 prefix (${objId})`)
  const hash = objId.slice('sha256:'.length)
  const { DatabaseSync } = await import('node:sqlite')
  const db = new DatabaseSync(path.join(DATA!, 'filedb.sqlite'), { readOnly: true })
  let row: { root_id: string; path: string; size: number | bigint } | undefined
  try {
    row = db
      .prepare('SELECT root_id, path, size FROM content_index WHERE hash = ?')
      .get(hash) as typeof row
  } finally {
    db.close()
  }
  assert(row, `content_index has a row for ${hash}`)
  assertEq(row.root_id, 'home', 'store row points into the home export root')
  const bytes = fs.readFileSync(hp(row.path))
  assertEq(bytes.length, Number(row.size), 'store size matches file on disk')
  assertEq(sha256hex(bytes), hash, 'manual store read: bytes hash back to obj_id')
  return bytes
}

// ---------------------------------------------------------------------------
// Manifest: what previous runs built, so this run can verify persistence.
// ---------------------------------------------------------------------------

interface RunRecord {
  tag: string
  createdAt: string
  dir: string // dfs path, e.g. /home/runs/<tag>
  files: { path: string; sha256: string; size: number }[]
  objId: string
  objPath: string
  largeSha256: string
  collectionId: string
  collectionMembers: number
  viewId: string
}
interface Manifest { runs: RunRecord[] }

const MANIFEST = hp('persist-manifest.json')
const manifest: Manifest = fs.existsSync(MANIFEST)
  ? (JSON.parse(fs.readFileSync(MANIFEST, 'utf8')) as Manifest)
  : { runs: [] }

const RUN = `r${Date.now().toString(36)}`
const RUN_DIR = `/home/runs/${RUN}`
const record: RunRecord = {
  tag: RUN,
  createdAt: new Date().toISOString(),
  dir: RUN_DIR,
  files: [],
  objId: '',
  objPath: '',
  largeSha256: '',
  collectionId: `col-${RUN}`,
  collectionMembers: 0,
  viewId: `runs/${RUN}`,
}

const c = new NfspClient({ baseUrl: BASE, uploadChunkSize: 4 * 1024 * 1024 })
await c.hello()

// ---------------------------------------------------------------------------
// 1) Persistence: everything the previous run built must have survived the
//    server restart (fresh process ⇒ new epoch/revisions, but anchors, meta,
//    collections, views and the content index live in filedb + export tree).
// ---------------------------------------------------------------------------

await test('previous run survived restart: files, obj_id, collection, view', async () => {
  const prev = manifest.runs.at(-1)
  if (!prev) {
    console.log('  (first run against this directory — seeding only)')
    return
  }
  console.log(`  verifying run ${prev.tag} from ${prev.createdAt}`)

  // Files readable through the protocol, bytes unchanged.
  for (const f of prev.files) {
    const info = await c.resolve(f.path, ['base', 'ident'])
    assertEq(info.kind, 'file', `${f.path} still a file`)
    assertEq(info.size, f.size, `${f.path} size kept`)
    const resp = await c.readFile(info.node_id!)
    const body = new Uint8Array(await resp.arrayBuffer())
    assertEq(sha256hex(body), f.sha256, `${f.path} content survived restart`)
  }

  // The anchored obj_id is still served from stat-by-path (anchor + full_hash
  // persist in filedb), and the named store still resolves it manually.
  const info = await c.stat(prev.objPath, { want: ['base', 'ident'] })
  assert(info.node_id!.startsWith('n_'), 'anchor survived restart')
  assertEq((info as Record<string, unknown>).obj_id, prev.objId, 'obj_id stable across restart')
  await namedStoreRead(prev.objId)
  await namedStoreRead(`sha256:${prev.largeSha256}`)

  // probe still knows the content (content_index is persistent).
  const probe = await c.probe([{ hash: prev.largeSha256 }])
  assertEq(probe.missing.length, 0, 'large file content still probeable')

  // Collection reopens with its members; canonical paths resolve.
  const col = await c.openCollection(prev.collectionId)
  assertEq(col.kind, 'collection', 'collection reopened')
  const colListing = await c.list({ uri: `collection://${prev.collectionId}` })
  assertEq(colListing.entries.length, prev.collectionMembers, 'collection member count kept')
  for (const e of colListing.entries) {
    if (e.target.kind !== 'group') {
      assert(e.canonical_path?.startsWith('dfs://home/'), `member ${e.name} canonical path resolves`)
    }
  }

  // View reopens with groups + members + provenance.
  const view = await c.openView(prev.viewId)
  assertEq(view.kind, 'view', 'view reopened')
  const vListing = await c.list({ uri: `view://${prev.viewId}` })
  assert(vListing.entries.length >= 2, 'view still has groups + members')
})

await test('trim: old run dirs removed via recursive delete, recent kept', async () => {
  // Keep the newest 2 previous runs; delete older run dirs through the
  // protocol (also exercises recursive delete on a persistent tree).
  const keep = manifest.runs.slice(-2)
  const drop = manifest.runs.slice(0, -2)
  for (const old of drop) {
    const name = path.posix.basename(old.dir)
    if (fs.existsSync(hp('runs', name))) {
      await c.delete('/home/runs', name, { recursive: true })
      assert(!fs.existsSync(hp('runs', name)), `${old.dir} removed from disk`)
      await expectErr('NOT_FOUND', () => c.resolve(old.dir))
    }
  }
  manifest.runs = keep
  for (const kept of keep) {
    assert(fs.existsSync(hp('runs', path.posix.basename(kept.dir))), `${kept.dir} kept`)
  }
})

// ---------------------------------------------------------------------------
// 2) fs: dirs, uploads, listing filters, move, read, delete in the run dir.
// ---------------------------------------------------------------------------

await test('fs: mkdir -p, upload, list filters, move, range read, delete', async () => {
  const mk = await c.mkdir(`${RUN_DIR}/docs/notes`)
  assertEq(mk.existed, false, 'mkdir -p created run dir chain')
  assert(fs.statSync(hp('runs', RUN, 'docs/notes')).isDirectory(), 'on disk')

  const dirRef = (await c.resolve(RUN_DIR)).ref as WireRef
  const hello = new TextEncoder().encode(`hello from ${RUN}`)
  const readme = new TextEncoder().encode(`# run ${RUN}\npersistent suite\n`)
  await c.uploadFile(dirRef, 'hello.txt', hello)
  const notesRef = (await c.resolve(`${RUN_DIR}/docs/notes`)).ref as WireRef
  await c.uploadFile(notesRef, 'readme.md', readme)
  await c.uploadFile(dirRef, 'data.bin', prandBytes(4096, 42))

  // Listing: byte order, glob and kind filters.
  const listing = await c.list(RUN_DIR, undefined, ['base', 'ident'])
  assertEq(listing.entries.map((e) => e.name), ['data.bin', 'docs', 'hello.txt'], 'byte-ordered')
  const globbed = await c.list(RUN_DIR, { filter: { name_glob: '*.txt' } })
  assertEq(globbed.entries.map((e) => e.name), ['hello.txt'], 'glob filter')
  const dirsOnly = await c.list(RUN_DIR, { filter: { kind: ['dir'] } })
  assertEq(dirsOnly.entries.map((e) => e.name), ['docs'], 'kind filter')

  // Move into the subdir; the entity follows on disk.
  await c.move(
    { parentRef: dirRef, name: 'hello.txt' },
    { parentRef: (await c.resolve(`${RUN_DIR}/docs`)).ref as WireRef, name: 'hello-moved.txt' },
  )
  assert(fs.existsSync(hp('runs', RUN, 'docs/hello-moved.txt')), 'moved on disk')
  await expectErr('NOT_FOUND', () => c.resolve(`${RUN_DIR}/hello.txt`))

  // Read roundtrip: full, range, conditional.
  const info = await c.resolve(`${RUN_DIR}/docs/hello-moved.txt`, ['base', 'ident'])
  const full = await c.readFile(info.node_id!)
  assertEq(Array.from(new Uint8Array(await full.arrayBuffer())), Array.from(hello), 'full read')
  const part = await c.readFile(info.node_id!, { range: { start: 6, end: 9 } })
  assertEq(part.status, 206, 'partial content')
  assertEq(await part.text(), 'from', 'range bytes')
  const cond = await c.readFile(info.node_id!, { ifNoneMatch: full.headers.get('etag')! })
  assertEq(cond.status, 304, 'etag revalidation')

  // Delete one file; guard on non-empty dir still applies.
  await c.delete(RUN_DIR, 'data.bin')
  assert(!fs.existsSync(hp('runs', RUN, 'data.bin')), 'deleted on disk')
  await expectErr('NOT_EMPTY', () => c.delete(RUN_DIR, 'docs'))

  record.files.push(
    { path: `${RUN_DIR}/docs/hello-moved.txt`, sha256: sha256hex(hello), size: hello.length },
    { path: `${RUN_DIR}/docs/notes/readme.md`, sha256: sha256hex(readme), size: readme.length },
  )
})

// ---------------------------------------------------------------------------
// 3) Large file: multi-chunk tus upload with resume, range reads, 秒传.
// ---------------------------------------------------------------------------

const LARGE_SIZE = 20 * 1024 * 1024 + 12345 // > 5 chunks at 4MB, unaligned tail
const large = prandBytes(LARGE_SIZE, 0xc0ffee)
const largeHash = sha256hex(large)

await test('large file: chunked upload + resume + commit hash', async () => {
  const dirRef = (await c.resolve(RUN_DIR)).ref as WireRef
  const ow = await c.openWrite({ parentRef: dirRef, name: 'large.bin', size: LARGE_SIZE })

  // First 3MB, then "interruption": resume must pick up at the server offset.
  const first = 3 * 1024 * 1024
  await c.uploadChunk(ow.fb_handle, 0, large.subarray(0, first))
  assertEq(await c.uploadOffset(ow.fb_handle), first, 'server offset after partial upload')
  await c.uploadContent(ow.fb_handle, large) // resumes from offset, 4MB chunks
  assertEq(await c.uploadOffset(ow.fb_handle), LARGE_SIZE, 'fully uploaded')

  const done = await c.commitFile(RUN_DIR, 'large.bin', {
    fbHandle: ow.fb_handle,
    leaseId: ow.lease.lease_id,
  })
  assertEq(done.obj.size, LARGE_SIZE, 'committed size')
  assertEq(done.obj.sha256, largeHash, 'server-side hash matches local hash')
  assertEq(fs.statSync(hp('runs', RUN, 'large.bin')).size, LARGE_SIZE, 'size on disk')
})

await test('large file: full + boundary range reads, etag, 秒传 dedup', async () => {
  const info = await c.resolve(`${RUN_DIR}/large.bin`, ['base', 'ident'])
  assertEq(info.size, LARGE_SIZE, 'stat size')

  const full = await c.readFile(info.node_id!)
  const body = new Uint8Array(await full.arrayBuffer())
  assertEq(body.length, LARGE_SIZE, 'full read length')
  assertEq(sha256hex(body), largeHash, 'full read hash')

  // Range straddling a 4MB chunk boundary.
  const b = 4 * 1024 * 1024
  const straddle = await c.readFile(info.node_id!, { range: { start: b - 4, end: b + 3 } })
  assertEq(straddle.status, 206, 'boundary range 206')
  assertEq(straddle.headers.get('content-range'), `bytes ${b - 4}-${b + 3}/${LARGE_SIZE}`, 'content-range')
  assertEq(
    Array.from(new Uint8Array(await straddle.arrayBuffer())),
    Array.from(large.subarray(b - 4, b + 4)),
    'boundary bytes',
  )

  // Open-ended tail range.
  const tail = await c.readFile(info.node_id!, { range: { start: LARGE_SIZE - 16 } })
  assertEq(tail.status, 206, 'tail range 206')
  assertEq(
    Array.from(new Uint8Array(await tail.arrayBuffer())),
    Array.from(large.subarray(LARGE_SIZE - 16)),
    'tail bytes',
  )

  const cond = await c.readFile(info.node_id!, { ifNoneMatch: full.headers.get('etag')! })
  assertEq(cond.status, 304, 'large etag revalidation')

  // 秒传: commit by hash only — no re-upload of 20MB.
  const probe = await c.probe([{ hash: largeHash, size: LARGE_SIZE }])
  assertEq(probe.missing.length, 0, 'content known before dedup commit')
  const dedup = await c.commitFile(RUN_DIR, 'large-copy.bin', { hash: largeHash })
  assertEq(dedup.obj.sha256, largeHash, 'dedup hash')
  const copy = fs.readFileSync(hp('runs', RUN, 'large-copy.bin'))
  assertEq(copy.length, LARGE_SIZE, 'dedup copy size')
  assertEq(sha256hex(copy), largeHash, 'dedup copy content')

  record.largeSha256 = largeHash
  record.files.push({ path: `${RUN_DIR}/large.bin`, sha256: largeHash, size: LARGE_SIZE })
})

// ---------------------------------------------------------------------------
// 4) obj_id via nfs+path, then a manual read from the named store.
//    v1's named store = filedb content_index (hash → local path); the later
//    NamedStore link mode replaces it with the same contract (README §后续接入).
// ---------------------------------------------------------------------------

await test('obj_id via stat(path) → manual named-store read bypassing NFSP', async () => {
  const dirRef = (await c.resolve(RUN_DIR)).ref as WireRef
  const content = prandBytes(1024 * 1024 + 7, 0xbeef)
  const contentHash = sha256hex(content)

  // stat-by-path only carries obj_id for anchored nodes with a known full
  // hash: upload, anchor via set_meta, then re-commit so the anchor records
  // the content hash (lazy anchoring is by design — pure browsing writes
  // nothing to filedb).
  await c.uploadFile(dirRef, 'store-probe.bin', prandBytes(2048, 1))
  await c.setMeta(`${RUN_DIR}/store-probe.bin`, [{ ns: 'user', key: 'run', value: RUN }])
  const committed = await c.uploadFile(dirRef, 'store-probe.bin', content, { overwrite: true })
  assertEq(committed.obj.sha256, contentHash, 'commit hash')

  // Step 1: nfs + path → obj_id.
  const info = await c.stat(`${RUN_DIR}/store-probe.bin`, { want: ['base', 'ident'] })
  assert(info.node_id!.startsWith('n_'), 'node is anchored (stable id)')
  const objId = (info as Record<string, unknown>).obj_id as string
  assertEq(objId, `sha256:${contentHash}`, 'stat by path returns obj_id')

  // Step 2: manual store read — resolve obj_id through the content index and
  // read the bytes from disk without touching the NFSP data plane.
  const stored = await namedStoreRead(objId)
  assertEq(Array.from(stored), Array.from(content), 'store bytes == uploaded bytes')

  // The large file's content is reachable the same way.
  const storedLarge = await namedStoreRead(`sha256:${largeHash}`)
  assertEq(storedLarge.length, LARGE_SIZE, 'large file readable from store')

  // Cross-check: protocol read of the same node serves identical bytes.
  const viaNfs = new Uint8Array(await (await c.readFile(info.node_id!)).arrayBuffer())
  assertEq(sha256hex(viaNfs), contentHash, 'NFSP read agrees with store read')

  record.objId = objId
  record.objPath = `${RUN_DIR}/store-probe.bin`
  record.files.push({ path: record.objPath, sha256: contentHash, size: content.length })
})

// ---------------------------------------------------------------------------
// 5) Collection over the persistent tree.
// ---------------------------------------------------------------------------

await test('collection: create, add_ref, groups, order, canonical paths', async () => {
  const largeRef = (await c.resolve(`${RUN_DIR}/large.bin`)).ref as WireRef
  const helloRef = (await c.resolve(`${RUN_DIR}/docs/hello-moved.txt`)).ref as WireRef
  const probeRef = (await c.resolve(`${RUN_DIR}/store-probe.bin`)).ref as WireRef

  const col = await c.createCollection(`Run ${RUN}`, record.collectionId)
  assertEq(col.kind, 'collection', 'collection kind')
  assertEq(col.capabilities.remove_semantics, 'unlink', 'unlink semantics')
  const cref = col.ref as WireRef

  const patched = await c.collectionPatch(cref, [
    { add_ref: { target_ref: largeRef } },
    { add_ref: { target_ref: helloRef, position: 0, name: 'greeting' } },
    { create_group: { name: 'blobs' } },
  ], { expectedRevision: col.revision! })
  assert(patched.revision !== col.revision, 'revision advanced')

  const listing = await c.list({ uri: `collection://${record.collectionId}` })
  assertEq(listing.container.kind, 'collection', 'listed as collection')
  assertEq(listing.entries[0].name, 'greeting', 'manual order honored')
  assertEq(listing.entries[0].canonical_path, `dfs://home/runs/${RUN}/docs/hello-moved.txt`, 'canonical path')
  const group = listing.entries.find((e) => e.target.kind === 'group')!
  assertEq(group.binding, 'member', 'group binding')

  // Populate the group, then rename it; the group lists as its own container.
  await c.collectionPatch(cref, [
    { add_ref: { target_ref: probeRef, parent_entry_ref: group.entry_ref! } },
    { rename_group: { entry_ref: group.entry_ref!, name: 'kept-blobs' } },
  ])
  const groupListing = await c.list(group.target.ref as WireRef)
  assertEq(groupListing.container.kind, 'group', 'group container')
  assertEq(groupListing.entries.length, 1, 'group member added')
  assertEq(groupListing.entries[0].canonical_path, `dfs://home/runs/${RUN}/store-probe.bin`, 'group member path')

  const reopened = await c.openCollection(record.collectionId)
  assertEq(reopened.ref, cref, 'open_collection resolves same node')
  record.collectionMembers = (await c.list({ uri: `collection://${record.collectionId}` })).entries.length
})

// ---------------------------------------------------------------------------
// 6) View over the persistent tree (seeded via the debug door — v1 has no
//    AI generator), read-only overlay with groups + provenance.
// ---------------------------------------------------------------------------

await test('view: seed, open, groups, provenance, read-only', async () => {
  const seed = await (await fetch(`${BASE}/nfs/v1/debug/create_view`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      view_id: record.viewId,
      title: `Run ${RUN} 摘要`,
      groups: [{ label: 'big', by: 'size' }],
      members: [
        {
          path: `home/runs/${RUN}/large.bin`,
          group: 'big',
          provenance: { why: 'large-file pipeline', matched_by: 'test', score: 1 },
        },
        { path: `home/runs/${RUN}/docs/hello-moved.txt` },
      ],
    }),
  })).json() as { ok: boolean }
  assertEq(seed.ok, true, 'debug create_view')

  const view = await c.openView(record.viewId)
  assertEq(view.kind, 'view', 'view kind')
  assertEq(view.title, `Run ${RUN} 摘要`, 'title')
  assertEq(view.capabilities.accepts_content, false, 'view rejects content')
  assert(view.flags!.includes('read_only'), 'read_only flag')

  const listing = await c.list({ uri: `view://${record.viewId}` })
  assertEq(listing.entries.length, 2, 'group + ungrouped member')
  const group = listing.entries.find((e) => e.target.kind === 'group')!
  assertEq(group.context!.count, 1, 'group count')
  const loose = listing.entries.find((e) => e.target.kind !== 'group')!
  assertEq(loose.binding, 'derived', 'derived binding')
  assertEq(loose.canonical_path, `dfs://home/runs/${RUN}/docs/hello-moved.txt`, 'member canonical path')

  const members = await c.list(group.target.ref as WireRef)
  assertEq(members.entries.length, 1, 'group member listed')
  assertEq(
    (members.entries[0].context!.provenance as { why: string }).why,
    'large-file pipeline',
    'provenance kept',
  )

  const byUri = await c.resolve(`view://${record.viewId}`)
  assertEq(byUri.ref, view.ref, 'uri and open_view agree')
})

// ---------------------------------------------------------------------------
// Manifest update — the next run verifies everything recorded here.
// ---------------------------------------------------------------------------

await test('manifest recorded for the next run', async () => {
  assert(record.files.length >= 3 && record.objId && record.largeSha256, 'record complete')
  manifest.runs.push(record)
  fs.writeFileSync(MANIFEST, JSON.stringify(manifest, null, 2))
  // The bypass-written manifest is immediately visible through the protocol.
  const info = await c.resolve('/home/persist-manifest.json', ['base'])
  assertEq(info.kind, 'file', 'manifest visible via NFSP')
})

await c.bye()

// ---------------------------------------------------------------------------

console.log(`\n${passed} passed, ${failures.length} failed (run ${RUN}, home=${HOME})`)
if (failures.length > 0) {
  for (const f of failures) console.log(`  FAIL ${f}`)
  process.exit(1)
}
