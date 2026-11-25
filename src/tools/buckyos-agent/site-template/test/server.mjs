#!/usr/bin/env node
// Serves the template the way `reflow.<zone>` does (Personal Site Protocol §2, §6.1, §7), for tests and for looking at it:
//
//   node server.mjs [--port 4819]
//   http://<case>.<run>.localhost:<port>/     e.g. http://base.me.localhost:4819/
//
// <case> is `base` (the shared fixtures plus data/base), a directory of data/, or `big` / `wide` (10 sources x 20
// items, in four columns / in one). <run> is any name: each one has its own settings, changed data and log, set with
// POST /__test/<run> { auth, drop, delay, slow, overrides: { "/_data/…": <json> | null } } and read with GET /__test/<run>:
// `auth` is open | expired (no session until POST /sso_refresh) | locked (no session, and none to renew); without a
// session a navigation gets the host's sign-in page and anything else 401. `drop` (true, or "locked") ends the session
// right after the next page was served. `delay` holds back the answers under the path prefix `slow` (default /_data/) for that many
// milliseconds. /login stands in for the Zone's login page.

import http from 'node:http'
import { createHash } from 'node:crypto'
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs'
import { dirname, extname, join, resolve, sep } from 'node:path'
import { fileURLToPath } from 'node:url'
import { crc32, deflateSync } from 'node:zlib'

const here = dirname(fileURLToPath(import.meta.url))
const template = resolve(here, '..')
const fixtures = resolve(here, '../../../../../doc/opendan/protocol/fixtures/site')
const TYPES = {
  '.html': 'text/html; charset=utf-8', '.js': 'text/javascript', '.mjs': 'text/javascript', '.css': 'text/css',
  '.json': 'application/json', '.png': 'image/png', '.jpg': 'image/jpeg', '.jpeg': 'image/jpeg', '.webp': 'image/webp',
}
const READ_BODY_BYTES = 16 << 10
const SIGN_IN_PAGE = `<!doctype html><meta charset="utf-8"><title>Sign in</title>
<script>
fetch('/sso_refresh', { method: 'POST', credentials: 'same-origin' })
  .then((response) => response.ok, () => false)
  .then((renewed) => (renewed ? location.reload() : location.replace('/login?redirect_url=' + encodeURIComponent(location.href))))
</script>`
const LOGIN_PAGE = '<!doctype html><meta charset="utf-8"><title>Zone login</title><p id="login-stub">the login page of the Zone</p>'

const fixture = (name) => JSON.parse(readFileSync(join(fixtures, name), 'utf8'))
const body = (value) => Buffer.from(JSON.stringify(value))

function walk(dir, into, prefix = '') {
  if (!existsSync(dir)) return into
  for (const name of readdirSync(dir)) {
    const path = join(dir, name)
    if (statSync(path).isDirectory()) walk(path, into, `${prefix}${name}/`)
    else into.set(`/_data/${prefix}${name}`, readFileSync(path))
  }
  return into
}

function baseCase() {
  const feed = fixture('feed.json')
  const article = fixture('article.json')
  const data = new Map([
    ['/_data/site.json', body(fixture('site.json'))],
    [`/_data/feed/${feed.source}.json`, body(feed)],
    [`/_data/a/${article.id}.json`, body(article)],
    ['/_data/events.json', body(fixture('events.json'))],
    ['/_data/reading.json', body(fixture('reading.json'))],
  ])
  return walk(join(here, 'data/base'), data)
}

function bigCase(columns) {
  const site = fixture('site.json')
  const seed = fixture('feed.json')
  const [first, second] = seed.items
  const sources = Array.from({ length: 10 }, (_, i) => ({
    ...site.sources[0], id: `source-${String(i + 1).padStart(2, '0')}`, title: `来源 ${i + 1}`, job: `src-source-${i + 1}`,
  }))
  let at = 0
  site.sources = sources
  site.upgrades = []
  site.template.params.hide_topics = []
  site.columns = columns.map((size, i) => ({ id: `c${i + 1}`, title: `栏目 ${i + 1}`, sources: sources.slice(at, at += size).map((s) => s.id) }))
  const data = new Map([['/_data/site.json', body(site)]])
  sources.forEach((source, s) => {
    const items = Array.from({ length: 20 }, (_, i) => {
      const like = i % 2 ? second : first
      const id = createHash('sha256').update(`${source.id}/${i}`).digest('hex')
      return {
        ...like, id: id.slice(0, 16), object_id: `cyfeed:${id}`, url: `${like.url.slice(0, -40)}${id.slice(0, 40)}`,
        title: `${like.title}（${s + 1}-${i + 1}）`, image: i % 2 ? null : `_data/img/${id}.jpg`,
        published_at: seed.updated_at - (i * 10 + s) * 360000,
      }
    })
    data.set(`/_data/feed/${source.id}.json`, body({ ...seed, source: source.id, status: { ...seed.status, pending: null }, items }))
  })
  return data
}

const cases = new Map([['base', baseCase()], ['big', bigCase([3, 3, 2, 2])], ['wide', bigCase([10])]])
for (const name of readdirSync(join(here, 'data'))) if (name !== 'base') cases.set(name, walk(join(here, 'data', name), new Map()))

function picture(name) {
  const seed = createHash('sha256').update(name).digest()
  const portrait = seed[3] % 5 === 0
  const [w, h] = portrait ? [270, 480] : [480, 270]
  const tone = (i, base) => [base + seed[i] % 40, base + seed[i + 1] % 40, base + seed[i + 2] % 40]
  const sky = tone(0, 150), land = tone(4, 90), sun = tone(8, 200)
  const horizon = Math.round(h * (0.5 + (seed[12] % 20) / 100)), cx = Math.round(w * (0.25 + (seed[13] % 50) / 100)), cy = Math.round(horizon * 0.5), r = Math.round(h * 0.12)
  const raw = Buffer.alloc((w * 3 + 1) * h)
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const color = (x - cx) ** 2 + (y - cy) ** 2 < r * r ? sun : y < horizon ? sky : land
      raw.set(color, y * (w * 3 + 1) + 1 + x * 3)
    }
  }
  const chunk = (type, data) => {
    const part = Buffer.concat([Buffer.from(type), data])
    const size = Buffer.alloc(4), sum = Buffer.alloc(4)
    size.writeUInt32BE(data.length)
    sum.writeUInt32BE(crc32(part))
    return Buffer.concat([size, part, sum])
  }
  const head = Buffer.alloc(13)
  head.writeUInt32BE(w, 0)
  head.writeUInt32BE(h, 4)
  head.set([8, 2, 0, 0, 0], 8)
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', head), chunk('IDAT', deflateSync(raw)), chunk('IEND', Buffer.alloc(0))])
}

const runs = new Map()
function runOf(id) {
  if (!runs.has(id)) runs.set(id, { auth: 'open', drop: false, delay: 0, slow: '/_data/', overrides: {}, reads: [], log: [] })
  return runs.get(id)
}

function readBody(req, limit) {
  return new Promise((done, fail) => {
    const parts = []
    let size = 0
    req.on('data', (part) => {
      size += part.length
      if (size <= limit) parts.push(part)
    })
    req.on('end', () => (size > limit ? fail(new Error('too large')) : done(Buffer.concat(parts))))
    req.on('error', fail)
  })
}

const isCount = (value) => Number.isSafeInteger(value) && value >= 0
const isSource = (id) => id === '_supplement' || (/^[a-z0-9-]{1,48}$/.test(id) && !id.startsWith('-') && !id.endsWith('-'))
const isRecord = (r) => r !== null && typeof r === 'object' && !Array.isArray(r) && Object.keys(r).length === 5
  && typeof r.article === 'string' && /^[0-9a-f]{16}$/.test(r.article) && typeof r.source === 'string' && isSource(r.source)
  && isCount(r.opened_at) && isCount(r.dwell_ms) && isCount(r.max_scroll) && r.max_scroll <= 100

function isReadingBatch(bytes) {
  try {
    const records = JSON.parse(bytes.toString())
    return Array.isArray(records) && records.length >= 1 && records.length <= 50 && records.every(isRecord)
  } catch {
    return false
  }
}

function originHost(value) {
  try { return new URL(value).hostname } catch { return null }
}

async function handle(req, res) {
  const url = new URL(req.url, 'http://x')
  const path = decodeURIComponent(url.pathname)
  const hostName = (req.headers.host ?? '').split(':')[0]
  const labels = hostName.endsWith('.localhost') ? hostName.slice(0, -'.localhost'.length).split('.') : []
  const data = cases.get(labels[0]) ?? cases.get('base')
  const run = runOf(labels[1] ?? labels[0] ?? 'default')
  const send = (status, headers = {}, content = '') => {
    const bytes = Buffer.isBuffer(content) ? content : Buffer.from(content)
    run.log.push({ method: req.method, path, status, bytes: bytes.length })
    res.writeHead(status, { 'X-Content-Type-Options': 'nosniff', 'Referrer-Policy': 'no-referrer', 'Content-Length': bytes.length, ...headers })
    res.end(req.method === 'HEAD' ? undefined : bytes)
  }
  const plain = (status, text) => send(status, { 'Content-Type': 'text/plain; charset=utf-8', 'Cache-Control': 'no-store' }, text)
  const stored = (content, type, cache = 'private, no-cache') => {
    const etag = `"${createHash('sha1').update(content).digest('hex')}"`
    if (req.headers['if-none-match'] === etag) return send(304, { ETag: etag, 'Cache-Control': cache })
    send(200, { 'Content-Type': type, ETag: etag, 'Cache-Control': cache }, content)
  }

  if (path.startsWith('/__test/')) {
    const target = runOf(path.slice('/__test/'.length))
    if (req.method === 'POST') {
      const next = JSON.parse((await readBody(req, 4 << 20)).toString() || '{}')
      Object.assign(target, { ...next, overrides: { ...target.overrides, ...next.overrides } })
    }
    res.writeHead(200, { 'Content-Type': 'application/json', 'Cache-Control': 'no-store' })
    return res.end(JSON.stringify(target))
  }
  if (path === '/__parent.html') return send(200, { 'Content-Type': TYPES['.html'], 'Cache-Control': 'no-store' }, readFileSync(join(here, 'parent.html')))

  if (path === '/sso_refresh') {
    if (req.method !== 'POST') return plain(405, 'POST')
    if (run.auth === 'locked') return plain(401, 'no session to refresh')
    run.auth = 'open'
    return send(204, { 'Cache-Control': 'no-store' })
  }
  if (path === '/login') return send(200, { 'Content-Type': TYPES['.html'], 'Cache-Control': 'no-store' }, LOGIN_PAGE)

  // From here on it is the host as Personal Site Protocol §7 states it, the checks in its order.
  const report = req.method === 'POST' && path === '/_read'
  if (req.method !== 'GET' && req.method !== 'HEAD' && !report) return plain(405, 'the reflow library is read with GET')
  if (run.auth !== 'open') {
    const navigation = req.method === 'GET' && (req.headers.accept ?? '').includes('text/html')
    return navigation ? send(200, { 'Content-Type': TYPES['.html'], 'Cache-Control': 'no-store' }, SIGN_IN_PAGE) : plain(401, 'sign in to open this page')
  }
  if (report) {
    if (originHost(req.headers.origin) !== hostName) return plain(403, 'reading records are reported by the pages of this host')
    let bytes
    try { bytes = await readBody(req, READ_BODY_BYTES) } catch { return plain(413, 'a reading report is at most 16 KiB') }
    if (!isReadingBatch(bytes)) return plain(400, 'a reading report is a JSON array of 1 to 50 reading records')
    run.reads.push({ type: req.headers['content-type'] ?? '', origin: req.headers.origin, records: JSON.parse(bytes.toString()) })
    return send(204)
  }

  if (run.delay && path.startsWith(run.slow)) await new Promise((done) => setTimeout(done, run.delay))
  if (path in run.overrides) {
    const value = run.overrides[path]
    return value === null ? plain(404, 'not found') : stored(body(value), TYPES['.json'])
  }
  if (path === '/' || path === '/index.html') {
    if (run.drop) Object.assign(run, { drop: false, auth: run.drop === 'locked' ? 'locked' : 'expired' })
    return stored(readFileSync(join(template, 'index.html')), TYPES['.html'])
  }
  const version = /^\/_app\/t\d+\//.exec(path)
  if (version) {
    const file = resolve(template, path.slice(version[0].length))
    const inside = file.startsWith(template + sep) && !file.startsWith(join(template, 'test') + sep) && file !== join(template, 'index.html')
    if (!inside || !existsSync(file) || !statSync(file).isFile()) return plain(404, 'not found')
    return stored(readFileSync(file), TYPES[extname(file)] ?? 'application/octet-stream', 'public, max-age=31536000, immutable')
  }
  if (/^\/_data\/img\/[0-9a-f]{64}\.[a-z]+$/.test(path)) return stored(picture(path), TYPES[extname(path)] ?? 'image/png', 'public, max-age=31536000, immutable')
  if (path.startsWith('/_data/')) {
    return data.has(path) ? stored(data.get(path), TYPES[extname(path)] ?? 'application/octet-stream') : plain(404, 'not found')
  }
  plain(404, 'not found')
}

const at = process.argv.indexOf('--port')
const port = Number(at > 0 ? process.argv[at + 1] : process.env.SITE_TEMPLATE_PORT ?? 4819)
const serve = (req, res) => handle(req, res).catch((e) => {
  res.writeHead(500, { 'Content-Type': 'text/plain' })
  res.end(String(e))
})
http.createServer(serve).listen(port, '127.0.0.1', () => console.log(`site template: http://base.me.localhost:${port}/`))
http.createServer(serve).on('error', () => {}).listen(port, '::1')
