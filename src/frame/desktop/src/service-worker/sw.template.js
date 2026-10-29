/* BuckyOS Desktop service worker — application resources only (first-phase document §7 item 2).
 *
 * Generated into `dist/sw.js` by the `desktop-service-worker` plugin of vite.config.ts, which fills
 * in the version and the list of built files. It lives next to index.html, so its scope is the
 * directory the Desktop is served from and no `Service-Worker-Allowed` header is needed (the
 * control-panel directory handler sends no custom headers).
 *
 * What it does, and nothing else:
 *   - install: download every built file into one versioned cache (all or nothing);
 *   - a navigation to the Desktop entry or to an app route that works offline (`workspace/…`): network
 *     first (so a new deployment is picked up at once), the cached index.html when the network is not there;
 *     on a Workspace's own origin (`aiws-<id>.<zone>`, where the same build's aiws.html is the whole site) every
 *     navigation is such an entry and the cached aiws.html stands in;
 *   - a built file: cache first;
 *   - everything else (kRPC, uploads, other paths, other origins, non-GET) is not touched.
 * It never stores user data: the offline replica lives in OPFS, managed by the app.
 *
 * Update: file names are content-hashed and the version is a hash of all files, so a new build is a
 * new cache. The new worker installs in the background and takes over when the old pages are gone,
 * or at once when a page posts { type: 'SKIP_WAITING' }. Old caches are deleted on activation. */

const VERSION = '__SW_VERSION__'
const PRECACHE = __SW_PRECACHE__
const PREFIX = 'buckyos-desktop-'
const CACHE = PREFIX + VERSION
const SCOPE = new URL(self.registration.scope)
const WORKSPACE_HOST = /^aiws-[a-z0-9]+\./.test(SCOPE.hostname)
const ENTRY = WORKSPACE_HOST ? 'aiws.html' : 'index.html'
const INDEX = new URL(ENTRY, SCOPE).href
const ASSETS = new Map(PRECACHE.map((path) => [new URL(path, SCOPE).href, path]))
const NAVIGATION_TIMEOUT_MS = 4000
const OFFLINE_ROUTES = ['workspace']

function isEntry(pathname) {
  if (!pathname.startsWith(SCOPE.pathname)) return false
  if (WORKSPACE_HOST) return true
  const rest = pathname.slice(SCOPE.pathname.length)
  return rest === '' || rest === 'index.html' || OFFLINE_ROUTES.some((route) => rest === route || rest.startsWith(route + '/'))
}

async function precache() {
  const cache = await caches.open(CACHE)
  const pending = [...ASSETS.entries()]
  const worker = async () => {
    for (;;) {
      const next = pending.pop()
      if (!next) return
      const [url, path] = next
      const response = await fetch(new Request(url, { cache: 'reload' }))
      if (!response.ok) throw new Error(`${path}: HTTP ${response.status}`)
      // a static server with an index.html fallback answers a missing file with the page itself; built pages
      // (index.html, an app's own .html) are HTML
      if (!/\.html?$/i.test(path) && /text\/html/i.test(response.headers.get('content-type') || '')) throw new Error(`${path}: got an HTML page instead of the file`)
      await cache.put(url, response)
    }
  }
  await Promise.all(Array.from({ length: 6 }, worker))
}

self.addEventListener('install', (event) => {
  event.waitUntil(precache())
})

self.addEventListener('activate', (event) => {
  event.waitUntil((async () => {
    for (const name of await caches.keys()) if (name.startsWith(PREFIX) && name !== CACHE) await caches.delete(name)
    await self.clients.claim()
  })())
})

self.addEventListener('message', (event) => {
  const data = event.data || {}
  if (data.type === 'SKIP_WAITING') self.skipWaiting()
  if (data.type === 'GET_VERSION' && event.ports[0]) event.ports[0].postMessage({ version: VERSION, files: PRECACHE.length })
})

function escapeHtml(text) {
  return String(text).replace(/[&<>"]/g, (ch) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[ch]))
}

/** The entry cannot be served: say so instead of a blank page or a half-loaded application. The worker cannot read
 * the Desktop's language setting, so the page says it in Chinese and English, the browser's language first. */
function unavailablePage(reason) {
  const zh = `<p style="font-size:1.25rem;font-weight:600;margin:0 0 .5rem">无法离线启动 BuckyOS Desktop</p>
<p>连接不到服务器，而本机缓存的应用资源不完整：${escapeHtml(reason.zh)}。</p>
<p>缓存可能被浏览器清理过，或者这个版本从未在联网时完整载入。本机保存的离线数据没有被改动；请在恢复网络连接后重新载入。</p>`
  const en = `<p style="font-size:1.25rem;font-weight:600;margin:0 0 .5rem">BuckyOS Desktop cannot start offline</p>
<p>The server cannot be reached, and the application resources cached on this device are incomplete: ${escapeHtml(reason.en)}.</p>
<p>The browser may have cleared the cache, or this version was never fully loaded while online. Offline data stored on this device was not touched; reload once you are connected again.</p>`
  const chineseFirst = /^zh\b/i.test(self.navigator.language || '')
  const [first, second] = chineseFirst ? [zh, en] : [en, zh]
  const body = `<!doctype html><html lang="${chineseFirst ? 'zh-CN' : 'en'}"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>BuckyOS — ${chineseFirst ? '离线不可用' : 'Unavailable offline'}</title></head>
<body style="font:14px/1.7 system-ui,sans-serif;max-width:36rem;margin:15vh auto 0;padding:1.5rem">
<div role="alert" data-testid="desktop-offline-unavailable">
${first}
<div style="color:#666">${second}</div>
<button type="button" onclick="location.reload()" style="padding:.5rem 1rem;font:inherit;cursor:pointer">${chineseFirst ? '重新载入 / Reload' : 'Reload / 重新载入'}</button>
</div></body></html>`
  return new Response(body, { status: 503, statusText: 'Offline resources unavailable', headers: { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-store' } })
}

async function navigation(request) {
  const controller = new AbortController()
  const timer = setTimeout(() => controller.abort(), NAVIGATION_TIMEOUT_MS)
  try {
    const response = await fetch(request, { signal: controller.signal })
    // a gateway error page is not the application: fall back to the cached entry
    if (response.status < 500) return response
  } catch (error) {
    // no network: the cache decides
  } finally {
    clearTimeout(timer)
  }
  const cache = await caches.open(CACHE)
  const cached = await cache.match(INDEX)
  if (!cached) return unavailablePage({ zh: `入口页面 ${ENTRY} 不在缓存中`, en: `the entry page ${ENTRY} is not in the cache` })
  // The page is only useful with its scripts, styles and WASM modules: check before starting it.
  const have = new Set((await cache.keys()).map((entry) => entry.url))
  const missing = [...ASSETS.keys()].filter((url) => !have.has(url))
  if (missing.length > 0) return unavailablePage({ zh: `缺少 ${missing.length} 个文件（例如 ${ASSETS.get(missing[0])}）`, en: `${missing.length} file(s) missing (e.g. ${ASSETS.get(missing[0])})` })
  return cached
}

async function asset(url, request) {
  const cache = await caches.open(CACHE)
  const cached = await cache.match(url)
  if (cached) return cached
  try {
    return await fetch(request)
  } catch (error) {
    return new Response('', { status: 504, statusText: 'Offline: resource is not cached' })
  }
}

self.addEventListener('fetch', (event) => {
  const request = event.request
  if (request.method !== 'GET') return
  const url = new URL(request.url)
  if (url.origin !== SCOPE.origin) return
  if (request.mode === 'navigate') {
    if (isEntry(url.pathname)) event.respondWith(navigation(request))
    return
  }
  const key = url.origin + url.pathname
  if (ASSETS.has(key)) event.respondWith(asset(key, request))
})
