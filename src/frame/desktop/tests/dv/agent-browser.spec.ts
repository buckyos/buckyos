/**
 * The Agent browser on a real Zone: original page mode and handover
 * (notepads/agent-runtime-browser-todo.md §8.4, doc/opendan/protocol/Agent Browser Protocol.md §1–§4):
 *
 *   BUCKYOS_TEST_ZONE_HOST=test.buckyos.io BUCKYOS_TEST_ADMIN_PASSWORD=… BUCKYOS_TEST_BROWSER_AGENT=webby.test.buckyos.io \
 *     pnpm exec playwright test --config=playwright.dv.config.ts tests/dv/agent-browser.spec.ts
 *
 * Needs:
 * - a ready Agent of the admin user (`BUCKYOS_TEST_BROWSER_AGENT`, its AgentId) whose browser is available
 *   (ext-core with Chromium on its node). The specs use its browser (the one group `default`) and leave one tab there.
 * - the Agent's browser must reach this machine: the specs serve a test page on 0.0.0.0 and open it as
 *   `http://<BUCKYOS_TEST_SITE_HOST>:<port>/` (default 172.17.0.1, the Docker bridge of a Linux node).
 * - a second user (`BUCKYOS_TEST_SHARE_USER`) for the owner-only checks.
 * - the Desktop under test: the Zone root by default; `BUCKYOS_UI_DV_BASE_URL=http://127.0.0.1:<port>` runs the
 *   same specs on a Vite dev server proxied to the Zone (`VITE_CP_USE_MOCK=false VITE_ZONE_PROXY=https://<zone>
 *   VITE_ZONE_PROXY_IP=127.0.0.1 VITE_NFS_PROXY=http://127.0.0.1:4110`).
 *
 * No message is sent to the Agent (nothing wakes the model): the Agent's side of the handover is played with the
 * owner's token (`browser.acquire`, `release`, `request_handover`). The specs create and delete two files in the
 * owner's home, one saved download and its copy in the Agent's `artifacts` bucket. Left behind: the browser
 * conversation (a session with the Agent titled “Browser”, with the source mark), started and bound the first time
 * it is needed and kept as the Agent's (`browser.bind_default`).
 */
import { createHash } from 'node:crypto'
import { createServer, type Server } from 'node:http'
import type { AddressInfo } from 'node:net'
import { devices, expect, test, type APIRequestContext, type Browser, type BrowserContext, type Page } from '@playwright/test'
import { openFromAllApps } from '../e2e/fixtures/desktop'

type Json = Record<string, any> // eslint-disable-line @typescript-eslint/no-explicit-any

const zone = process.env.BUCKYOS_TEST_ZONE_HOST || 'test.buckyos.io'
const origin = `https://${zone}`
const base = process.env.BUCKYOS_UI_DV_BASE_URL || origin
const owner = process.env.BUCKYOS_TEST_ADMIN_USER || 'devtest'
const ownerPassword = process.env.BUCKYOS_TEST_ADMIN_PASSWORD || 'bucky2025'
const peer = process.env.BUCKYOS_TEST_SHARE_USER || 'bob'
const peerPassword = process.env.BUCKYOS_TEST_USER_PASSWORD || 'buckyos2026'
const AGENT = process.env.BUCKYOS_TEST_BROWSER_AGENT || `webby.${zone}`
const siteHost = process.env.BUCKYOS_TEST_SITE_HOST || '172.17.0.1'
const ownerDid = `did:bns:${owner}`

test.describe.configure({ mode: 'serial' })

// ── The test page: a form at fixed places that reports what it holds back to this process ──

const LEFT = ['text', 'area', 'pick', 'chk', 'count', 'file', 'dl', 'next']
const RIGHT = ['alert', 'confirm', 'prompt', 'popup', 'cookie', 'geo']
/** Where an element of the form is, in the page's CSS pixels. */
function at(id: string): { x: number; y: number } {
  const column = LEFT.includes(id) ? 0 : 1
  const n = (column === 0 ? LEFT : RIGHT).indexOf(id)
  const link = id === 'dl' || id === 'next' || id === 'popup'
  return { x: (column === 0 ? 8 : 186) + (id === 'chk' ? 14 : id === 'file' ? 40 : link ? 30 : 85), y: 64 + n * 44 + (link ? 8 : 18) }
}
const place = (id: string) => `left:${LEFT.includes(id) ? 8 : 186}px;top:${64 + (LEFT.includes(id) ? LEFT : RIGHT).indexOf(id) * 44}px`
const FORM = `<!doctype html><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>DV form</title>
<style>body{margin:0;font:14px sans-serif;height:2400px}.row{position:absolute;width:170px;height:36px;box-sizing:border-box;font-size:14px}
#mode{position:absolute;left:0;top:0;width:100%;height:52px;background:#06c;color:#fff}@media (max-width:600px){#mode{background:#c00}}</style>
<div id="mode">DV 表单</div>
<input id="text" class="row" style="${place('text')}"><textarea id="area" class="row" style="${place('area')}"></textarea>
<select id="pick" class="row" style="${place('pick')}"><option>Red</option><option>Green</option><option>Blue</option><option>蓝绿</option></select>
<label class="row" style="${place('chk')}"><input id="chk" type="checkbox" style="width:24px;height:24px"> check</label>
<button id="count" class="row" style="${place('count')}">count</button>
<input id="file" type="file" class="row" style="${place('file')}" multiple>
<a id="dl" class="row" style="${place('dl')}" href="/download.txt">download</a><a id="next" class="row" style="${place('next')}" href="/page2">page2</a>
<button id="alert" class="row" style="${place('alert')}">alert</button><button id="confirm" class="row" style="${place('confirm')}">confirm</button>
<button id="prompt" class="row" style="${place('prompt')}">prompt</button><a id="popup" class="row" style="${place('popup')}" href="/popup" target="_blank">popup</a>
<button id="cookie" class="row" style="${place('cookie')}">cookie</button><button id="geo" class="row" style="${place('geo')}">geo</button>
<script>
const $ = (id) => document.getElementById(id)
const s = { count: 0, confirm: null, prompt: null, alerted: 0, keys: [], comp: [], files: [], geo: null }
const load = Date.now()
let n = 0
function report() {
  fetch('/report', { method: 'POST', keepalive: true, body: JSON.stringify({ ...s, load, n: ++n, text: $('text').value, area: $('area').value, pick: $('pick').value, chk: $('chk').checked, cookie: document.cookie,
    scrollY: Math.round(scrollY), innerWidth, ua: navigator.userAgent, touch: navigator.maxTouchPoints, active: document.activeElement && document.activeElement.id, url: location.href }) }).catch(() => {})
}
for (const id of ['text', 'area']) {
  $(id).addEventListener('input', report)
  for (const type of ['compositionstart', 'compositionupdate', 'compositionend']) $(id).addEventListener(type, (e) => { s.comp.push(type.slice(11) + ':' + e.data); report() })
}
$('pick').addEventListener('change', report); $('chk').addEventListener('change', report)
$('count').addEventListener('click', () => { s.count += 1; report() })
$('alert').addEventListener('click', () => { alert('DV 提醒'); s.alerted += 1; report() })
$('confirm').addEventListener('click', () => { s.confirm = confirm('确定吗?'); report() })
$('prompt').addEventListener('click', () => { s.prompt = prompt('名字?', '默认值'); report() })
$('cookie').addEventListener('click', () => { document.cookie = 'dv_js=1; max-age=86400; path=/'; report() })
$('geo').addEventListener('click', () => navigator.geolocation.getCurrentPosition(() => { s.geo = 'granted'; report() }, () => { s.geo = 'refused'; report() }))
$('file').addEventListener('change', async () => {
  s.files = []
  for (const f of $('file').files) s.files.push({ name: f.name, size: f.size, sha: (await (await fetch('/upload', { method: 'POST', body: f })).json()).sha })
  report()
})
document.addEventListener('keydown', (e) => { s.keys.push(e.key); report() })
document.addEventListener('scroll', () => { clearTimeout(window.__t); window.__t = setTimeout(report, 150) })
document.addEventListener('focusin', report)
report()
</script>`
const plain = (title: string) => `<!doctype html><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>${title}</title><p>${title}</p>`

interface Site { url: string; state: Json; hits: Array<{ path: string; cookie: string }>; reset(): void; close(): Promise<void> }
async function startSite(): Promise<Site> {
  const site = { state: {} as Json, hits: [] as Array<{ path: string; cookie: string }> }
  const server: Server = createServer((req, res) => {
    const path = (req.url ?? '/').split('?')[0]
    const parts: Buffer[] = []
    req.on('data', (chunk: Buffer) => parts.push(chunk))
    req.on('end', () => {
      const body = Buffer.concat(parts)
      if (path === '/report') {
        // The reports of one page can arrive out of order: the newest one of the newest page load is the state.
        const next = JSON.parse(body.toString() || '{}') as Json
        if (!(next.load < site.state.load) && !(next.load === site.state.load && next.n < site.state.n)) site.state = next
        res.end('{}')
        return
      }
      if (path === '/upload') { res.end(JSON.stringify({ sha: createHash('sha256').update(body).digest('hex') })); return }
      site.hits.push({ path, cookie: req.headers.cookie ?? '' })
      if (path === '/download.txt') { res.writeHead(200, { 'Content-Type': 'text/plain; charset=utf-8', 'Content-Disposition': 'attachment; filename="dv-download.txt"' }); res.end('DV download 下载内容\n'); return }
      res.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8', ...(path === '/' ? { 'Set-Cookie': 'dv_http=1; Max-Age=86400; Path=/; HttpOnly' } : {}) })
      res.end(path === '/' ? FORM : plain(`DV ${path.slice(1)}`))
    })
  })
  await new Promise<void>((resolve) => server.listen(0, '0.0.0.0', resolve))
  return {
    url: `http://${siteHost}:${(server.address() as AddressInfo).port}`,
    get state() { return site.state },
    get hits() { return site.hits },
    reset() { site.state = {}; site.hits.length = 0 },
    close: () => new Promise<void>((resolve) => server.close(() => resolve())),
  }
}

// ── The Zone ──

function hashPassword(username: string, password: string, nonce: number): string {
  const original = createHash('sha256').update(`${password}${username}.buckyos`, 'utf8').digest('base64')
  return createHash('sha256').update(`${original}${nonce}`, 'utf8').digest('base64')
}

async function rpc(request: APIRequestContext, service: string, method: string, params: Json, token?: string): Promise<Json> {
  const seq = Date.now()
  const response = await request.post(`${origin}/kapi/${service}`, { data: { method, params, sys: token ? [seq, token] : [seq] } })
  const body = await response.json().catch(() => ({ error: `HTTP ${response.status()}` })) as { result?: Json; error?: string }
  if (body.error) throw new Error(String(body.error))
  return body.result ?? {}
}
const refusal = (call: Promise<unknown>) => call.then(() => 'accepted', (error: Error) => error.message)

async function apiLogin(request: APIRequestContext, username = owner, password = ownerPassword): Promise<Json> {
  const nonce = Date.now()
  return rpc(request, 'control-panel', 'auth.login', {
    username, password: hashPassword(username, password, nonce), appid: 'control-panel',
    target: { kind: 'system', service_id: 'control-panel' }, login_nonce: nonce, remember_me: true,
  })
}
const browserRpc = (request: APIRequestContext, token: string, method: string, params: Json = {}) => rpc(request, AGENT, method, params, token)

/** Minimal NFSP client, for fixtures and for checking what is really in the owner's files. */
async function nfsp(request: APIRequestContext, token: string) {
  let seq = 0
  const headers = { Authorization: `Bearer ${token}` }
  const call = async (method: string, body: Json) => {
    const parsed = await (await request.post(`${origin}/nfs/v1/${method}`, { headers, data: body })).json() as { ok: boolean; result: Json; error?: Json }
    if (!parsed.ok) throw new Error(`${method}: ${JSON.stringify(parsed.error)}`)
    return parsed.result
  }
  const session = (await call('hello', { args: {} })).session
  return {
    write: async (path: string, bytes: Buffer) => {
      const name = path.split('/').pop()!
      const parent = await call('stat', { session, args: {}, at: { realm: 'dfs', path: path.slice(0, path.lastIndexOf('/')) } })
      const open = await call('open_write', { session, seq: ++seq, args: { parent_ref: parent.ref, name, size: bytes.length } })
      await request.fetch(`${origin}/nfs/v1/uploads/${open.fb_handle}`, { method: 'PATCH', headers: { ...headers, 'Upload-Offset': '0', 'Tus-Resumable': '1.0.0', 'Content-Type': 'application/offset+octet-stream' }, data: bytes })
      await call('commit_file', { session, seq: ++seq, args: { name, fb_handle: open.fb_handle, lease_id: open.lease.lease_id }, at: { ref: parent.ref } })
    },
    read: async (path: string) => {
      const info = await call('stat', { session, args: {}, want: ['base', 'ident'], at: { realm: 'dfs', path } })
      return (await request.get(`${origin}/nfs/v1/read/${encodeURIComponent(info.node_id ?? info.ref.node_id)}`, { headers })).text()
    },
    remove: (parent: string, name: string) => call('delete', { session, seq: ++seq, args: { name, recursive: true }, at: { realm: 'dfs', path: parent } }).catch(() => undefined),
  }
}

/** Signs in: through the login page of the Zone root, or on a dev server with what the SSO callback would have left. */
async function signIn(browser: Browser, request: APIRequestContext, options: Parameters<Browser['newContext']>[0] = {}): Promise<{ context: BrowserContext; page: Page; token: string }> {
  const context = await browser.newContext({ ignoreHTTPSErrors: true, viewport: { width: 1440, height: 1000 }, ...options })
  const page = await context.newPage()
  const session = await apiLogin(request)
  if (base.startsWith('https://')) {
    await page.goto(`${base}/`)
    await page.getByLabel('Username', { exact: true }).fill(owner)
    await page.getByLabel('Password', { exact: true }).fill(ownerPassword)
    await page.getByRole('button', { name: 'Sign In', exact: true }).click()
  } else {
    await context.addCookies([{ name: 'buckyos_refresh_token', value: String(session.refresh_token), domain: new URL(base).hostname, path: '/', httpOnly: true, secure: false, sameSite: 'Lax' }])
    await page.addInitScript((userInfo) => { window.localStorage.setItem('user_info', JSON.stringify(userInfo)) }, session.user_info)
    await page.goto(`${base}/`)
  }
  await expect(page.getByRole('button', { name: 'BuckyOS' }).first()).toBeVisible({ timeout: 40_000 })
  return { context, page, token: String(session.session_token) }
}

// ── The App ──

async function openBrowser(page: Page) {
  await openFromAllApps(page, 'agent-browser')
  const root = page.getByTestId('agent-browser')
  await expect(root).toBeVisible()
  if ((await root.getAttribute('data-agent')) !== AGENT) await page.getByTestId(`ab-agent-${AGENT}`).click()
  await expect(root).toHaveAttribute('data-agent', AGENT)
  await expect(page.getByTestId('ab-screen')).toHaveAttribute('data-status', 'open', { timeout: 40_000 })
  // What an earlier run left waiting.
  await page.waitForTimeout(800)
  const picker = page.getByTestId('window-dialog').filter({ has: page.getByTestId('select-dialog') })
  if (await picker.count()) await picker.getByRole('button', { name: 'Close dialog' }).click()
  for (const id of ['ab-file-cancel', 'ab-download-dismiss']) {
    while (await page.getByTestId(id).count()) { await page.getByTestId(id).first().click(); await page.waitForTimeout(200) }
  }
  if (await page.getByTestId('ab-dialog').count()) await page.getByTestId('ab-dialog-accept').click()
}

async function browse(page: Page, address: string) {
  const input = page.getByTestId('ab-address-input')
  await input.click()
  await input.fill(address)
  await input.press('Enter')
  await expect(page.getByTestId('ab-screen-canvas')).toHaveAttribute('data-seq', /\d+/)
  await expect(page.getByTestId('ab-screen-canvas')).toBeVisible()
}

/** Opens the form in the default group's only tab. */
async function openForm(page: Page, site: Site) {
  site.reset()
  await browse(page, `${site.url}/`)
  await expect.poll(() => site.state.url).toBe(`${site.url}/`)
  for (let left = 12; left > 0 && Number(await page.getByTestId('ab-tabs-count').innerText()) > 1; left--) {
    const count = Number(await page.getByTestId('ab-tabs-count').innerText())
    await page.getByTestId('ab-tabs-button').click()
    await page.locator('[data-testid="ab-tab"][data-active="false"]').first().locator('button[data-testid^="ab-tab-close-"]').click()
    await expect(page.getByTestId('ab-tabs-count')).toHaveText(String(count - 1))
    await page.getByTestId('ab-tabs-close').click()
  }
}

/** A point of the remote page (its CSS pixels) as a point of this window. */
async function onScreen(page: Page, id: string): Promise<{ x: number; y: number }> {
  const point = at(id)
  const [width, height] = ((await page.getByTestId('ab-screen-surface').getAttribute('data-viewport')) ?? '1x1').split('x').map(Number)
  const box = (await page.getByTestId('ab-screen-canvas').boundingBox())!
  return { x: box.x + (point.x / width) * box.width, y: box.y + (point.y / height) * box.height }
}
async function press(page: Page, id: string) { const p = await onScreen(page, id); await page.mouse.click(p.x, p.y) }
async function tap(page: Page, id: string) { const p = await onScreen(page, id); await page.touchscreen.tap(p.x, p.y) }

const selectRow = (page: Page, name: string) => page.locator(`[data-testid="select-row"][data-name="${name}"]`)
/** The owner's own folder in the system picker ("My files" of an administrator is /home). */
async function pickerHome(page: Page) {
  const dialog = page.getByTestId('select-dialog')
  await dialog.getByTestId('select-locations').getByRole('button', { name: /^(My files|我的文件)$/ }).click()
  await expect(dialog.getByTestId('select-crumbs')).toContainText('home')
  await page.waitForTimeout(600)
  if (await selectRow(page, owner).count()) await selectRow(page, owner).click()
  await expect(dialog.getByTestId('select-crumbs')).toContainText(owner)
}

let site: Site
test.beforeAll(async ({ request }) => {
  const { session_token: token } = await apiLogin(request)
  const status = await browserRpc(request, String(token), 'browser.status', {}).catch((error: Error): Json => ({ error: error.message }))
  test.skip(!status.available || !status.enabled, `the browser of ${AGENT} is not available: ${JSON.stringify(status).slice(0, 200)}`)
  site = await startSite()
})
test.afterAll(async () => { await site?.close() })

test('a page is browsed: address, link, back and forward, reload, tabs, a window the page opens', async ({ browser, request }) => {
  const { context, page } = await signIn(browser, request)
  await openBrowser(page)
  await openForm(page, site)
  const address = page.getByTestId('ab-address-input')
  const shown = (path: string) => `${site.url}${path}`.replace(/\/$/, '')
  await expect(address).toHaveValue(shown('/'))

  await press(page, 'next')
  await expect(address).toHaveValue(shown('/page2'))
  await page.getByTestId('ab-back').click()
  await expect(address).toHaveValue(shown('/'))
  await page.getByTestId('ab-forward').click()
  await expect(address).toHaveValue(shown('/page2'))
  const loads = site.hits.filter((hit) => hit.path === '/page2').length
  await page.getByTestId('ab-reload').click()
  await expect.poll(() => site.hits.filter((hit) => hit.path === '/page2').length).toBe(loads + 1)
  await page.getByTestId('ab-back').click()
  await expect(address).toHaveValue(shown('/'))

  // A new tab, switching, and the page's own new window as a tab of the same group.
  await page.getByTestId('ab-tabs-button').click()
  await page.getByTestId('ab-tab-new').click()
  await expect(page.getByTestId('ab-tabs-count')).toHaveText('2')
  await browse(page, `${site.url}/page2`)
  await page.getByTestId('ab-tabs-button').click()
  await expect(page.locator('[data-testid="ab-tab"]')).toHaveCount(2)
  await page.locator('[data-testid="ab-tab"][data-active="false"] .ab-list-item').click()
  await expect(address).toHaveValue(shown('/'))
  await press(page, 'popup')
  await expect(page.getByTestId('ab-tabs-count')).toHaveText('3')
  await expect(address).toHaveValue(shown('/popup'))
  await page.getByTestId('ab-tabs-button').click()
  await page.locator('[data-testid="ab-tab"][data-active="true"] button[data-testid^="ab-tab-close-"]').click()
  await expect(page.getByTestId('ab-tabs-count')).toHaveText('2')
  await context.close()
})

test('typing reaches the page: keys, whole text, an input method composing; select, dialogs, a refused permission', async ({ browser, request }) => {
  const { context, page } = await signIn(browser, request)
  await openBrowser(page)
  await openForm(page, site)
  const state = () => site.state

  await press(page, 'text')
  await expect.poll(() => state().active).toBe('text')
  await expect(page.getByTestId('ab-screen-input')).toBeFocused()
  await page.keyboard.type('hello')
  await expect.poll(() => state().text).toBe('hello')
  await page.keyboard.insertText('中文输入')
  await expect.poll(() => state().text).toBe('hello中文输入')
  // The browser's own input method path: composition events on the hidden input, then the commit.
  const cdp = await context.newCDPSession(page)
  await cdp.send('Input.imeSetComposition', { text: 'ni hao', selectionStart: 6, selectionEnd: 6 })
  await expect.poll(() => state().text).toBe('hello中文输入ni hao')
  await cdp.send('Input.insertText', { text: '你好' })
  await expect.poll(() => state().text).toBe('hello中文输入你好')
  expect(state().comp).toEqual(expect.arrayContaining(['start:', 'update:ni hao', 'end:你好']))
  await page.keyboard.press('Backspace')
  await expect.poll(() => state().text).toBe('hello中文输入你')
  await page.keyboard.press('Control+a')
  await page.keyboard.type('Z')
  await expect.poll(() => state().text).toBe('Z')
  await page.keyboard.press('Tab')
  await expect.poll(() => state().active).toBe('area')
  await page.keyboard.type('line1')
  await page.keyboard.press('Enter')
  await page.keyboard.insertText('第二行')
  await expect.poll(() => state().area).toBe('line1\n第二行')

  await press(page, 'chk')
  await expect.poll(() => state().chk).toBe(true)
  await press(page, 'count')
  await expect.poll(() => state().count).toBe(1)

  await press(page, 'pick')
  await expect(page.getByTestId('ab-select')).toBeVisible()
  await page.getByTestId('ab-select-option-3').click()
  await expect.poll(() => state().pick).toBe('蓝绿')

  await press(page, 'alert')
  await expect(page.getByTestId('ab-dialog-message')).toHaveText('DV 提醒')
  await page.getByTestId('ab-dialog-accept').click()
  await expect.poll(() => state().alerted).toBe(1)
  await press(page, 'confirm')
  await expect(page.getByTestId('ab-dialog-message')).toHaveText('确定吗?')
  await page.getByTestId('ab-dialog-cancel').click()
  await expect.poll(() => state().confirm).toBe(false)
  await press(page, 'prompt')
  await expect(page.getByTestId('ab-dialog-input')).toHaveValue('默认值')
  await page.getByTestId('ab-dialog-input').fill('张三')
  await page.getByTestId('ab-dialog-accept').click()
  await expect.poll(() => state().prompt).toBe('张三')

  // A permission: the headless browser refuses it itself, the App is not asked.
  await press(page, 'geo')
  await expect.poll(() => state().geo).toBe('refused')
  await expect(page.getByTestId('ab-notes')).toHaveCount(0)

  const middle = (await page.getByTestId('ab-screen-canvas').boundingBox())!
  await page.mouse.move(middle.x + middle.width / 2, middle.y + middle.height / 2)
  await page.mouse.wheel(0, 400)
  await expect.poll(() => state().scrollY).toBeGreaterThan(300)
  await context.close()
})

test('files: from the Zone and from this device into the page; a download saved to the Zone', async ({ browser, request }) => {
  const stamp = Date.now().toString(36)
  const home = `/home/${owner}`
  const sha = (bytes: Buffer) => createHash('sha256').update(bytes).digest('hex')
  const zoneFiles = [
    { name: `dv-browser-上传-${stamp}.txt`, bytes: Buffer.from(`zone upload ${stamp} 中文\n`.repeat(40)) },
    { name: `dv-browser-upload-${stamp}.bin`, bytes: Buffer.alloc(300_000, 7) },
  ]
  const saved = `dv-browser-download-${stamp}.txt`
  const { context, page, token } = await signIn(browser, request)
  const files = await nfsp(request, token)
  for (const file of zoneFiles) await files.write(`${home}/${file.name}`, file.bytes)
  try {
    await openBrowser(page)
    await openForm(page, site)
    const picker = page.getByTestId('select-dialog')

    // The picker opens at once on the page's own upload control; the service reads the files with the owner's token.
    await press(page, 'file')
    await expect(picker).toBeVisible()
    await pickerHome(page)
    for (const file of zoneFiles) await selectRow(page, file.name).click()
    await picker.getByTestId('select-confirm').click()
    await expect.poll(() => site.state.files?.length, { timeout: 30_000 }).toBe(2)
    for (const file of zoneFiles) expect(site.state.files).toContainEqual({ name: file.name, size: file.bytes.length, sha: sha(file.bytes) })
    await expect(page.getByTestId('ab-file-chooser')).toHaveCount(0)

    const local = { name: '本机 文件.png', mimeType: 'image/png', buffer: Buffer.alloc(1_500_000, 3) }
    await press(page, 'file')
    await expect(picker).toBeVisible()
    const chooser = page.waitForEvent('filechooser')
    await picker.getByTestId('select-local').click()
    await (await chooser).setFiles(local)
    await expect.poll(() => site.state.files?.[0]?.name, { timeout: 30_000 }).toBe(local.name)
    expect(site.state.files).toEqual([{ name: local.name, size: local.buffer.length, sha: sha(local.buffer) }])

    await press(page, 'dl')
    const note = page.getByTestId('ab-download')
    await expect(note).toBeVisible()
    await note.getByTestId('ab-download-save').click()
    await expect(picker).toHaveAttribute('data-mode', 'save')
    await expect(picker.getByTestId('select-name')).toHaveValue('dv-download.txt')
    await pickerHome(page)
    await picker.getByTestId('select-name').fill(saved)
    await picker.getByTestId('select-confirm').click()
    await expect(note).toHaveAttribute('data-state', 'saved', { timeout: 30_000 })
    expect(await files.read(`${home}/${saved}`)).toBe('DV download 下载内容\n')
    await note.getByTestId('ab-download-dismiss').click()

    // A file request left waiting is offered when the App is opened again; the picker opens only when asked for.
    await press(page, 'file')
    await expect(picker).toBeVisible()
    await page.reload()
    await expect(page.getByRole('button', { name: 'BuckyOS' }).first()).toBeVisible({ timeout: 40_000 })
    await openFromAllApps(page, 'agent-browser')
    await expect(page.getByTestId('ab-file-chooser')).toBeVisible({ timeout: 40_000 })
    await page.waitForTimeout(1500)
    await expect(picker).toHaveCount(0)
    await page.getByTestId('ab-file-choose').click()
    await expect(picker).toBeVisible()
    await page.getByTestId('window-dialog').filter({ has: picker }).getByRole('button', { name: 'Close dialog' }).click()
    await expect(page.getByTestId('ab-file-chooser')).toHaveCount(0)
  } finally {
    for (const name of [...zoneFiles.map((file) => file.name), saved]) await files.remove(home, name)
    // The exported download stays in the Agent's `artifacts` bucket.
    const bucket = `user/${AGENT}/${owner}/artifacts`
    const kept = await rpc(request, 'nfs-server', 'bucket.list_objects', { bucket_id: bucket }, token).catch(() => ({}) as Json)
    for (const item of (kept.items ?? []) as Json[]) {
      if (String(item.key).endsWith('/dv-download.txt')) await rpc(request, 'nfs-server', 'bucket.delete', { bucket_id: bucket, key: item.key }, token).catch(() => undefined)
    }
    await context.close()
  }
})

test('handover: view only while the Agent operates; take over, hand back, done, a request to take over, a lease that runs out', async ({ browser, request }) => {
  test.setTimeout(240_000)
  const { context, page, token } = await signIn(browser, request)
  await openBrowser(page)
  await openForm(page, site)
  const status = page.getByTestId('ab-status')
  const session = 'dv-no-such-session' // every session, also one the service does not know, works in the one group
  /** Hands back without the message that would wake the Agent. */
  const handBack = async () => {
    await page.getByTestId('ab-hand-back').click()
    await page.getByTestId('ab-handover-note').fill('')
    await page.getByTestId('ab-handover-confirm').click()
    await expect(status).toHaveAttribute('data-user-hold', 'false')
  }

  const lease = await browserRpc(request, token, 'browser.acquire', { session_id: session })
  expect(lease.group_id).toBe('default')
  await expect(status).toHaveAttribute('data-controller', 'agent')
  await expect(page.getByTestId('ab-screen-readonly')).toBeVisible()
  await expect(page.getByTestId('ab-address-input')).toHaveAttribute('readonly', '')
  await press(page, 'count')
  await expect(page.getByTestId('ab-toast')).toHaveAttribute('data-kind', 'agent_in_control')
  await page.waitForTimeout(600)
  expect(site.state.count).toBe(0)

  await page.getByTestId('ab-take-over').click()
  await expect(status).toHaveAttribute('data-user-hold', 'true')
  await expect(page.getByTestId('ab-screen-readonly')).toHaveCount(0)
  expect(await refusal(browserRpc(request, token, 'browser.acquire', { session_id: session }))).toContain('user_in_control')
  expect(await refusal(browserRpc(request, token, 'browser.renew', { lease_id: lease.lease_id }))).toContain('lease_lost')
  await press(page, 'count')
  await expect.poll(() => site.state.count).toBe(1)

  await handBack()
  const second = await browserRpc(request, token, 'browser.acquire', { session_id: session })
  await expect(status).toHaveAttribute('data-controller', 'agent')
  await browserRpc(request, token, 'browser.release', { lease_id: second.lease_id, state: 'done', note: 'DV: 做完了' })
  await expect(status).toHaveAttribute('data-agent-state', 'done')
  await expect(status).toContainText('DV: 做完了')

  await browserRpc(request, token, 'browser.request_handover', { session_id: session, reason: 'password', note: 'DV: 请输入密码' })
  await expect(status).toHaveAttribute('data-agent-state', 'waiting_takeover')
  await expect(status).toContainText('DV: 请输入密码')
  await press(page, 'count')
  await expect.poll(() => site.state.count).toBe(2)
  await handBack()
  await expect(status).toHaveAttribute('data-agent-state', 'idle')

  // A lease nobody renews: the user operates again after its 60 seconds.
  const last = await browserRpc(request, token, 'browser.acquire', { session_id: session })
  await expect(status).toHaveAttribute('data-controller', 'agent')
  await expect(status).toHaveAttribute('data-controller', 'user', { timeout: 90_000 })
  expect(await refusal(browserRpc(request, token, 'browser.release', { lease_id: last.lease_id }))).toContain('lease_lost')
  await context.close()
})

test('the browser conversation: a session with the source mark, bound the first time it is needed, the same one after that', async ({ browser, request }) => {
  const { context, page, token } = await signIn(browser, request)
  await openBrowser(page)
  await openForm(page, site)
  expect((await browserRpc(request, token, 'browser.status', {})).groups.map((item: Json) => item.group_id)).toEqual(['default'])
  await page.getByTestId('ab-chat-toggle').click()
  const conversation = page.getByTestId('ab-chat-sheet').getByTestId('ab-conversation')
  await expect(conversation).toBeVisible({ timeout: 30_000 })
  const session = (await conversation.getAttribute('data-session'))!
  await expect.poll(async () => (await browserRpc(request, token, 'browser.status', {})).default_session_id).toBe(session)
  const marks = await rpc(request, 'msg-center', 'ui_session.list_state', { session_id: session, owner: ownerDid }, token) as unknown as Array<{ key: string; value: unknown }>
  expect(marks).toContainEqual(expect.objectContaining({ key: 'ui.source', value: 'agent-browser' }))
  expect(await refusal(browserRpc(request, token, 'browser.status', { group_id: session }))).toContain('no_such_group')

  // The next visit (a link to the Agent's browser in conversation mode) finds the same conversation and binds nothing new.
  await page.goto(`${base}/?agent_browser=${AGENT}&mode=chat`)
  await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-mode', 'chat', { timeout: 40_000 })
  await expect(page.getByTestId('ab-chat-panel').getByTestId('ab-conversation')).toHaveAttribute('data-session', session, { timeout: 30_000 })
  expect((await browserRpc(request, token, 'browser.status', {})).default_session_id).toBe(session)
  await context.close()
})

test('only the owner reaches the screen, the owner methods and the upload', async ({ request }) => {
  const ownerToken = String((await apiLogin(request)).session_token)
  const other = await apiLogin(request, peer, peerPassword).then((session) => String(session.session_token), () => `${ownerToken.slice(0, -6)}AAAAAA`)
  const hello = (token: string | null) => new Promise<Json>((resolve) => {
    const socket = new WebSocket(`wss://${zone}/kapi/${AGENT}/browser/screen`)
    const timer = setTimeout(() => { socket.close(); resolve({ type: 'timeout' }) }, 20_000)
    socket.onopen = () => socket.send(JSON.stringify({ type: 'hello', ...(token ? { token } : {}), group_id: 'default', viewport: { width: 800, height: 600, mobile: false } }))
    socket.onmessage = (event) => {
      if (typeof event.data !== 'string') return
      const message = JSON.parse(event.data) as Json
      if (message.type !== 'ready' && message.type !== 'error') return
      clearTimeout(timer)
      socket.close()
      resolve(message)
    }
  })
  const previous = process.env.NODE_TLS_REJECT_UNAUTHORIZED
  process.env.NODE_TLS_REJECT_UNAUTHORIZED = '0' // the Zone's own certificate authority
  try {
    expect(await hello(other)).toMatchObject({ type: 'error', code: 'unauthorized' })
    expect(await hello('not-a-token')).toMatchObject({ type: 'error', code: 'unauthorized' })
    expect(await hello(null)).toMatchObject({ type: 'error', code: 'unauthorized' })
    expect(await hello(ownerToken)).toMatchObject({ type: 'ready' })
  } finally {
    if (previous === undefined) delete process.env.NODE_TLS_REJECT_UNAUTHORIZED
    else process.env.NODE_TLS_REJECT_UNAUTHORIZED = previous
  }
  for (const [method, params] of [['browser.take_over', { group_id: 'default' }], ['browser.clear_profile', {}], ['browser.status', {}], ['browser.acquire', { session_id: 'x' }], ['browser.download_export', { download_id: 'x' }]] as const) {
    expect(await refusal(rpc(request, AGENT, method, params, other)), method).toContain('No permission')
    expect(await refusal(rpc(request, AGENT, method, params)), `${method} without a token`).toContain('No permission')
  }
  const put = (token: string | null, id: string) => request.fetch(`${origin}/kapi/${AGENT}/browser/upload/${id}?name=a.txt&done=1`, { method: 'PUT', headers: token ? { Authorization: `Bearer ${token}` } : {}, data: Buffer.from('x') })
  expect((await put(other, 'r1')).status()).toBe(403)
  expect((await put(null, 'r1')).status()).toBe(401)
  expect((await put(ownerToken, 'r-none')).status()).toBe(404)
})

test('clearing the sign-ins signs the website out; the tabs come back', async ({ browser, request }) => {
  const { context, page } = await signIn(browser, request)
  await openBrowser(page)
  await openForm(page, site)
  const lastLoad = () => site.hits.filter((hit) => hit.path === '/').at(-1)
  await press(page, 'cookie')
  await expect.poll(() => site.state.cookie).toContain('dv_js=1')
  const before = site.hits.length
  await page.getByTestId('ab-reload').click()
  await expect.poll(() => site.hits.length).toBeGreaterThan(before)
  expect(lastLoad()?.cookie).toContain('dv_http=1')
  expect(lastLoad()?.cookie).toContain('dv_js=1')
  const loads = site.hits.length

  await page.getByTestId('ab-menu-button').click()
  await page.getByTestId('ab-clear-profile').click()
  await page.getByTestId('ab-clear-accept').click()
  await expect(page.getByTestId('ab-toast')).toHaveAttribute('data-kind', 'profile_cleared', { timeout: 30_000 })
  // The browser starts again with the page it had, which the site now sees without its cookies.
  await expect.poll(() => site.hits.length, { timeout: 60_000 }).toBeGreaterThan(loads)
  expect(lastLoad()?.cookie).toBe('')
  await expect.poll(() => site.state.cookie).toBe('')
  await expect(page.getByTestId('ab-address-input')).toHaveValue(site.url)
  await press(page, 'count')
  await expect.poll(() => site.state.count).toBe(1)
  await context.close()
})

test('on a phone: the phone layout, taps, the keyboard input and Chinese, a select; the desktop site scaled to the width', async ({ browser, request }) => {
  const phone = devices['Pixel 7']
  const { context, page } = await signIn(browser, request, { viewport: phone.viewport, userAgent: phone.userAgent, deviceScaleFactor: phone.deviceScaleFactor, isMobile: true, hasTouch: true })
  await openBrowser(page)
  const desktopSite = async (on: boolean) => {
    await page.getByTestId('ab-menu-button').tap()
    if ((await page.getByTestId('ab-desktop-site').getAttribute('aria-checked')) !== String(on)) await page.getByTestId('ab-desktop-site').tap()
    await page.getByTestId('ab-menu-close').tap()
  }
  await desktopSite(false)
  await openForm(page, site)
  const surface = page.getByTestId('ab-screen-surface')
  await expect(surface).toHaveAttribute('data-mobile', 'true')
  await expect.poll(() => site.state.innerWidth).toBe(phone.viewport.width)
  expect(site.state.ua).toContain('Mobile')
  expect(site.state.touch).toBeGreaterThan(0)

  // The page's focus on a text field brings up the keyboard input; a press elsewhere puts it away.
  await tap(page, 'text')
  await expect.poll(() => site.state.active).toBe('text')
  await expect(page.getByTestId('ab-screen-input')).toBeFocused()
  await expect(page.getByTestId('ab-keyboard')).toHaveAttribute('aria-pressed', 'true')
  await page.keyboard.insertText('手机输入')
  await expect.poll(() => site.state.text).toBe('手机输入')
  const cdp = await context.newCDPSession(page)
  await cdp.send('Input.imeSetComposition', { text: 'zhong', selectionStart: 5, selectionEnd: 5 })
  await cdp.send('Input.insertText', { text: '中' })
  await expect.poll(() => site.state.text).toBe('手机输入中')
  await page.getByTestId('ab-keyboard').tap()
  await expect(page.getByTestId('ab-keyboard')).toHaveAttribute('aria-pressed', 'false')
  await page.getByTestId('ab-keyboard').tap()
  await expect(page.getByTestId('ab-screen-input')).toBeFocused()
  await tap(page, 'count')
  await expect.poll(() => site.state.count).toBe(1)
  await expect(page.getByTestId('ab-keyboard')).toHaveAttribute('aria-pressed', 'false')

  await tap(page, 'pick')
  await page.getByTestId('ab-select-option-2').tap()
  await expect.poll(() => site.state.pick).toBe('Blue')
  await tap(page, 'confirm')
  await page.getByTestId('ab-dialog-accept').tap()
  await expect.poll(() => site.state.confirm).toBe(true)

  await desktopSite(true)
  await expect(surface).toHaveAttribute('data-viewport', /^1280x/)
  await expect.poll(() => site.state.innerWidth).toBe(1280)
  expect(site.state.ua).not.toContain('Mobile')
  expect(Math.round((await page.getByTestId('ab-screen-canvas').boundingBox())!.width)).toBe(phone.viewport.width)
  await tap(page, 'count')
  await expect.poll(() => site.state.count).toBe(1)
  await desktopSite(false)
  await expect.poll(() => site.state.innerWidth).toBe(phone.viewport.width)
  await context.close()
})
