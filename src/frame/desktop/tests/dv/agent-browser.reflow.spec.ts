/**
 * Reflow mode of the Agent browser on a real Zone (notepads/agent-runtime-browser-todo.md §8.4 "重排";
 * doc/opendan/protocol/Agent Browser Protocol.md §5):
 *
 *   BUCKYOS_TEST_ZONE_HOST=test.buckyos.io BUCKYOS_TEST_ADMIN_PASSWORD=… BUCKYOS_TEST_REFLOW_AGENT=reflowy.test.buckyos.io \
 *     ./node_modules/.bin/playwright test --config=playwright.dv.config.ts tests/dv/agent-browser.reflow.spec.ts
 *
 * Needs:
 * - the deployed Desktop at the Zone root (the reflow page is a frame of `reflow.<zone>` that carries a session
 *   cookie: it only loads from the Zone's own site, not from a dev server);
 * - an Agent of the owner (`BUCKYOS_TEST_REFLOW_AGENT`, its AgentId) that runs in a container on this machine with
 *   the browser of ext-core 1.3.0 or later. The spec stops that browser once (`browser.clear_profile`: its sign-ins
 *   are gone) and opens tabs in it;
 * - the container reaching this machine as `BUCKYOS_TEST_REFLOW_SITE_HOST` (default `host.docker.internal`): the
 *   spec serves its own "old news site" on 0.0.0.0:`BUCKYOS_TEST_REFLOW_SITE_PORT` (default 18743);
 * - optionally a second user (`BUCKYOS_TEST_SHARE_USER`, default `bob`); without one that part is skipped.
 *
 * The spec owns the site of that host in the owner's reflow library (`host-docker-internal`), the site
 * `reflow-dv-example` and the folder `/home/<owner>/reflow-dv` of the Zone: all three are removed at the start and
 * at the end.
 *
 * What the App would write to the Agent (a fallback page, a failed refresh, a changed preference) is held back and
 * only looked at, so no model is woken. `BUCKYOS_TEST_REFLOW_ASK_AGENT=1` adds the one test that lets a request
 * through and waits up to ten minutes for the Agent's fallback page; it costs tokens.
 */
import { createHash } from 'node:crypto'
import { createServer, type Server } from 'node:http'
import { crc32, deflateSync } from 'node:zlib'
import { devices, expect, test, type APIRequestContext, type Browser, type BrowserContext, type Page } from '@playwright/test'

type JsonRecord = Record<string, unknown>

const zone = process.env.BUCKYOS_TEST_ZONE_HOST || 'test.buckyos.io'
const origin = `https://${zone}`
const reflowHost = `reflow.${zone}`
const reflowOrigin = `https://${reflowHost}`
const owner = process.env.BUCKYOS_TEST_ADMIN_USER || 'devtest'
const ownerPassword = process.env.BUCKYOS_TEST_ADMIN_PASSWORD || 'bucky2025'
const peer = process.env.BUCKYOS_TEST_SHARE_USER || 'bob'
const peerPassword = process.env.BUCKYOS_TEST_USER_PASSWORD || 'buckyos2026'
const agentId = process.env.BUCKYOS_TEST_REFLOW_AGENT || `reflowy.${zone}`
const siteHost = process.env.BUCKYOS_TEST_REFLOW_SITE_HOST || 'host.docker.internal'
const sitePort = Number(process.env.BUCKYOS_TEST_REFLOW_SITE_PORT || 18743)
const askAgent = process.env.BUCKYOS_TEST_REFLOW_ASK_AGENT === '1'
const siteOrigin = `http://${siteHost}:${sitePort}`
const SITE = siteHost.replaceAll('.', '-')
const BARE_SITE = 'reflow-dv-example'
const BARE_URL = 'http://reflow-dv.example/'
const zoneFolder = 'reflow-dv'
const story = (n: number) => `${siteOrigin}/news/${n}`
const bucketOf = (user: string) => `user/control-panel/${user}/reflow`

test.describe.configure({ mode: 'serial' })

// ── The old news site ──

const stories = [
  { title: '老城区图书馆周末延长开放 <b>到晚九点</b>', date: '2026-11-03', first: ['市图书馆宣布，自本周起周末开放时间延长到晚上九点。', '延长开放是应读者 & 学生的要求。'], second: ['第二页：新增的自习座位共 120 个。'] },
  { title: '河滨步道二期本月完工', date: '2026-11-08', first: ['河滨步道二期工程进入收尾阶段。'], second: ['第二页：沿线新增路灯 240 盏。'] },
  { title: '社区菜市场改造后重新开张', date: '2026-11-12', first: ['改造历时四个月的社区菜市场今天重新开张。'], second: ['第二页：市场二层新设便民服务点。'] },
]
/** `redesigned`: the title is no longer an `h1`, so the package's refresh fails. `delay`: story pages answer late. */
const site = { redesigned: false, delay: 0, hits: 0 }
const escapeHtml = (text: string) => text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')

function png(width: number, height: number): Buffer {
  const chunk = (type: string, data: Buffer) => {
    const body = Buffer.concat([Buffer.from(type), data])
    const out = Buffer.alloc(body.length + 8)
    out.writeUInt32BE(data.length, 0)
    body.copy(out, 4)
    out.writeUInt32BE(crc32(body), body.length + 4)
    return out
  }
  const head = Buffer.alloc(13)
  head.writeUInt32BE(width, 0)
  head.writeUInt32BE(height, 4)
  head[8] = 8
  head[9] = 2
  const row = Buffer.alloc(1 + width * 3)
  for (let x = 0; x < width; x++) row.set([30, 110 + Math.floor(x * 100 / width), 184], 1 + x * 3)
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', head), chunk('IDAT', deflateSync(Buffer.concat(Array.from({ length: height }, () => row)))), chunk('IEND', Buffer.alloc(0))])
}
const photo = png(1600, 900)

function frame(title: string, body: string): string {
  return `<!doctype html><html><head><meta charset="utf-8"><title>${escapeHtml(title)} - 晨报网</title></head><body>
<table width="980"><tr><td colspan="2"><font size="6">晨报网</font> 首页 | 国内 | 国际 | 论坛</td></tr>
<tr><td width="220"><p>广告位招租</p><ol><li>天气预报</li><li>彩票开奖</li></ol></td><td>${body}</td></tr></table></body></html>`
}

function storyPage(index: number): string {
  const item = stories[index]
  const heading = site.redesigned ? `<h2 class="headline">${escapeHtml(item.title)}</h2>` : `<h1>${escapeHtml(item.title)}</h1>`
  const next = stories[index + 1] ? `<a class="next" href="/news/${index + 2}">下一篇：${escapeHtml(stories[index + 1].title)}</a>` : ''
  return frame(item.title, `${heading}<p class="date">${item.date}</p><p>第 <span id="hits">${site.hits}</span> 次访问</p>
<div id="content"><img src="/photo.png" width="400"><div id="text">${item.first.map((text) => `<p>${escapeHtml(text)}</p>`).join('')}</div></div>
<p>第 <span id="pageno">1</span> 页 <button id="next-page" type="button">下一页</button></p><p><a href="/">返回列表</a> ${next}</p>
<script>
document.getElementById('next-page').addEventListener('click', () => {
  document.getElementById('text').replaceChildren(...${JSON.stringify(item.second)}.map((text) => Object.assign(document.createElement('p'), { textContent: text })));
  document.getElementById('pageno').textContent = '2';
  document.getElementById('next-page').remove();
});
</script>`)
}

function startSite(): Promise<Server> {
  const server = createServer((request, response) => {
    const path = new URL(request.url ?? '/', 'http://site').pathname
    const index = /^\/news\/([1-3])$/.exec(path)
    const html = (body: string) => { response.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8', 'Cache-Control': 'no-store' }); response.end(body) }
    if (path === '/photo.png') {
      response.writeHead(200, { 'Content-Type': 'image/png' })
      response.end(photo)
    } else if (path === '/') {
      html(frame('本地新闻', `<h1>本地新闻</h1><ul>${stories.map((item, i) => `<li><a href="/news/${i + 1}">${escapeHtml(item.title)}</a> <span class="date">${item.date}</span></li>`).join('')}</ul>`))
    } else if (index) {
      site.hits += 1
      setTimeout(() => html(storyPage(Number(index[1]) - 1)), site.delay)
    } else {
      response.writeHead(404)
      response.end('not found')
    }
  })
  return new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(sitePort, '0.0.0.0', () => resolve(server))
  })
}

// ── The package a user wrote for it (references/reflow.md of the `browser` skill) ──

const manifest = (preference: string) => JSON.stringify({
  schema: 'buckyos.reflow/1',
  site: SITE,
  match: [{ host: siteHost, path: '/news/*' }],
  preference,
  refresh: { script: 'refresh.mjs', timeout_s: 15 },
}, null, 2)

const REFRESH = `export default async ({ page, url, save }) => {
  await page.locator('h1').first().waitFor();
  const data = await page.evaluate(() => {
    const text = (selector) => document.querySelector(selector)?.textContent.trim() ?? '';
    return {
      title: text('h1'),
      date: text('.date'),
      hits: text('#hits'),
      pageno: text('#pageno'),
      paragraphs: [...document.querySelectorAll('#text p')].map((p) => p.innerText.trim()).filter(Boolean),
      image: document.querySelector('#content img')?.src ?? null,
      links: [...document.querySelectorAll('a.next')].map((a) => ({ text: a.textContent.trim(), url: a.href })),
      more: document.querySelector('#next-page') ? { action: { selector: '#next-page' } } : null,
    };
  });
  if (data.image) data.image = await save.image(data.image);
  data.env = Object.keys(process.env).sort();
  return data;
};
`

const INDEX = `<!doctype html>
<html>
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Reflow</title>
<style>
  body { margin: 0 auto; max-width: 40rem; padding: 1rem; font: 17px/1.7 system-ui, sans-serif; }
  img { max-width: 100%; }
  button { font: inherit; }
</style>
</head>
<body>
<main id="main"></main>
<script>
const query = new URLSearchParams(location.search);
const main = document.getElementById('main');
const tell = (message) => parent.postMessage(message, '*');
let updates = 0;

function add(tag, text, into = main) {
  const node = document.createElement(tag);
  if (text) node.textContent = text;
  into.append(node);
  return node;
}

async function result() {
  const file = 'results/' + query.get('page') + '.json';
  let response = await fetch(file, { cache: 'no-store' });
  if (response.status === 401) {
    document.body.dataset.signedInAgain = 'true';
    await fetch('/sso_refresh', { method: 'POST' });
    response = await fetch(file, { cache: 'no-store' });
  }
  if (!response.ok) throw new Error(file + ': HTTP ' + response.status);
  return response.json();
}

async function draw() {
  const { data } = await result();
  main.replaceChildren();
  add('h1', data.title).id = 'title';
  add('p', data.date).id = 'date';
  add('p', '第 ' + data.hits + ' 次访问，第 ' + data.pageno + ' 页').id = 'stamp';
  if (data.image) add('img').src = data.image;
  for (const paragraph of data.paragraphs) add('p', paragraph).className = 'para';
  if (data.more) {
    const more = add('button', '下一页');
    more.id = 'more';
    more.onclick = () => tell({ type: 'reflow.action', selector: data.more.action.selector });
  }
  for (const link of data.links) {
    if (!/^https?:/.test(link.url)) continue;
    const a = add('a', link.text, add('p'));
    a.className = 'next';
    a.href = link.url;
    a.onclick = (event) => {
      event.preventDefault();
      tell({ type: 'reflow.open', url: link.url });
    };
  }
  const again = add('button', '重新制作', add('p'));
  again.id = 'again';
  again.onclick = () => tell({ type: 'reflow.refresh' });
  document.body.dataset.updates = String(updates);
  tell({ type: 'reflow.ready' });
}

addEventListener('message', (event) => {
  if (event.source === parent && event.data?.type === 'reflow.updated') { updates += 1; draw(); }
});
draw().catch((error) => { add('p', 'Cannot show this page: ' + error.message).id = 'error'; });
</script>
</body>
</html>
`

const packageFiles = (preference: string): Record<string, string> => ({ 'manifest.json': manifest(preference), 'refresh.mjs': REFRESH, 'index.html': INDEX })

/** A zip archive of these files, stored. */
function zip(files: Record<string, string>): Buffer {
  const parts: Buffer[] = []
  const directory: Buffer[] = []
  let offset = 0
  for (const [name, content] of Object.entries(files)) {
    const data = Buffer.from(content)
    const file = Buffer.from(name)
    const local = Buffer.alloc(30)
    local.writeUInt32LE(0x04034b50, 0)
    local.writeUInt16LE(20, 4)
    local.writeUInt32LE(crc32(data), 14)
    local.writeUInt32LE(data.length, 18)
    local.writeUInt32LE(data.length, 22)
    local.writeUInt16LE(file.length, 26)
    const entry = Buffer.alloc(46)
    entry.writeUInt32LE(0x02014b50, 0)
    entry.writeUInt16LE(20, 4)
    entry.writeUInt16LE(20, 6)
    entry.writeUInt32LE(crc32(data), 16)
    entry.writeUInt32LE(data.length, 20)
    entry.writeUInt32LE(data.length, 24)
    entry.writeUInt16LE(file.length, 28)
    entry.writeUInt32LE(offset, 42)
    parts.push(local, file, data)
    directory.push(entry, file)
    offset += 30 + file.length + data.length
  }
  const all = Buffer.concat(directory)
  const end = Buffer.alloc(22)
  end.writeUInt32LE(0x06054b50, 0)
  end.writeUInt16LE(directory.length / 2, 8)
  end.writeUInt16LE(directory.length / 2, 10)
  end.writeUInt32LE(all.length, 12)
  end.writeUInt32LE(offset, 16)
  return Buffer.concat([...parts, all, end])
}

// ── The Zone ──

let api: APIRequestContext
let server: Server
let agentDid = ''
let agentName = ''
const tokens = new Map<string, { token: string; at: number }>()

function hashPassword(username: string, password: string, nonce: number): string {
  const original = createHash('sha256').update(`${password}${username}.buckyos`, 'utf8').digest('base64')
  return createHash('sha256').update(`${original}${nonce}`, 'utf8').digest('base64')
}

async function call<T = JsonRecord>(service: string, method: string, params: JsonRecord, token?: string): Promise<T> {
  const seq = Date.now()
  const response = await api.post(`${origin}/kapi/${service}`, { data: { method, params, sys: token ? [seq, token] : [seq] } })
  const body = await response.json() as { result?: T; error?: string }
  if (body.error !== undefined && body.error !== null) throw new Error(`${method}: ${body.error}`)
  return body.result as T
}

/** A session token of `user`; sessions last 15 minutes, so one is used for 10. */
async function tokenOf(user = owner, password = ownerPassword): Promise<string> {
  const known = tokens.get(user)
  if (known && Date.now() - known.at < 10 * 60_000) return known.token
  const nonce = Date.now()
  const login = await call<{ session_token: string }>('control-panel', 'auth.login', {
    username: user, password: hashPassword(user, password, nonce), target: { kind: 'system', service_id: 'control-panel' }, login_nonce: nonce,
  })
  tokens.set(user, { token: login.session_token, at: Date.now() })
  return login.session_token
}

const agent = async <T = JsonRecord>(method: string, params: JsonRecord = {}) => call<T>(agentId, method, params, await tokenOf())

interface ReflowState {
  url: string; site: string; page: string; view: string | null
  package: { preference: string } | null
  result: { made_at: number; made_by: string; source_url: string } | null
  fallback: { made_at: number } | null
  refresh: { status: string; error: string | null }
}
const reflowState = (url: string) => agent<ReflowState>('browser.reflow_state', { url })

const keyUrl = (user: string, key: string) => `${origin}/bucket/v1/${bucketOf(user)}/${key.split('/').map(encodeURIComponent).join('/')}`

async function putKey(key: string, body: string, contentType: string) {
  const response = await api.put(keyUrl(owner, key), { headers: { Authorization: `Bearer ${await tokenOf()}`, 'Content-Type': contentType }, data: body })
  expect(response.status(), `PUT ${key}`).toBeLessThan(300)
}

/** The text of a key of the owner's library, or null when it is not there. */
async function readKey(key: string): Promise<string | null> {
  const response = await api.get(keyUrl(owner, key), { headers: { Authorization: `Bearer ${await tokenOf()}` } })
  if (response.status() === 404) return null
  expect(response.status(), `GET ${key}`).toBe(200)
  return response.text()
}

async function keysOf(prefix: string): Promise<string[]> {
  const page = await call<{ items: Array<{ key: string }> }>('nfs-server', 'bucket.list_objects', { bucket_id: bucketOf(owner), prefix }, await tokenOf())
  return page.items.map((item) => item.key)
}

async function removeSite(name: string) {
  for (const key of await keysOf(`${name}/`)) await call('nfs-server', 'bucket.delete', { bucket_id: bucketOf(owner), key }, await tokenOf())
}

async function nfs(method: string, body: JsonRecord): Promise<JsonRecord> {
  const response = await api.post(`${origin}/nfs/v1/${method}`, { headers: { Authorization: `Bearer ${await tokenOf()}` }, data: body })
  const parsed = await response.json() as { ok: boolean; result?: JsonRecord; error?: JsonRecord }
  if (!parsed.ok) throw new Error(`${method}: ${JSON.stringify(parsed.error)}`)
  return parsed.result ?? {}
}

/** Puts `files` into `/home/<owner>/<dir>` of the Zone. */
async function putZoneFolder(dir: string, files: Record<string, string>) {
  const session = (await nfs('hello', { args: {} })).session
  const path = `/home/${owner}/${dir}`
  let seq = 0
  await nfs('mkdir', { session, seq: ++seq, args: {}, at: { path } })
  const folder = await nfs('stat', { session, args: {}, at: { path } })
  for (const [name, content] of Object.entries(files)) {
    const bytes = Buffer.from(content)
    const open = await nfs('open_write', { session, seq: ++seq, args: { parent_ref: folder.ref, name, size: bytes.length } })
    const patch = await api.fetch(`${origin}/nfs/v1/uploads/${String(open.fb_handle)}`, {
      method: 'PATCH',
      headers: { Authorization: `Bearer ${await tokenOf()}`, 'Upload-Offset': '0', 'Tus-Resumable': '1.0.0', 'Content-Type': 'application/offset+octet-stream' },
      data: bytes,
    })
    expect(patch.status(), `upload ${name}`).toBe(204)
    await nfs('commit_file', { session, seq: ++seq, args: { name, fb_handle: open.fb_handle, lease_id: (open.lease as JsonRecord).lease_id }, at: { ref: folder.ref } })
  }
}

async function removeZoneFolder() {
  const session = (await nfs('hello', { args: {} })).session
  await nfs('delete', { session, seq: 1, args: { name: zoneFolder, recursive: true }, at: { path: `/home/${owner}` } }).catch(() => undefined)
}

test.beforeAll(async ({ playwright }) => {
  api = await playwright.request.newContext({ ignoreHTTPSErrors: true })
  server = await startSite()
  const { agents } = await call<{ agents: Array<{ agent_id: string; agent_did: string; name: string; display_name?: string | null; profile?: { display_name?: string | null } }> }>('control-panel', 'agent.list', {}, await tokenOf())
  const found = agents.find((item) => item.agent_id === agentId)
  expect(found, `${agentId} is an Agent of ${owner}`).toBeTruthy()
  agentDid = found!.agent_did
  // The App names the maker as the Agent setup does (`agentDisplayName`): its display name, else its name.
  agentName = found!.profile?.display_name?.trim() || found!.display_name?.trim() || found!.name
  await removeSite(SITE)
  await removeSite(BARE_SITE)
  await removeZoneFolder()
})

test.afterAll(async () => {
  await removeSite(SITE).catch(() => undefined)
  await removeSite(BARE_SITE).catch(() => undefined)
  await removeZoneFolder()
  await new Promise((resolve) => server.close(resolve))
  await api.dispose()
})

// ── The App ──

interface App {
  context: BrowserContext
  page: Page
  /** What the App wrote to the Agent, in order. */
  sent: string[]
  /** Lets the messages through from now on. */
  letThrough(): void
}

/** Signs the owner in and opens the Agent browser on the Agent in reflow mode. */
async function openReflow(browser: Browser, options: Parameters<Browser['newContext']>[0] = { viewport: { width: 1280, height: 900 } }): Promise<App> {
  const context = await browser.newContext({ ignoreHTTPSErrors: true, ...options })
  const page = await context.newPage()
  const sent: string[] = []
  let held = true
  await page.route(/\/kapi\/msg-center\/?$/, async (route) => {
    const body = route.request().postData() ?? ''
    if (body.includes('"msg.post_send"')) {
      sent.push(String(((JSON.parse(body) as { params: { msg: { content: { content: unknown } } } }).params.msg.content.content)))
      if (held) return route.abort()
    }
    return route.continue()
  })
  await page.addInitScript(() => {
    window.addEventListener('message', (event) => {
      const type = (event.data as { type?: unknown } | null)?.type
      if (typeof type === 'string' && type.startsWith('reflow.')) ((window as unknown as { __reflow: unknown[] }).__reflow ??= []).push({ origin: event.origin, ...(event.data as object) })
    })
  })
  await page.goto(`${origin}/`)
  await page.getByLabel('Username', { exact: true }).fill(owner)
  await page.getByLabel('Password', { exact: true }).fill(ownerPassword)
  const signIn = page.getByRole('button', { name: 'Sign In', exact: true })
  await (options.hasTouch ? signIn.tap() : signIn.click())
  await expect(options.hasTouch ? page.getByTestId('mobile-taskbar') : page.getByRole('button', { name: 'BuckyOS' }).first()).toBeAttached({ timeout: 30_000 })
  await page.goto(`${origin}/?agent_browser=${encodeURIComponent(agentId)}&mode=reflow`)
  await expect(panel(page)).toBeVisible({ timeout: 30_000 })
  // Reflow mode starts on the page the Agent's browser shows, whatever an earlier run left there: let that settle
  // (a request for it is held back), so that each test sees only what its own steps bring about.
  await expect(panel(page)).toHaveAttribute('data-phase', /^(blank|view|unmade|unsupported|failed)$/, { timeout: 60_000 })
  sent.length = 0
  return { context, page, sent, letThrough: () => { held = false } }
}

const panel = (page: Page) => page.getByTestId('ab-reflow-panel')
const inner = (page: Page) => page.frameLocator('[data-testid="ab-reflow-frame"]')
const madeAt = async (page: Page) => Number(await page.getByTestId('ab-reflow-made').getAttribute('data-made-at'))
const pageMessages = (page: Page) => page.evaluate(() => (window as unknown as { __reflow?: Array<{ origin: string; type: string; url?: string; selector?: string }> }).__reflow ?? [])

async function go(page: Page, address: string) {
  const input = page.getByTestId('ab-address-input')
  await input.click()
  await input.fill(address)
  await input.press('Enter')
}

/** The address shown as a page, made with the package. */
async function shown(page: Page, address: string, title: string) {
  await expect(panel(page)).toHaveAttribute('data-url', address)
  await expect(panel(page)).toHaveAttribute('data-phase', 'view', { timeout: 60_000 })
  await expect(inner(page).locator('#title')).toContainText(title, { timeout: 30_000 })
}

async function openLibrary(page: Page) {
  await page.getByTestId('ab-menu-button').click()
  await page.getByTestId('ab-reflow-library-open').click()
  await expect(page.getByTestId('ab-reflow-library')).toBeVisible()
}

/** Closes the library panel and the menu it was opened from. */
async function closeLibrary(page: Page) {
  await page.getByTestId('ab-reflow-library-close').click()
  await page.getByTestId('ab-menu-close').click()
  await expect(page.getByTestId('ab-menu')).toHaveCount(0)
}

async function importZip(page: Page, name: string, files: Record<string, string>) {
  await page.getByTestId('ab-reflow-import-zip').click()
  const chooser = page.waitForEvent('filechooser')
  await page.getByTestId('select-dialog').getByTestId('select-local').click()
  await (await chooser).setFiles({ name, mimeType: 'application/zip', buffer: zip(files) })
  await expect(page.getByTestId('ab-reflow-report')).toBeVisible({ timeout: 30_000 })
}

async function setPreference(page: Page, text: string): Promise<string> {
  await page.getByTestId('ab-reflow-preference').click()
  const input = page.getByTestId('ab-reflow-preference-input')
  await expect(input).toBeVisible()
  const before = await input.inputValue()
  await input.fill(text)
  await page.getByTestId('ab-reflow-preference-save').click()
  await expect(page.getByTestId('ab-reflow-preference-sheet')).toHaveCount(0)
  return before
}

// ── §5.4: the host of the pages ──

test('reflow.<zone> serves the signed-in user their own library, and its session opens nothing else', async ({ playwright }) => {
  const key = `${SITE}/dv-hello.html`
  await putKey(key, '<!doctype html><meta charset="utf-8"><h1>owner</h1>', 'text/html; charset=utf-8')
  const landing = `${reflowOrigin}/${key}`
  const signIn = async (user: string, password: string) => {
    const visitor = await playwright.request.newContext({ ignoreHTTPSErrors: true })
    const launch = await call<{ status: string; callback_url: string }>('control-panel', 'auth.prepare_app_launch', { service_id: 'nfs-server', redirect_url: landing }, await tokenOf(user, password))
    expect(launch.status).toBe('ready')
    expect(new URL(launch.callback_url).origin).toBe(reflowOrigin)
    const page = await visitor.get(launch.callback_url)
    const cookies = (await visitor.storageState()).cookies.filter((cookie) => cookie.domain === reflowHost)
    return { visitor, page, session: cookies.find((cookie) => cookie.name === 'buckyos_session_token')?.value ?? '', callback: launch.callback_url }
  }
  const mine = await signIn(owner, ownerPassword)
  try {
    expect(mine.page.url()).toBe(landing)
    expect(mine.page.status()).toBe(200)
    expect(await mine.page.text()).toContain('<h1>owner</h1>')
    const headers = mine.page.headers()
    expect(headers['content-type']).toBe('text/html; charset=utf-8')
    expect(headers['content-disposition']).toBeUndefined()
    expect(headers['content-security-policy']).toBeUndefined()
    expect(headers['x-content-type-options']).toBe('nosniff')
    const claims = JSON.parse(Buffer.from(mine.session.split('.')[1], 'base64url').toString('utf8')) as JsonRecord
    expect(claims).toMatchObject({ sub: owner, appid: 'nfs-server', target_kind: 'system', principal_kind: 'user' })

    // The callback is good once; the host answers GET and HEAD only, and nothing but the library.
    expect((await api.get(mine.callback, { maxRedirects: 0 })).status()).toBeGreaterThanOrEqual(400)
    expect((await mine.visitor.post(landing)).status()).toBe(405)
    expect((await mine.visitor.get(`${reflowOrigin}/${SITE}/missing.html`)).status()).toBe(404)
    expect((await mine.visitor.get(`${reflowOrigin}/${SITE}/%2e%2e/x`)).status()).toBe(404)

    // Not signed in on this host: no cookie, or the Desktop's own session.
    const stranger = await playwright.request.newContext({ ignoreHTTPSErrors: true })
    const refused = await stranger.get(landing, { maxRedirects: 0 })
    expect(refused.status()).toBe(401)
    expect(refused.headers()['content-type']).toContain('text/plain')
    expect((await stranger.get(landing, { headers: { Authorization: `Bearer ${await tokenOf()}` } })).status()).toBe(401)

    // The page session is no sign-in on the file plane or the bucket.
    const bearer = { Authorization: `Bearer ${mine.session}` }
    expect((await stranger.post(`${origin}/nfs/v1/hello`, { headers: bearer, data: { args: {} } })).status()).toBe(403)
    expect((await stranger.get(keyUrl(owner, key), { headers: bearer })).status()).toBe(403)
    expect((await stranger.put(keyUrl(owner, `${SITE}/x.txt`), { headers: bearer, data: 'x' })).status()).toBe(403)
    const viaCookie = await stranger.post(`${origin}/nfs/v1/hello`, { headers: { Cookie: `buckyos_session_token=${mine.session}` }, data: { args: {} } })
    expect(((await viaCookie.json()) as { result?: { identity?: { role?: string } } }).result?.identity?.role).toBe('anonymous')
    await stranger.dispose()

    // Another user at the same address reads their own library.
    const other = await signIn(peer, peerPassword).catch(() => null)
    test.info().annotations.push({ type: 'second user', description: other ? `${peer}: checked` : `${peer} cannot sign in: not checked` })
    if (other) {
      expect(other.page.url()).toBe(landing)
      expect(other.page.status()).toBe(404)
      expect((await api.get(keyUrl(owner, key), { headers: { Authorization: `Bearer ${await tokenOf(peer, peerPassword)}` } })).status()).toBe(403)
      await other.visitor.dispose()
    }
  } finally {
    await mine.visitor.dispose()
    await call('nfs-server', 'bucket.delete', { bucket_id: bucketOf(owner), key }, await tokenOf())
  }
})

// ── §8.4 "没有包的站由 LLM 兜底重排" (opt-in: wakes the model) ──

test('a site without a package: the Agent is asked, makes a fallback page, and the App shows it', async ({ browser }) => {
  test.skip(!askAgent, 'wakes the real model: set BUCKYOS_TEST_REFLOW_ASK_AGENT=1')
  test.setTimeout(12 * 60_000)
  const address = `${siteOrigin}/`
  const app = await openReflow(browser)
  try {
    app.letThrough()
    await go(app.page, address)
    await expect(app.page.getByTestId('ab-reflow-making')).toHaveAttribute('data-by', 'agent', { timeout: 30_000 })
    await expect.poll(() => app.sent.length).toBe(1)
    expect(app.sent[0]).toContain(address)
    await expect(app.page.getByTestId('ab-reflow-made')).toHaveAttribute('data-kind', 'fallback', { timeout: 10 * 60_000 })
    await expect(app.page.getByTestId('ab-reflow-frame')).toHaveAttribute('data-drawn', 'true', { timeout: 30_000 })
    const state = await reflowState(address)
    expect(state.view).toBe(`${SITE}/fallback/${state.page}.html`)
    await expect(inner(app.page).locator('body')).toContainText(stories[1].title)
    expect((await keysOf(`${SITE}/fallback/`)).sort()).toEqual([`${SITE}/fallback/${state.page}.html`, `${SITE}/fallback/${state.page}.json`])
  } finally {
    await app.context.close()
  }
})

// ── §8.4 "用户自己写的一个包导入后，打开匹配的网址时生效" ──

test('a package the user wrote is imported as a zip and makes the pages it covers', async ({ browser }) => {
  const app = await openReflow(browser)
  const { page } = app
  try {
    await openLibrary(page)
    // The archive holds the package as one folder, beside what an archive tool adds.
    const files = packageFiles('只看标题、日期和正文，手机单栏')
    await importZip(page, 'package.zip', { ...Object.fromEntries(Object.entries(files).map(([name, text]) => [`${SITE}/${name}`, text])), [`${SITE}/.git/config`]: 'x', [`__MACOSX/${SITE}/._index.html`]: 'x' })
    await expect(page.getByTestId('ab-reflow-report')).toHaveAttribute('data-kind', 'imported')
    await expect(page.getByTestId(`ab-reflow-site-${SITE}`)).toHaveAttribute('data-package', 'true')
    expect((await keysOf(`${SITE}/`)).sort()).toEqual([`${SITE}/index.html`, `${SITE}/manifest.json`, `${SITE}/refresh.mjs`])
    const stored = JSON.parse((await readKey(`${SITE}/manifest.json`))!) as JsonRecord
    expect(stored).toMatchObject({ site: SITE, author: { kind: 'user', id: owner }, preference: '只看标题、日期和正文，手机单栏' })
    expect(stored.updated_at).toBeGreaterThan(Date.now() - 60_000)
    await closeLibrary(page)

    // The package covers the story and nothing is made yet: the Agent's browser makes it, without a model.
    site.delay = 3000
    await go(page, story(1))
    await expect(page.getByTestId('ab-reflow-making')).toHaveAttribute('data-by', 'refresh', { timeout: 20_000 })
    // The tab the refresh runs in belongs to no thing of the user.
    const groups = new Set<string>()
    await expect.poll(async () => {
      for (const group of (await agent<{ groups: Array<{ group_id: string }> }>('browser.status')).groups) groups.add(group.group_id)
      return (await reflowState(story(1))).refresh.status
    }, { timeout: 60_000, intervals: [300] }).toBe('idle')
    expect([...groups].filter((id) => id.startsWith('_'))).toEqual([])
    await shown(page, story(1), '图书馆')
    expect(app.sent).toEqual([])

    // The address bar keeps the original address; beside it, when the page was made and by whom.
    await expect(page.getByTestId('ab-address-input')).toHaveValue(story(1))
    await expect(page.getByTestId('ab-reflow-made')).toHaveAttribute('data-kind', 'package')
    await expect(page.getByTestId('ab-reflow-maker')).toContainText(agentName)
    expect(Date.now() - await madeAt(page)).toBeLessThan(120_000)
    await expect(page.getByTestId('ab-reflow-frame')).toHaveAttribute('src', new RegExp(`^${reflowOrigin}/sso_callback\\?nonce=`))
    expect(page.frames().some((frame) => frame.url().startsWith(`${reflowOrigin}/${SITE}/?url=`))).toBe(true)
    expect(await pageMessages(page)).toContainEqual({ origin: reflowOrigin, type: 'reflow.ready' })

    // The site's markup is text; the picture travels inside the result; the script saw no token.
    await expect(inner(page).locator('#title')).toHaveText(stories[0].title)
    await expect(inner(page).locator('#title b')).toHaveCount(0)
    const state = await reflowState(story(1))
    const result = JSON.parse((await readKey(`${SITE}/results/${state.page}.json`))!) as { made_by: string; source_url: string; data: { image: string; env: string[] } }
    expect(result).toMatchObject({ made_by: agentDid, source_url: story(1) })
    expect(result.data.image).toMatch(/^data:image\/jpeg;base64,/)
    expect(result.data.env.filter((name) => /TOKEN|BUCKYOS|OPENDAN/i.test(name))).toEqual([])
  } finally {
    site.delay = 0
    await app.context.close()
  }
})

// ── §5.5: the page and the App ──

test('links, making it again and an action on the original page go through the App', async ({ browser }) => {
  const app = await openReflow(browser)
  const { page } = app
  try {
    await go(page, story(1))
    await shown(page, story(1), '图书馆')

    // "Make this page again": the time moves on and the page reads its result again, without a reload.
    const first = await madeAt(page)
    site.delay = 2000
    await page.getByTestId('ab-reflow-refresh').click()
    await expect(page.getByTestId('ab-reflow-busy')).toHaveAttribute('data-job', 'refresh')
    await expect.poll(() => madeAt(page), { timeout: 40_000 }).toBeGreaterThan(first)
    await expect(inner(page).locator('body')).toHaveAttribute('data-updates', '1')
    await expect(page.getByTestId('ab-reflow-busy')).toHaveCount(0)
    site.delay = 0

    // The page asks for the same.
    const second = await madeAt(page)
    await inner(page).locator('#again').click()
    await expect.poll(() => madeAt(page), { timeout: 40_000 }).toBeGreaterThan(second)
    await expect(inner(page).locator('body')).toHaveAttribute('data-updates', '2')

    // A control only the original page can carry out.
    await inner(page).locator('#more').click()
    await expect(inner(page).locator('#stamp')).toContainText('第 2 页', { timeout: 40_000 })
    await expect(inner(page).locator('.para').first()).toHaveText(stories[0].second[0])
    await expect(inner(page).locator('#more')).toHaveCount(0)

    // A link: the next story has nothing made yet, so it is made; then back.
    await inner(page).locator('a.next').click()
    await shown(page, story(2), '河滨步道')
    await expect(page.getByTestId('ab-address-input')).toHaveValue(story(2))
    await page.getByTestId('ab-back').click()
    await shown(page, story(1), '图书馆')

    const types = (await pageMessages(page)).filter((message) => message.origin === reflowOrigin)
    expect(types).toContainEqual({ origin: reflowOrigin, type: 'reflow.refresh' })
    expect(types).toContainEqual({ origin: reflowOrigin, type: 'reflow.action', selector: '#next-page' })
    expect(types).toContainEqual({ origin: reflowOrigin, type: 'reflow.open', url: story(2) })
    expect(app.sent).toEqual([])
    // Leave the first page of the story.
    await page.getByTestId('ab-reflow-refresh').click()
    await expect(inner(page).locator('#stamp')).toContainText('第 1 页', { timeout: 40_000 })
  } finally {
    site.delay = 0
    await app.context.close()
  }
})

// ── §8.4 "网站改版、刷新脚本失败时" (what the Agent is asked; the message is held back) ──

test('a refresh that fails keeps the page that was made and hands the page to the Agent', async ({ browser }) => {
  const app = await openReflow(browser)
  const { page } = app
  try {
    await go(page, story(1))
    await shown(page, story(1), '图书馆')
    const before = await madeAt(page)
    site.redesigned = true

    await page.getByTestId('ab-reflow-refresh').click()
    const notice = page.getByTestId('ab-reflow-notice')
    await expect(notice).toHaveAttribute('data-kind', 'refresh_failed', { timeout: 60_000 })
    await expect(notice).toContainText('reflow failed')
    await expect(inner(page).locator('#title')).toHaveText(stories[0].title)
    expect(await madeAt(page)).toBe(before)
    expect((await reflowState(story(1))).refresh).toMatchObject({ status: 'failed' })
    expect(app.sent).toHaveLength(1)
    for (const part of [story(1), 'reflow failed', SITE]) expect(app.sent[0]).toContain(part)

    // A page with nothing made yet: the refresh starts by itself, fails, and the Agent is asked for this page too.
    await go(page, story(3))
    await expect(page.getByTestId('ab-reflow-making')).toHaveAttribute('data-by', 'refresh', { timeout: 20_000 })
    await expect.poll(() => app.sent.length, { timeout: 60_000 }).toBe(2)
    for (const part of [story(3), 'reflow failed', SITE]) expect(app.sent[1]).toContain(part)
    await expect(page.getByTestId('ab-reflow-failure')).toContainText('reflow failed')
    // Held back here, so the App says the Agent could not be asked.
    await expect(page.getByTestId('ab-reflow-unmade')).toHaveAttribute('data-reason', 'agent_unreachable')

    // The site is back: the page can be made after all.
    site.redesigned = false
    await page.getByTestId('ab-reflow-retry').click()
    await shown(page, story(3), '菜市场')
  } finally {
    site.redesigned = false
    await app.context.close()
  }
})

// ── §5.2: the preference ──

test('the preference of a site is kept in its package, or beside a site that has none', async ({ browser }) => {
  const app = await openReflow(browser)
  const { page } = app
  try {
    await go(page, story(1))
    await shown(page, story(1), '图书馆')
    const before = JSON.parse((await readKey(`${SITE}/manifest.json`))!) as JsonRecord
    expect(await setPreference(page, '只看标题和正文，字号大一些')).toBe('只看标题、日期和正文，手机单栏')
    await expect.poll(() => app.sent.length).toBe(1)
    for (const part of [siteHost, '只看标题和正文，字号大一些', story(1)]) expect(app.sent[0]).toContain(part)
    expect(JSON.parse((await readKey(`${SITE}/manifest.json`))!)).toEqual({ ...before, preference: '只看标题和正文，字号大一些' })
    expect(await readKey(`${SITE}/preference.json`)).toBeNull()

    // A site the library has nothing for: the App asks for a page (held back), the preference goes beside the site.
    await go(page, BARE_URL)
    await expect(page.getByTestId('ab-reflow-unmade')).toHaveAttribute('data-reason', 'agent_unreachable', { timeout: 30_000 })
    expect(app.sent[1]).toContain(BARE_URL)
    expect(await setPreference(page, '只看正文')).toBe('')
    await expect.poll(() => readKey(`${BARE_SITE}/preference.json`)).not.toBeNull()
    expect(JSON.parse((await readKey(`${BARE_SITE}/preference.json`))!)).toMatchObject({ schema: 'buckyos.reflow.preference/1', preference: '只看正文' })
    await expect.poll(() => app.sent.length).toBe(3)
    expect(app.sent[2]).toContain('只看正文')
    await openLibrary(page)
    await expect(page.getByTestId(`ab-reflow-site-${BARE_SITE}`)).toHaveAttribute('data-package', 'false')
    await expect(page.getByTestId(`ab-reflow-site-${BARE_SITE}`)).toContainText('只看正文')
    await closeLibrary(page)

    // Cleared: the file is gone and nobody is told.
    expect(await setPreference(page, '')).toBe('只看正文')
    await expect.poll(() => readKey(`${BARE_SITE}/preference.json`)).toBeNull()
    expect(app.sent).toHaveLength(3)
  } finally {
    await app.context.close()
  }
})

// ── §5.6 "原始页面" ──

test('the original page opens in the Agent’s browser', async ({ browser }) => {
  const app = await openReflow(browser)
  const { page } = app
  try {
    await go(page, story(2))
    await shown(page, story(2), '河滨步道')
    await page.getByTestId('ab-reflow-source').click()
    await expect(page.getByTestId('ab-screen')).toHaveAttribute('data-status', 'open', { timeout: 60_000 })
    await expect.poll(async () => {
      const status = await agent<{ groups: Array<{ group_id: string; tabs: Array<{ url: string; active: boolean }> }> }>('browser.status')
      return status.groups.find((group) => group.group_id === 'default')?.tabs.find((tab) => tab.active)?.url
    }, { timeout: 40_000 }).toBe(story(2))
    await expect(page.getByTestId('ab-address-input')).toHaveValue(story(2))
  } finally {
    await app.context.close()
  }
})

// ── §8.4 "常驻浏览器没有运行时，已有结果的重排页照样能打开"; §5.4 the session of the pages' host ──

test('a page that was made opens without the browser, and after the sign-in on the pages’ host ran out', async ({ browser }) => {
  await agent('browser.clear_profile')
  await expect.poll(async () => (await agent<{ running: boolean }>('browser.status')).running).toBe(false)
  const app = await openReflow(browser)
  const { page, context } = app
  try {
    await go(page, story(1))
    await shown(page, story(1), '图书馆')
    await go(page, story(2))
    await shown(page, story(2), '河滨步道')
    await page.getByTestId('ab-back').click()
    await shown(page, story(1), '图书馆')
    expect((await agent<{ running: boolean }>('browser.status')).running).toBe(false)

    // Signed out of the pages' host altogether (cleared site data, or seven days): a link signs in again.
    await context.clearCookies({ domain: reflowHost })
    await inner(page).locator('a.next').click()
    await shown(page, story(2), '河滨步道')

    // Only the session ran out (15 minutes): the page's own read is refused, it refreshes the session and reads again.
    await context.clearCookies({ domain: reflowHost, name: 'buckyos_session_token' })
    const before = await madeAt(page)
    await page.getByTestId('ab-reflow-refresh').click()
    await expect.poll(() => madeAt(page), { timeout: 60_000 }).toBeGreaterThan(before)
    await expect(inner(page).locator('body')).toHaveAttribute('data-updates', '1')
    await expect(inner(page).locator('body')).toHaveAttribute('data-signed-in-again', 'true')
    await expect(inner(page).locator('#error')).toHaveCount(0)
    await expect(inner(page).locator('#title')).toContainText('河滨步道')
  } finally {
    await app.context.close()
  }
})

// ── A phone ──

test('on a phone the page is one column and the library is at hand', async ({ browser }) => {
  const app = await openReflow(browser, { ...devices['Pixel 7'] })
  const { page } = app
  try {
    await page.getByTestId('ab-address-input').tap()
    await page.getByTestId('ab-address-input').fill(story(1))
    await page.getByTestId('ab-address-input').press('Enter')
    await shown(page, story(1), '图书馆')
    const width = page.viewportSize()!.width
    expect((await page.getByTestId('ab-reflow-frame').boundingBox())!.width).toBe(width)
    expect(await inner(page).locator('html').evaluate((html) => html.scrollWidth <= html.clientWidth)).toBe(true)
    await expect(page.getByTestId('ab-reflow-made')).toBeVisible()

    await inner(page).locator('a.next').tap()
    await shown(page, story(2), '河滨步道')
    const before = await madeAt(page)
    await page.getByTestId('ab-reflow-refresh').tap()
    await expect.poll(() => madeAt(page), { timeout: 40_000 }).toBeGreaterThan(before)
    await page.getByTestId('ab-back').tap()
    await shown(page, story(1), '图书馆')

    await page.getByTestId('ab-menu-button').tap()
    await page.getByTestId('ab-reflow-library-open').tap()
    await page.getByTestId(`ab-reflow-site-${SITE}`).tap()
    await expect(page.getByTestId('ab-reflow-manifest')).toContainText('buckyos.reflow/1')
    const sheet = (await page.getByTestId('ab-reflow-site').boundingBox())!
    expect(sheet.x).toBeGreaterThanOrEqual(0)
    expect(sheet.x + sheet.width).toBeLessThanOrEqual(width)
  } finally {
    await app.context.close()
  }
})

// ── RF7: the library ──

test('the library shows a site, refuses what is no package, imports a folder of the Zone and deletes a site', async ({ browser }) => {
  const folder = packageFiles('（来自 Zone 目录）只看标题和正文')
  await putZoneFolder(`${zoneFolder}/${SITE}`, { ...folder, '.DS_Store': 'x' })
  const app = await openReflow(browser)
  const { page } = app
  try {
    await go(page, story(1))
    await shown(page, story(1), '图书馆')
    await openLibrary(page)
    const row = page.getByTestId(`ab-reflow-site-${SITE}`)
    await expect(row).toHaveAttribute('data-package', 'true')
    await expect(row).toContainText(siteHost)
    expect(Number(await row.getAttribute('data-results'))).toBeGreaterThanOrEqual(2)

    // What is no package is refused with the reasons, and nothing is written.
    const before = await keysOf(`${SITE}/`)
    const files = packageFiles('x')
    await importZip(page, 'no-manifest.zip', { 'index.html': INDEX })
    await expect(page.getByTestId('ab-reflow-report')).toHaveAttribute('data-kind', 'invalid')
    await expect(page.getByTestId('ab-reflow-report')).toContainText('manifest.json is missing')
    await importZip(page, 'wrong.zip', { 'manifest.json': manifest('x').replace(`"${siteHost}"`, '"www.example.com"').replace('"timeout_s": 15', '"timeout_s": 6000'), 'index.html': INDEX })
    await expect(page.getByTestId('ab-reflow-report')).toHaveAttribute('data-kind', 'invalid')
    for (const problem of ['match[0].host', 'refresh.timeout_s', 'refresh.mjs is missing']) await expect(page.getByTestId('ab-reflow-report')).toContainText(problem)
    await importZip(page, 'data.zip', { ...files, 'results/0123456789abcdef.json': '{}', 'preference.json': '{}' })
    for (const problem of ['results/', 'preference.json']) await expect(page.getByTestId('ab-reflow-report')).toContainText(problem)
    await page.getByTestId('ab-reflow-import-zip').click()
    const chooser = page.waitForEvent('filechooser')
    await page.getByTestId('select-dialog').getByTestId('select-local').click()
    await (await chooser).setFiles({ name: 'notes.zip', mimeType: 'application/zip', buffer: Buffer.from('not an archive') })
    await expect(page.getByTestId('ab-reflow-report')).toContainText('not a zip archive')
    expect(await keysOf(`${SITE}/`)).toEqual(before)

    // A folder of the Zone (where an Agent that may not write the library leaves its package).
    await page.getByTestId('ab-reflow-import-folder').click()
    const dialog = page.getByTestId('select-dialog')
    await expect(dialog).toHaveAttribute('data-mode', 'folder')
    // The picker opens where it was left: go down from wherever that is.
    for (const name of [owner, zoneFolder, SITE]) {
      const row = dialog.locator(`[data-testid="select-row"][data-name="${name}"]`)
      const crumb = dialog.getByTestId('select-crumbs').getByText(name, { exact: true })
      await expect(row.or(crumb).first()).toBeVisible()
      if (await crumb.count() === 0) await row.click()
    }
    await expect(dialog.getByTestId('select-crumbs')).toContainText(SITE)
    await dialog.getByTestId('select-confirm').click()
    await expect(page.getByTestId('ab-reflow-report')).toHaveAttribute('data-kind', 'imported', { timeout: 30_000 })
    await expect(row).toContainText('（来自 Zone 目录）只看标题和正文')
    expect(JSON.parse((await readKey(`${SITE}/manifest.json`))!)).toMatchObject({ preference: '（来自 Zone 目录）只看标题和正文', author: { kind: 'user', id: owner } })
    expect((await keysOf(`${SITE}/`)).filter((key) => key.includes('.DS_Store'))).toEqual([])
    expect((await keysOf(`${SITE}/results/`)).length).toBeGreaterThanOrEqual(2)

    // A look inside, then delete: the package, what was made with it and the fallback pages go together.
    await putKey(`${SITE}/fallback/0123456789abcdef.html`, '<!doctype html><p>one-off</p>', 'text/html; charset=utf-8')
    await putKey(`${SITE}/fallback/0123456789abcdef.json`, JSON.stringify({ schema: 'buckyos.reflow.fallback/1', source_url: `${siteOrigin}/`, made_at: Date.now(), made_by: agentDid }), 'application/json')
    await row.click()
    await expect(page.getByTestId('ab-reflow-site-facts')).toContainText(owner)
    await expect(page.getByTestId('ab-reflow-manifest')).toContainText('"/news/*"')
    expect(await page.getByTestId('ab-reflow-file').evaluateAll((items) => items.map((item) => item.getAttribute('data-name')).sort())).toEqual(['index.html', 'manifest.json', 'refresh.mjs'])
    await page.getByTestId('ab-reflow-site-delete').click()
    await page.getByTestId('ab-reflow-delete-accept').click()
    await expect.poll(() => keysOf(`${SITE}/`), { timeout: 30_000 }).toEqual([])
    await expect(page.getByTestId('ab-reflow-library')).toBeVisible()
    await expect(row).toHaveCount(0)
    expect(await reflowState(story(1))).toMatchObject({ package: null, result: null, fallback: null, view: null })
  } finally {
    await app.context.close()
  }
})
