/**
 * Home mode of the Agent browser on a real Zone (notepads/agent-browser-phase2-todo.md §7; Personal Site Protocol,
 * Agent Browser Protocol §5): the personal site and the pages tidied one by one, both read from the owner's reflow
 * bucket and shown from `reflow.<zone>`.
 *
 *   BUCKYOS_TEST_ZONE_HOST=test.buckyos.io BUCKYOS_TEST_ADMIN_PASSWORD=… BUCKYOS_TEST_REFLOW_AGENT=reflowy.test.buckyos.io \
 *     ./node_modules/.bin/playwright test --config=playwright.dv.config.ts tests/dv/agent-browser.reflow.spec.ts
 *
 * Needs:
 * - the deployed Desktop at the Zone root (the pages are a frame of `reflow.<zone>` that carries a session cookie:
 *   it only loads from the Zone's own site, not from a dev server);
 * - an Agent of the owner (`BUCKYOS_TEST_REFLOW_AGENT`, its AgentId) that runs in a container on this machine with
 *   the browser of ext-core. The spec stops that browser once (`browser.clear_profile`: its sign-ins are gone) and
 *   opens tabs in it;
 * - the container reaching this machine as `BUCKYOS_TEST_REFLOW_SITE_HOST` (default `host.docker.internal`): the
 *   spec serves its own "old news site" on 0.0.0.0:`BUCKYOS_TEST_REFLOW_SITE_PORT` (default 18743);
 * - optionally a second user (`BUCKYOS_TEST_SHARE_USER`, default `bob`); without one that part is skipped.
 *
 * The spec owns the tidied pages of that host in the owner's bucket (`host-docker-internal/`), removed at the start
 * and at the end. The personal site (`_data/…`, `index.html`) is the owner's: the spec puts the shared samples there
 * only when the bucket has no site, and takes them away again; with a site in place those tests are skipped.
 *
 * What the App would write to the Agent (tidy this page, an upgrade) is held back and only looked at, so no model
 * is woken. `BUCKYOS_TEST_REFLOW_ASK_AGENT=1` adds the one test that lets a request through and waits up to ten
 * minutes for the Agent's page; it costs tokens.
 */
import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { createServer, type Server } from 'node:http'
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
const story = (n: number) => `${siteOrigin}/news/${n}`
const bucketOf = (user: string) => `user/control-panel/${user}/reflow`
const sample = (name: string) => JSON.parse(readFileSync(new URL(`../../../../../doc/opendan/protocol/fixtures/site/${name}.json`, import.meta.url), 'utf8')) as JsonRecord

test.describe.configure({ mode: 'serial' })

// ── The old news site ──

const stories = [
  { title: '老城区图书馆周末延长开放 <b>到晚九点</b>', date: '2026-11-03', first: ['市图书馆宣布，自本周起周末开放时间延长到晚上九点。', '延长开放是应读者 & 学生的要求。'], second: ['第二页：新增的自习座位共 120 个。'] },
  { title: '河滨步道二期本月完工', date: '2026-11-08', first: ['河滨步道二期工程进入收尾阶段。'], second: ['第二页：沿线新增路灯 240 盏。'] },
  { title: '社区菜市场改造后重新开张', date: '2026-11-12', first: ['改造历时四个月的社区菜市场今天重新开张。'], second: ['第二页：市场二层新设便民服务点。'] },
]
const escapeHtml = (text: string) => text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')

function frame(title: string, body: string): string {
  return `<!doctype html><html><head><meta charset="utf-8"><title>${escapeHtml(title)} - 晨报网</title></head><body>
<table width="980"><tr><td colspan="2"><font size="6">晨报网</font> 首页 | 国内 | 国际 | 论坛</td></tr>
<tr><td width="220"><p>广告位招租</p><ol><li>天气预报</li><li>彩票开奖</li></ol></td><td>${body}</td></tr></table></body></html>`
}

function storyPage(index: number): string {
  const item = stories[index]
  const next = stories[index + 1] ? `<a class="next" href="/news/${index + 2}">下一篇：${escapeHtml(stories[index + 1].title)}</a>` : ''
  return frame(item.title, `<h1>${escapeHtml(item.title)}</h1><p class="date">${item.date}</p>
<div id="text">${item.first.map((text) => `<p>${escapeHtml(text)}</p>`).join('')}</div><p><a href="/">返回列表</a> ${next}</p>`)
}

function startSite(): Promise<Server> {
  const server = createServer((request, response) => {
    const path = new URL(request.url ?? '/', 'http://site').pathname
    const index = /^\/news\/([1-3])$/.exec(path)
    const html = (body: string) => { response.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8', 'Cache-Control': 'no-store' }); response.end(body) }
    if (path === '/') {
      html(frame('本地新闻', `<h1>本地新闻</h1><ul>${stories.map((item, i) => `<li><a href="/news/${i + 1}">${escapeHtml(item.title)}</a> <span class="date">${item.date}</span></li>`).join('')}</ul>`))
    } else if (index) {
      html(storyPage(Number(index[1]) - 1))
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


// ── The Zone ──

let api: APIRequestContext
let server: Server
let agentDid = ''
let ownerDid = ''
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

/** `browser.reflow_state` (Agent Browser Protocol §5.3). */
interface ReflowState {
  url: string; site: string; page: string; view: string | null
  fallback: { made_at: number; made_by: string; source_url: string } | null
}
const reflowState = (url: string) => agent<ReflowState>('browser.reflow_state', { url })

const keyUrl = (user: string, key: string) => `${origin}/bucket/v1/${bucketOf(user)}/${key.split('/').map(encodeURIComponent).join('/')}`

async function putKey(key: string, body: string, contentType: string) {
  const response = await api.put(keyUrl(owner, key), { headers: { Authorization: `Bearer ${await tokenOf()}`, 'Content-Type': contentType }, data: body })
  expect(response.status(), `PUT ${key}`).toBeLessThan(300)
}

/** The text of a key of the owner's bucket, or null when it is not there. */
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

const removeKey = async (key: string) => { await call('nfs-server', 'bucket.delete', { bucket_id: bucketOf(owner), key }, await tokenOf()) }

async function removeTidied() {
  for (const key of await keysOf(`${SITE}/`)) await removeKey(key)
}


// ── The Agent's side, played by the spec ──

/** A page tidied for `url`, as `reflow result put` leaves it: the page and its record. It posts the messages of §5.5. */
async function putTidied(url: string, title: string, links: string[] = []): Promise<ReflowState> {
  const state = await reflowState(url)
  const html = `<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>${escapeHtml(title)}</title></head>
<body><h1 id="title">${escapeHtml(title)}</h1>${links.map((link) => `<p><a href="${link}">${escapeHtml(link)}</a></p>`).join('')}<button id="original" type="button">原始页面</button>
<script>
document.addEventListener('click', (event) => {
  const link = event.target.closest('a[href]');
  if (!link) return;
  event.preventDefault();
  parent.postMessage({ type: 'reflow.open', url: link.href }, '*');
});
document.getElementById('original').onclick = () => parent.postMessage({ type: 'reflow.source' }, '*');
parent.postMessage({ type: 'reflow.ready' }, '*');
</script></body></html>`
  await putKey(`${state.site}/fallback/${state.page}.html`, html, 'text/html; charset=utf-8')
  await putKey(`${state.site}/fallback/${state.page}.json`, JSON.stringify({ schema: 'buckyos.reflow.fallback/1', source_url: state.url, made_at: Date.now(), made_by: agentDid }), 'application/json')
  return reflowState(url)
}

/** A stand-in for the site's template when the bucket has none: it draws the routes from `_data` and speaks §6.2. */
const SITE_PAGE = `<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>DV reading site</title></head>
<body><main id="main"></main>
<script type="module">
const tell = (message) => parent.postMessage(message, '*');
const json = async (key) => { const response = await fetch('/' + key, { cache: 'no-cache' }); return response.ok ? response.json() : null; };
const el = (tag, props, ...children) => { const node = Object.assign(document.createElement(tag), props); node.append(...children); return node; };
let route = location.search.slice(1);
async function draw() {
  const main = document.getElementById('main');
  const params = new URLSearchParams(route);
  const site = await json('_data/site.json');
  main.replaceChildren();
  document.body.dataset.route = route;
  if (params.get('a')) {
    const article = await json('_data/a/' + params.get('a') + '.json');
    main.append(el('h1', { id: 'title', textContent: article ? article.title.t : 'Not collected' }));
    for (const link of article ? article.links : []) main.append(el('a', { href: '#', className: 'link', textContent: link.text.t, onclick: (event) => { event.preventDefault(); if (link.article) go('a=' + link.article); else tell({ type: 'site.open', url: link.url }); } }));
    tell(article ? { type: 'site.location', route, title: article.title.t, url: article.url, source: article.source } : { type: 'site.location', route, title: '' });
    return;
  }
  const one = params.get('s');
  main.append(el('h1', { id: 'title', textContent: one ? (site.sources.find((source) => source.id === one) || { title: one }).title : site.title }));
  if (one) main.append(el('a', { href: '#', id: 'front', textContent: 'All', onclick: (event) => { event.preventDefault(); go(''); } }));
  for (const source of site.sources.filter((item) => !one || item.id === one)) {
    if (!one) main.append(el('a', { href: '#', id: 'source-' + source.id, textContent: source.title, onclick: (event) => { event.preventDefault(); go('s=' + source.id); } }));
    const feed = await json('_data/feed/' + source.id + '.json');
    for (const item of feed ? feed.items : []) main.append(el('a', { href: '#', id: 'item-' + item.id, textContent: item.title, onclick: (event) => { event.preventDefault(); go('a=' + item.id); } }));
  }
  tell(one ? { type: 'site.location', route, title: document.getElementById('title').textContent, source: one } : { type: 'site.location', route, title: site.title });
}
function go(next) { route = next; return draw(); }
addEventListener('message', (event) => {
  if (event.source !== parent || !event.data) return;
  if (event.data.type === 'site.navigate') go(String(event.data.route || ''));
  if (event.data.type === 'site.updated') draw();
  if (event.data.type === 'site.theme') document.documentElement.dataset.theme = event.data.theme;
});
await draw();
tell({ type: 'site.ready' });
</script></body></html>`

const ARTICLE = '7eec8758d56a244a'
const YAHOO = 'news-yahoo-co-jp'
const HN = 'news-ycombinator-com'
const SITE_KEYS = ['_data/site.json', `_data/feed/${YAHOO}.json`, `_data/a/${ARTICLE}.json`, 'index.html']
/** The spec put the personal site there itself (the bucket had none), so it may change it and takes it away again. */
let ownsSite = false

/** The shared samples as a site this Agent maintains (its Goal is made up: the jobs are not looked at here). */
async function putSite() {
  const site = sample('site')
  site.owner_did = ownerDid
  site.maintainer = { agent: agentDid, goal_id: 'g-dv-reading-site' }
  await putKey('_data/site.json', JSON.stringify(site), 'application/json')
  await putKey(`_data/feed/${YAHOO}.json`, JSON.stringify(sample('feed')), 'application/json')
  await putKey(`_data/a/${ARTICLE}.json`, JSON.stringify(sample('article')), 'application/json')
  await putKey('index.html', SITE_PAGE, 'text/html; charset=utf-8')
}

test.beforeAll(async ({ playwright }) => {
  api = await playwright.request.newContext({ ignoreHTTPSErrors: true })
  server = await startSite()
  const { agents } = await call<{ agents: Array<{ agent_id: string; agent_did: string; owner_did: string; name: string; display_name?: string | null; profile?: { display_name?: string | null } }> }>('control-panel', 'agent.list', {}, await tokenOf())
  const found = agents.find((item) => item.agent_id === agentId)
  expect(found, `${agentId} is an Agent of ${owner}`).toBeTruthy()
  agentDid = found!.agent_did
  ownerDid = found!.owner_did
  // The App names the maker as the Agent setup does (`agentDisplayName`): its display name, else its name.
  agentName = found!.profile?.display_name?.trim() || found!.display_name?.trim() || found!.name
  await removeTidied()
  ownsSite = (await readKey('_data/site.json')) === null && (await readKey('index.html')) === null
  if (ownsSite) await putSite()
})

test.afterAll(async () => {
  await removeTidied().catch(() => undefined)
  if (ownsSite) for (const key of SITE_KEYS) await removeKey(key).catch(() => undefined)
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

/** Signs the owner in and opens the Agent browser on the Agent, where `link` says (home without one). */
async function openApp(browser: Browser, link = '', options: Parameters<Browser['newContext']>[0] = { viewport: { width: 1280, height: 900 } }): Promise<App> {
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
  await page.goto(`${origin}/`)
  await page.getByLabel('Username', { exact: true }).fill(owner)
  await page.getByLabel('Password', { exact: true }).fill(ownerPassword)
  const signIn = page.getByRole('button', { name: 'Sign In', exact: true })
  await (options.hasTouch ? signIn.tap() : signIn.click())
  await expect(options.hasTouch ? page.getByTestId('mobile-taskbar') : page.getByRole('button', { name: 'BuckyOS' }).first()).toBeAttached({ timeout: 30_000 })
  await page.goto(`${origin}/?agent_browser=${encodeURIComponent(agentId)}${link}`)
  await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-agent', agentId, { timeout: 30_000 })
  return { context, page, sent, letThrough: () => { held = false } }
}

const home = (page: Page) => page.getByTestId('ab-home')
const panel = (page: Page) => page.getByTestId('ab-reflow-panel')
const tidied = (page: Page) => page.frameLocator('[data-testid="ab-reflow-frame"]')
const sitePage = (page: Page) => page.frameLocator('[data-testid="ab-site-frame"]')
const running = async () => (await agent<{ running: boolean }>('browser.status')).running
const shownAddress = (url: string) => url.replace(/^https:\/\//, '').replace(/\/$/, '')

/** An address entered in the bar of whichever mode is on. */
async function go(page: Page, address: string) {
  const input = page.getByTestId('ab-address-input')
  await input.click()
  await input.fill(address)
  await input.press('Enter')
}

/** The address shown at home as the page tidied for it. */
async function shown(page: Page, address: string, title: string) {
  await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-mode', 'home')
  await expect(panel(page)).toHaveAttribute('data-url', address)
  await expect(panel(page)).toHaveAttribute('data-phase', 'view', { timeout: 60_000 })
  await expect(tidied(page).locator('#title')).toContainText(title, { timeout: 30_000 })
}

/** The original page of an address in the Agent's browser, with the screen connected. */
async function original(page: Page, address: string) {
  await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-mode', 'page')
  await expect(page.getByTestId('ab-screen')).toHaveAttribute('data-status', 'open', { timeout: 60_000 })
  await expect(page.getByTestId('ab-address-input')).toHaveValue(shownAddress(address), { timeout: 60_000 })
}

// ── §5.4: the host of the pages ──

test('reflow.<zone> serves the signed-in user their own bucket, and its session opens nothing else', async ({ playwright }) => {
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

    // The callback is good once; the host reads with GET and HEAD only, and nothing but the bucket.
    expect((await api.get(mine.callback, { maxRedirects: 0 })).status()).toBeGreaterThanOrEqual(400)
    expect((await mine.visitor.post(landing)).status()).toBe(405)
    expect((await mine.visitor.get(`${reflowOrigin}/${SITE}/missing.html`)).status()).toBe(404)
    expect((await mine.visitor.get(`${reflowOrigin}/${SITE}/%2e%2e/x`)).status()).toBe(404)

    // Not signed in on this host: no cookie, or the Desktop's own session.
    const stranger = await playwright.request.newContext({ ignoreHTTPSErrors: true })
    const refused = await stranger.get(landing, { maxRedirects: 0, headers: { Accept: 'application/json' } })
    expect(refused.status()).toBe(401)
    expect(refused.headers()['content-type']).toContain('text/plain')
    expect((await stranger.get(landing, { headers: { Accept: 'application/json', Authorization: `Bearer ${await tokenOf()}` } })).status()).toBe(401)
    // A navigation without a session gets the small page that signs in and comes back (Personal Site Protocol §7).
    const navigation = await stranger.get(landing, { maxRedirects: 0, headers: { Accept: 'text/html,application/xhtml+xml' } })
    expect(navigation.status()).toBe(200)
    expect(navigation.headers()['content-type']).toContain('text/html')
    expect(navigation.headers()['cache-control']).toContain('no-store')
    expect(await navigation.text()).not.toContain('<h1>owner</h1>')

    // The page session is no sign-in on the file plane or the bucket.
    const bearer = { Authorization: `Bearer ${mine.session}` }
    expect((await stranger.post(`${origin}/nfs/v1/hello`, { headers: bearer, data: { args: {} } })).status()).toBe(403)
    expect((await stranger.get(keyUrl(owner, key), { headers: bearer })).status()).toBe(403)
    expect((await stranger.put(keyUrl(owner, `${SITE}/x.txt`), { headers: bearer, data: 'x' })).status()).toBe(403)
    const viaCookie = await stranger.post(`${origin}/nfs/v1/hello`, { headers: { Cookie: `buckyos_session_token=${mine.session}` }, data: { args: {} } })
    expect(((await viaCookie.json()) as { result?: { identity?: { role?: string } } }).result?.identity?.role).toBe('anonymous')
    await stranger.dispose()

    // Another user at the same address reads their own bucket.
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
    await removeKey(key)
  }
})


// ── Home needs no browser ──

test('the App opens at home without starting the Agent’s browser; what was tidied shows there, also after the sign-in on the pages’ host ran out', async ({ browser }) => {
  await agent('browser.clear_profile')
  await expect.poll(running).toBe(false)
  const first = await putTidied(story(1), '图书馆（整理版）', [story(2)])
  expect(first.view).toBe(`${SITE}/fallback/${first.page}.html`)
  const app = await openApp(browser)
  const { page, context } = app
  try {
    await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-mode', 'home')
    await expect(home(page)).not.toHaveAttribute('data-status', 'loading', { timeout: 30_000 })
    await expect(page.getByTestId('ab-screen')).toHaveCount(0)

    // An address the owner has a tidied page for shows it; the frame is signed in on the pages' host for it.
    await go(page, story(1))
    await shown(page, story(1), '图书馆（整理版）')
    await expect(page.getByTestId('ab-reflow-frame')).toHaveAttribute('src', new RegExp(`^${reflowOrigin}/sso_callback\\?nonce=`))
    await expect(page.getByTestId('ab-reflow-maker')).toContainText(agentName)
    await expect(page.getByTestId('ab-address-input')).toHaveValue(shownAddress(story(1)))
    expect(await running()).toBe(false)
    expect(app.sent).toEqual([])

    // Signed out of the pages' host altogether (cleared site data, or seven days): the next page signs in again.
    await putTidied(story(2), '河滨步道（整理版）')
    await context.clearCookies({ domain: reflowHost })
    await tidied(page).locator('a').click()
    await shown(page, story(2), '河滨步道（整理版）')
    await page.getByTestId('ab-back').click()
    await shown(page, story(1), '图书馆（整理版）')
    expect(await running()).toBe(false)
  } finally {
    await context.close()
  }
})

// ── A link of a tidied page, the original page ──

test('a link nothing was tidied for and "original page" go to the Agent’s browser; the Agent is not asked', async ({ browser }) => {
  await putTidied(story(2), '河滨步道（整理版）', [story(3)])
  const app = await openApp(browser)
  const { page } = app
  try {
    await go(page, story(2))
    await shown(page, story(2), '河滨步道（整理版）')
    // `reflow.open` of a page the owner has nothing for: the original site, in original page mode.
    await tidied(page).locator('a').click()
    await original(page, story(3))
    expect(await running()).toBe(true)
    expect((await reflowState(story(3))).view).toBeNull()
    // Home kept what was being read; `reflow.source` opens what it was made from.
    await page.getByTestId('ab-mode-home').click()
    await shown(page, story(2), '河滨步道（整理版）')
    await tidied(page).locator('#original').click()
    await original(page, story(2))
    expect(app.sent).toEqual([])
  } finally {
    await app.context.close()
  }
})

// ── "Tidy this page only" ──

test('Reflow on a page the owner has nothing for offers the three choices; "tidy this page only" writes the fixed-format message', async ({ browser }) => {
  const app = await openApp(browser, '&mode=page')
  const { page } = app
  try {
    await expect(page.getByTestId('ab-screen')).toHaveAttribute('data-status', 'open', { timeout: 60_000 })
    await go(page, story(3))
    await original(page, story(3))
    await page.getByTestId('ab-reflow-button').click()
    // The page is an article without a form: `browser.page_hint` says what to offer first, and never a task here.
    await expect(page.getByTestId('ab-reflow-choices')).toHaveAttribute('data-hint', /^(reading|unknown)$/)
    await expect(page.getByTestId('ab-reflow-choice-add')).toHaveAttribute('data-first', 'true')
    await page.getByTestId('ab-reflow-choice-tidy').click()
    await page.getByTestId('ab-tidy-note').fill('只要正文')
    await page.getByTestId('ab-tidy-confirm').click()
    await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-mode', 'home')
    await expect.poll(() => app.sent.length, { timeout: 30_000 }).toBe(1)
    const lines = app.sent[0].split('\n')
    expect(lines[0]).toBe('Please tidy this page only: make a one-off reflow page for it (reflow result put), without adding the website to my reading site.')
    expect(lines[1]).toBe(`URL: ${story(3)}`)
    expect(lines.at(-1)).toBe('Note: 只要正文')
    // The message was held back, so the page says the Agent could not be asked.
    await expect(page.getByTestId('ab-reflow-unmade')).toHaveAttribute('data-reason', 'agent_unreachable', { timeout: 30_000 })

    // The Agent's page is there (put by the spec): Reflow on the original page now opens it at home, without asking.
    await putTidied(story(3), '菜市场（整理版）')
    await page.getByTestId('ab-mode-page').click()
    await original(page, story(3))
    await page.getByTestId('ab-reflow-button').click()
    await shown(page, story(3), '菜市场（整理版）')
    expect(app.sent).toHaveLength(1)
  } finally {
    await app.context.close()
  }
})

test('the Agent makes the page that was asked for, and the App shows it', async ({ browser }) => {
  test.skip(!askAgent, 'wakes the real model: set BUCKYOS_TEST_REFLOW_ASK_AGENT=1')
  test.setTimeout(12 * 60_000)
  const address = `${siteOrigin}/`
  const app = await openApp(browser, '&mode=page')
  const { page } = app
  try {
    app.letThrough()
    await expect(page.getByTestId('ab-screen')).toHaveAttribute('data-status', 'open', { timeout: 60_000 })
    await go(page, address)
    await original(page, address)
    await page.getByTestId('ab-reflow-button').click()
    await page.getByTestId('ab-reflow-choice-tidy').click()
    await page.getByTestId('ab-tidy-confirm').click()
    await expect(page.getByTestId('ab-reflow-making')).toBeVisible({ timeout: 30_000 })
    await expect.poll(() => app.sent.length).toBe(1)
    expect(app.sent[0]).toContain(`URL: ${address}`)
    await expect(panel(page)).toHaveAttribute('data-phase', 'view', { timeout: 10 * 60_000 })
    await expect(page.getByTestId('ab-reflow-frame')).toHaveAttribute('data-drawn', 'true', { timeout: 30_000 })
    const state = await reflowState(address)
    expect(state.view).toBe(`${SITE}/fallback/${state.page}.html`)
    await expect(tidied(page).locator('body')).toContainText(stories[1].title)
    expect((await keysOf(`${SITE}/fallback/`)).sort()).toEqual([`${SITE}/fallback/${state.page}.html`, `${SITE}/fallback/${state.page}.json`])
  } finally {
    await app.context.close()
  }
})

// ── The personal site ──

test('the personal site is read at home: sources, an article, a link that is not collected, the original page', async ({ browser }) => {
  test.skip(!ownsSite, 'the owner has a personal site already: the spec does not put its samples over it')
  await agent('browser.clear_profile')
  await expect.poll(running).toBe(false)
  const app = await openApp(browser)
  const { page } = app
  try {
    await expect(home(page)).toHaveAttribute('data-status', 'ready', { timeout: 30_000 })
    await expect(page.getByTestId('ab-site-frame')).toHaveAttribute('src', new RegExp(`^${reflowOrigin}/sso_callback\\?nonce=`))
    await expect(page.getByTestId('ab-site-frame')).toHaveAttribute('data-drawn', 'true', { timeout: 30_000 })
    await expect(sitePage(page).locator('#title')).toHaveText('我的阅读')
    // The shell's strip is read from `_data` by the App; the page was told the theme before it showed.
    await expect(page.getByTestId('ab-site-title')).toHaveText('我的阅读')
    await expect(page.getByTestId('ab-site-status')).toHaveText('Websites: 3')
    await expect(sitePage(page).locator('html')).toHaveAttribute('data-theme', /^(light|dark)$/)

    await sitePage(page).locator(`#source-${YAHOO}`).click()
    await expect(home(page)).toHaveAttribute('data-route', `s=${YAHOO}`)
    await expect(page.getByTestId('ab-site-strip')).toHaveAttribute('data-state', 'ok')
    await expect(page.getByTestId('ab-site-status')).toContainText('Synced')
    await sitePage(page).locator(`#item-${ARTICLE}`).click()
    await expect(home(page)).toHaveAttribute('data-route', `a=${ARTICLE}`)
    await expect(page.getByTestId('ab-address-input')).toHaveValue('news.yahoo.co.jp/articles/0a1b2c3d4e5f60718293a4b5c6d7e8f901234567')
    await page.getByTestId('ab-back').click()
    await expect(home(page)).toHaveAttribute('data-route', `s=${YAHOO}`)
    await expect(sitePage(page).locator('body')).toHaveAttribute('data-route', `s=${YAHOO}`)
    // Reading needed neither the browser nor the Agent.
    expect(await running()).toBe(false)
    expect(app.sent).toEqual([])

    // A link that is not collected is the original site's: only now the browser starts.
    await sitePage(page).locator(`#item-${ARTICLE}`).click()
    await sitePage(page).locator('a.link').last().click()
    await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-mode', 'page')
    await expect(page.getByTestId('ab-screen')).toHaveAttribute('data-status', 'open', { timeout: 60_000 })
    expect(app.sent).toEqual([])
  } finally {
    await app.context.close()
  }
})

test('an upgrade request opens from its link; the answers are written into site.json as it is, and "Upgrade" is the fixed-format message', async ({ browser }) => {
  test.skip(!ownsSite, 'the owner has a personal site already: the spec does not put its samples over it')
  const app = await openApp(browser, '&mode=home&upgrade=hn-comments-layout')
  const { page } = app
  try {
    await expect(home(page)).toHaveAttribute('data-route', `s=${HN}`, { timeout: 30_000 })
    const sheet = page.getByTestId('ab-upgrade-sheet')
    await expect(sheet).toContainText('Hacker News 的评论页改版了')
    // "Upgrade": the message is held back here, so nothing is recorded and the request stays open.
    await page.getByTestId('ab-upgrade-accept').click()
    await expect.poll(() => app.sent.length, { timeout: 30_000 }).toBe(1)
    expect(app.sent[0].split('\n')).toEqual([
      'Please go ahead with this upgrade of my reading site.',
      'Upgrade: hn-comments-layout',
      'File: upgrades/2026-11-24-hn-comments-layout.md',
      'Goal: g-20261123T020304-9c8d7e6f',
    ])
    await expect(page.getByTestId('ab-site-notice')).toHaveAttribute('data-kind', 'upgrade_unsent')
    const kept = JSON.parse((await readKey('_data/site.json'))!) as { upgrades: Array<{ id: string; state: string }> }
    expect(kept.upgrades.find((upgrade) => upgrade.id === 'hn-comments-layout')?.state).toBe('open')

    // "Not now" writes `dismissed` with `If-Match`: what somebody else wrote meanwhile is kept.
    const meanwhile = JSON.parse((await readKey('_data/site.json'))!) as JsonRecord
    meanwhile.title = '晨读'
    await putKey('_data/site.json', JSON.stringify(meanwhile), 'application/json')
    const before = Date.now()
    await page.getByTestId('ab-upgrade-dismiss').click()
    await expect(sheet).toHaveCount(0, { timeout: 30_000 })
    const now = JSON.parse((await readKey('_data/site.json'))!) as { title: string; updated_by: string; upgrades: Array<{ id: string; state: string; decided_at?: number }> }
    expect(now.title).toBe('晨读')
    expect(now.updated_by).toBe(ownerDid)
    const decided = now.upgrades.find((upgrade) => upgrade.id === 'hn-comments-layout')!
    expect(decided.state).toBe('dismissed')
    expect(decided.decided_at).toBeGreaterThanOrEqual(before - 60_000)
    await expect(page.getByTestId('ab-site-upgrade')).toHaveCount(0)
  } finally {
    await app.context.close()
  }
})

// ── A phone ──

test('on a phone home fits: the strip, the page, and a tidied page with the way back to the original', async ({ browser }) => {
  await putTidied(story(1), '图书馆（整理版）')
  const app = await openApp(browser, '', { ...devices['Pixel 7'] })
  const { page } = app
  try {
    const width = page.viewportSize()!.width
    const fits = async (testId: string) => {
      const box = await page.getByTestId(testId).boundingBox()
      expect(box, testId).not.toBeNull()
      expect(box!.x, testId).toBeGreaterThanOrEqual(0)
      expect(box!.x + box!.width, testId).toBeLessThanOrEqual(width)
    }
    await expect(home(page)).not.toHaveAttribute('data-status', 'loading', { timeout: 30_000 })
    await fits('ab-address')
    await fits('ab-status')
    if (ownsSite) {
      await expect(page.getByTestId('ab-site-frame')).toHaveAttribute('data-drawn', 'true', { timeout: 30_000 })
      for (const testId of ['ab-site-strip', 'ab-site-frame']) await fits(testId)
      // "Manage" is one tap away and fits as well; its jobs belong to a real Goal, which this spec does not have.
      await page.getByTestId('ab-site-manage').tap()
      await expect(page.getByTestId('ab-manage')).toHaveAttribute('data-tab', 'sources')
      await fits('ab-source-list')
      await page.getByTestId('ab-manage-back').tap()
    }
    const input = page.getByTestId('ab-address-input')
    await input.tap()
    await input.fill(story(1))
    await input.press('Enter')
    await shown(page, story(1), '图书馆（整理版）')
    for (const testId of ['ab-reflow-strip', 'ab-reflow-made', 'ab-reflow-source', 'ab-reflow-frame']) await fits(testId)
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(width)
    await page.getByTestId('ab-reflow-source').tap()
    await original(page, story(1))
  } finally {
    await app.context.close()
  }
})
