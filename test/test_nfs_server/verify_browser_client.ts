/**
 * Verification of NfsBrowserClient (src/api/nfs_browser_client.ts): the
 * caching layer over the already-verified NfspClient, exercised against a
 * real nfs_server. Uses an in-memory CacheStore standing in for
 * localStorage and a counting fetch to prove request behavior.
 *
 * Run (Node ≥ 23; same machine as the server, fresh server per run):
 *
 *   cargo build -p nfs_server
 *   mkdir -p /tmp/nfsb-test/home /tmp/nfsb-test/data
 *   nfs_server --listen 127.0.0.1:3262 --data-dir /tmp/nfsb-test/data \
 *              --export home=/tmp/nfsb-test/home --scan-interval-secs 0 --debug-api &
 *   NFSP_BASE=http://127.0.0.1:3262 NFSP_HOME=/tmp/nfsb-test/home \
 *              node test/test_nfs_server/verify_browser_client.ts
 */
import * as fs from 'node:fs'
import * as path from 'node:path'
import { NfspClient, NfspError, liveRef, type WireRef } from '../../src/frame/desktop/src/api/nfsp_client.ts'
import { NfsBrowserClient, type CacheStore } from '../../src/frame/desktop/src/api/nfs_browser_client.ts'

const BASE = process.env.NFSP_BASE ?? 'http://127.0.0.1:3262'
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
async function waitFor(cond: () => boolean, what: string, ms = 8000) {
  const t0 = Date.now()
  while (!cond()) {
    if (Date.now() - t0 > ms) throw new Error(`timeout waiting for ${what}`)
    await new Promise((r) => setTimeout(r, 50))
  }
}
const hp = (...seg: string[]) => path.join(HOME!, ...seg)

// ---------------------------------------------------------------------------
// Test doubles: in-memory store (localStorage stand-in) + counting fetch
// ---------------------------------------------------------------------------

class MemoryCacheStore implements CacheStore {
  readonly map = new Map<string, string>()
  async get(key: string): Promise<string | null> {
    return this.map.get(key) ?? null
  }
  async set(key: string, value: string): Promise<void> {
    this.map.set(key, value)
  }
  async delete(key: string): Promise<void> {
    this.map.delete(key)
  }
  async deletePrefix(prefix: string): Promise<void> {
    for (const k of [...this.map.keys()]) if (k.startsWith(prefix)) this.map.delete(k)
  }
  async keys(prefix: string): Promise<string[]> {
    return [...this.map.keys()].filter((k) => k.startsWith(prefix))
  }
  dataKeys(): string[] {
    return [...this.map.keys()].filter((k) => /^nfsp:v1:[^:]+:(list|stat|meta):/.test(k))
  }
}

/** MemoryCacheStore whose next N `set`s throw, simulating QuotaExceededError. */
class QuotaCacheStore extends MemoryCacheStore {
  failNextSets = 0
  override async set(key: string, value: string): Promise<void> {
    if (this.failNextSets > 0) {
      this.failNextSets--
      throw new Error('QuotaExceededError (simulated)')
    }
    await super.set(key, value)
  }
}

let fetchCount = 0
const countingFetch: typeof fetch = (...args) => {
  fetchCount++
  return globalThis.fetch(...args)
}

/** Rewinds cached entries' `at` so they look expired (SWR path). */
const ageEntries = (store: MemoryCacheStore, ms: number) => {
  for (const k of store.dataKeys()) {
    const p = JSON.parse(store.map.get(k)!) as { at: number }
    p.at -= ms
    store.map.set(k, JSON.stringify(p))
  }
}

const store = new MemoryCacheStore()
const bc = new NfsBrowserClient({
  baseUrl: BASE,
  store,
  autoWatch: false,
  client: new NfspClient({ baseUrl: BASE, fetch: countingFetch }),
})
// Independent protocol client: writes through it bypass bc's direct-write
// invalidation, standing in for "another tab / another user".
const other = new NfspClient({ baseUrl: BASE })

let ATTR_TTL = 5000

// ---------------------------------------------------------------------------

await test('hello: passthrough, limits available for TTL', async () => {
  const r = await bc.hello()
  assertEq(r.version, 'nfsp/0', 'protocol version')
  assert(r.limits.attr_ttl_ms > 0, 'attr_ttl_ms present')
  ATTR_TTL = r.limits.attr_ttl_ms
  await other.hello()
})

await test('cache hit: repeat reads answer from cache, no request', async () => {
  fs.mkdirSync(hp('hit'))
  fs.writeFileSync(hp('hit/a.txt'), 'aaaaa')

  const l1 = await bc.list('/home/hit', undefined, ['base', 'ident'])
  const r1 = await bc.resolve('/home/hit')
  const s1 = await bc.stat('/home/hit', { name: 'a.txt', want: ['base'] })
  const n = fetchCount
  const l2 = await bc.list('/home/hit', undefined, ['base', 'ident'])
  const r2 = await bc.resolve('/home/hit')
  const s2 = await bc.stat('/home/hit', { name: 'a.txt', want: ['base'] })
  assertEq(fetchCount, n, 'no requests on cache hits')
  assertEq(l2!.entries, l1!.entries, 'listing identical')
  assertEq(r2!.ref, r1!.ref, 'resolve identical')
  assertEq(s2!.size, s1!.size, 'stat identical')
  assert(bc.cacheStats().hits >= 3, 'hits counted')

  // Different want mask is a different entry → its own miss.
  await bc.list('/home/hit', undefined, ['base'])
  assert(fetchCount > n, 'different want fetches')
})

await test('stale-while-revalidate: stale value first, onInvalidate after', async () => {
  fs.mkdirSync(hp('swr'))
  fs.writeFileSync(hp('swr/old.txt'), 'x')
  const cached = await bc.list('/home/swr')
  assertEq(cached!.entries.map((e) => e.name), ['old.txt'], 'initial listing')

  // A foreign writer changes the dir (bumps its revision), cache unaware.
  await other.mkdir(cached!.container.ref as WireRef, 'added-behind-back')
  ageEntries(store, ATTR_TTL * 20)

  let fired: WireRef | null | undefined
  const off = bc.onInvalidate((ref) => {
    fired = ref
  })
  const stale = await bc.list('/home/swr')
  assertEq(stale!.entries.map((e) => e.name), ['old.txt'], 'stale value returned immediately')
  await waitFor(() => fired !== undefined, 'revalidation onInvalidate')
  off()
  assert(fired && (fired as WireRef).type === 'live', 'invalidated ref is the container')
  assert(bc.cacheStats().revalidations >= 1, 'revalidation counted')

  const fresh = await bc.list('/home/swr')
  assertEq(
    fresh!.entries.map((e) => e.name),
    ['added-behind-back', 'old.txt'],
    'cache updated by revalidation',
  )
})

await test('direct-write invalidation: own mkdir/delete/upload seen at once', async () => {
  fs.mkdirSync(hp('dw'))
  const listing = await bc.list('/home/dw')
  const dwRef = listing!.container.ref as WireRef

  await bc.mkdir(dwRef, 'newdir')
  let n = fetchCount
  const after = await bc.list('/home/dw')
  assert(fetchCount > n, 'list went back to origin after own mkdir')
  assertEq(after!.entries.map((e) => e.name), ['newdir'], 'own write visible')

  // Path-form mkdir -p invalidates the ancestor chain too.
  await bc.list('/home/dw') // re-prime cache
  await bc.mkdir('/home/dw/deep/nested')
  n = fetchCount
  const withDeep = await bc.list('/home/dw')
  assert(fetchCount > n, 'ancestor listing invalidated by mkdir -p')
  assertEq(withDeep!.entries.map((e) => e.name), ['deep', 'newdir'], 'new subtree visible')

  // uploadFile then delete: both invalidate the parent and the child's stat.
  await bc.uploadFile(dwRef, 'f.txt', new TextEncoder().encode('hello'))
  assertEq((await bc.stat('/home/dw', { name: 'f.txt' }))!.size, 5, 'uploaded stat')
  await bc.delete('/home/dw', 'f.txt')
  assert(!(await bc.list('/home/dw'))!.entries.some((e) => e.name === 'f.txt'), 'delete visible')
  await expectErr('NOT_FOUND', () => bc.stat('/home/dw', { name: 'f.txt' }))

  // setMeta invalidates the cached meta entry.
  fs.writeFileSync(hp('dw/m.txt'), 'm')
  assertEq((await bc.getMeta('/home/dw/m.txt'))!.records.length, 0, 'no meta yet')
  await bc.setMeta('/home/dw/m.txt', [{ ns: 'user', key: 'k', value: 42 }])
  assertEq((await bc.getMeta('/home/dw/m.txt'))!.records[0].value, 42, 'meta write visible at once')
})

await test("cache modes: 'no-cache' refetches, 'only-if-cached' never fetches", async () => {
  fs.mkdirSync(hp('modes'))
  fs.writeFileSync(hp('modes/a'), 'a')
  await bc.list('/home/modes')

  let n = fetchCount
  const forced = await bc.list('/home/modes', undefined, undefined, { cache: 'no-cache' })
  assertEq(fetchCount, n + 1, 'no-cache always fetches')
  assertEq(forced!.entries.length, 1, 'no-cache result')

  n = fetchCount
  const hit = await bc.list('/home/modes', undefined, undefined, { cache: 'only-if-cached' })
  const miss = await bc.list('/home/never-listed', undefined, undefined, { cache: 'only-if-cached' })
  assertEq(fetchCount, n, 'only-if-cached never fetches')
  assertEq(hit!.entries.length, 1, 'only-if-cached hit')
  assertEq(miss, null, 'only-if-cached miss → null')
})

await test('pagination: cursor requests pass through uncached', async () => {
  fs.mkdirSync(hp('pg'))
  for (let i = 0; i < 15; i++) fs.writeFileSync(hp('pg', `f${String(i).padStart(2, '0')}`), 'x')
  const p1 = await bc.list('/home/pg', { limit: 10 })
  assertEq(p1!.truncated, true, 'first page truncated')
  const n = fetchCount
  const p2a = await bc.list('/home/pg', { limit: 10, cursor: p1!.next_cursor })
  const p2b = await bc.list('/home/pg', { limit: 10, cursor: p1!.next_cursor })
  assertEq(fetchCount, n + 2, 'cursor pages hit origin every time')
  assertEq(p2a!.entries[0].name, 'f10', 'cursor continues')
  assertEq(p2b!.entries.length, p2a!.entries.length, 'stable continuation')
})

await test('watch: resync clears all, container_changed invalidates one dir', async () => {
  fs.mkdirSync(hp('wd'))
  fs.writeFileSync(hp('wd/seed'), 'x')
  await bc.list('/home/wd')
  assert(store.dataKeys().length > 0, 'cache primed')

  const events: (WireRef | null)[] = []
  const off = bc.onInvalidate((ref) => {
    events.push(ref)
  })

  // First event on any connection is resync → full wipe (lossy contract).
  bc.connectWatch()
  await waitFor(() => events.includes(null), 'resync on connect')
  assertEq(store.dataKeys().length, 0, 'resync wiped the whole cache')
  await waitFor(() => bc.watchConnected, 'watch healthy')

  // Foreign write → container_changed → only that container invalidated.
  const wd = await bc.list('/home/wd')
  const unrelated = await bc.list('/home/modes')
  assert(unrelated!.entries.length > 0, 'unrelated dir cached')
  events.length = 0
  await other.mkdir(wd!.container.ref as WireRef, 'pushed')
  await waitFor(() => events.some((r) => r !== null), 'container_changed invalidate')
  const pushedRef = events.find((r) => r !== null) as WireRef
  assertEq(pushedRef, wd!.container.ref, 'callback carries the container ref')
  const n = fetchCount
  assert((await bc.list('/home/modes'))!.entries.length > 0, 'unrelated dir still cached')
  assertEq(fetchCount, n, 'unrelated dir untouched (no refetch)')
  const refreshed = await bc.list('/home/wd')
  assert(fetchCount > n, 'invalidated dir refetched')
  assert(refreshed!.entries.some((e) => e.name === 'pushed'), 'pushed change visible')

  // Reconnect → resync again → full wipe again.
  events.length = 0
  bc.disconnectWatch()
  bc.connectWatch()
  await waitFor(() => events.includes(null), 'resync on reconnect')
  assertEq(store.dataKeys().length, 0, 'reconnect wiped the cache')
  off()
  bc.disconnectWatch()
})

await test('quota: failed write evicts oldest 25% then retries once', async () => {
  const qstore = new QuotaCacheStore()
  const qc = new NfsBrowserClient({ baseUrl: BASE, store: qstore, autoWatch: false })
  await qc.hello()
  fs.mkdirSync(hp('qa'))
  fs.mkdirSync(hp('qb'))
  fs.writeFileSync(hp('qa/old'), 'x')
  fs.writeFileSync(hp('qb/new'), 'x')

  await qc.list('/home/qa') // oldest entry
  const qaKeys = qstore.dataKeys()
  assertEq(qaKeys.length, 1, 'one cached entry before quota hit')

  qstore.failNextSets = 1 // the next data write hits "quota"
  const r = await qc.list('/home/qb')
  assertEq(r!.entries.map((e) => e.name), ['new'], 'read result unaffected by quota failure')
  assert(qc.cacheStats().evictions >= 1, 'eviction counted')
  assert(!qstore.map.has(qaKeys[0]), 'oldest entry evicted')
  assert(qstore.dataKeys().some((k) => k.includes('/home/qb')), 'retry stored the new entry')

  // A store that keeps failing must still never break reads.
  qstore.failNextSets = 1000
  const r2 = await qc.list('/home/qa')
  assertEq(r2!.entries.map((e) => e.name), ['old'], 'reads survive a dead store')
  await qc.bye()
})

await test('session self-heal: expired session re-hellos and replays once', async () => {
  fs.mkdirSync(hp('heal'))
  await bc.raw.bye() // kill the session behind the wrapper's back
  const l = await bc.list('/home/heal', undefined, undefined, { cache: 'no-cache' })
  assertEq(l!.container.kind, 'dir', 'read replayed under a fresh session')
  const mk = await bc.mkdir(l!.container.ref as WireRef, 'sub')
  assertEq(mk.existed, false, 'writes work after self-heal')
})

await test('equivalence: browse semantics unchanged through the cache layer', async () => {
  fs.mkdirSync(hp('eq/docs'), { recursive: true })
  fs.writeFileSync(hp('eq/a.txt'), 'hello')

  const dir = await bc.resolve('/home/eq')
  assertEq(dir!.kind, 'dir', 'resolve kind')
  assert(dir!.capabilities.list, 'listable')
  const listing = await bc.list('/home/eq', undefined, ['base', 'ident'])
  assertEq(listing!.entries.map((e) => e.name), ['a.txt', 'docs'], 'byte-ordered names')
  assertEq(listing!.entries[0].target.attrs!.size, 5, 'size via want=base')
  const info = await bc.stat(listing!.entries[0].target.ref, { want: ['base', 'ident', 'access'] })
  assertEq(info!.kind, 'file', 'stat by ref')
  assert(info!.access_urls!.some((u) => u.kind === 'fs' && u.url === 'cyfs:///home/eq/a.txt'), 'fs url')

  // Errors pass through and are never cached.
  await expectErr('NOT_FOUND', () => bc.resolve('/nope'))
  await expectErr('NOT_FOUND', () => bc.resolve('/nope'))
  await expectErr('NOT_A_CONTAINER', () => bc.list('/home/eq/a.txt'))
})

await test('equivalence: move keeps stable refs and cache coherent', async () => {
  fs.mkdirSync(hp('mv-src'))
  fs.mkdirSync(hp('mv-dst'))
  const srcRef = (await bc.resolve('/home/mv-src'))!.ref as WireRef
  const dstRef = (await bc.resolve('/home/mv-dst'))!.ref as WireRef
  await bc.uploadFile(srcRef, 'file.txt', new TextEncoder().encode('content!'))
  await bc.setMeta('/home/mv-src/file.txt', [{ ns: 'user', key: 'rating', value: 5 }])
  const stableId = (await bc.resolve('/home/mv-src/file.txt', ['ident']))!.node_id!
  assert(stableId.startsWith('n_'), 'anchored by set_meta')

  await bc.list('/home/mv-src')
  await bc.list('/home/mv-dst')
  await bc.move({ parentRef: srcRef, name: 'file.txt' }, { parentRef: dstRef, name: 'renamed.txt' })

  // Both containers were invalidated by the own-write path: fresh listings.
  assertEq((await bc.list('/home/mv-src'))!.entries.length, 0, 'source emptied')
  assertEq((await bc.list('/home/mv-dst'))!.entries.map((e) => e.name), ['renamed.txt'], 'dest updated')
  const after = await bc.stat(liveRef(stableId, 1), { want: ['base'] })
  assertEq(after!.name, 'renamed.txt', 'stable ref follows move')
  assertEq((await bc.getMeta(liveRef(stableId, 1)))!.records[0].value, 5, 'meta survives move')
})

// ---------------------------------------------------------------------------

bc.disconnectWatch()
try {
  await bc.bye()
} catch {
  /* session may already be gone */
}
try {
  await other.bye()
} catch {
  /* ignore */
}

console.log(`\n${passed} passed, ${failures.length} failed`)
for (const f of failures) console.log(`  FAIL ${f}`)
process.exit(failures.length > 0 ? 1 : 0)
