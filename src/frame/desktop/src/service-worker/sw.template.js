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
const INDEX = new URL('index.html', SCOPE).href
const ASSETS = new Map(PRECACHE.map((path) => [new URL(path, SCOPE).href, path]))
const NAVIGATION_TIMEOUT_MS = 4000
const OFFLINE_ROUTES = ['workspace']

function isEntry(pathname) {
  if (!pathname.startsWith(SCOPE.pathname)) return false
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
      // a static server with an index.html fallback answers a missing file with the page itself
      if (path !== 'index.html' && /text\/html/i.test(response.headers.get('content-type') || '')) throw new Error(`${path}: got an HTML page instead of the file`)
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

/** The entry cannot be served: say so instead of a blank page or a half-loaded application. */
function unavailablePage(reason) {
  const body = `<!doctype html><html lang="zh"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>BuckyOS — 离线不可用</title></head>
<body style="font:14px/1.7 system-ui,sans-serif;max-width:36rem;margin:15vh auto 0;padding:1.5rem">
<div role="alert" data-testid="desktop-offline-unavailable">
<p style="font-size:1.25rem;font-weight:600;margin:0 0 .5rem">无法离线启动 BuckyOS Desktop</p>
<p>连接不到服务器，而本机缓存的应用资源不完整：${escapeHtml(reason)}。</p>
<p>缓存可能被浏览器清理过，或者这个版本从未在联网时完整载入。本机保存的离线数据没有被改动；请在恢复网络连接后重新载入。</p>
<p style="color:#666">BuckyOS Desktop cannot start offline: the application resources cached on this device are incomplete. Reconnect and reload. Offline data stored on this device was not touched.</p>
<button type="button" onclick="location.reload()" style="padding:.5rem 1rem;font:inherit;cursor:pointer">重新载入 / Reload</button>
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
  if (!cached) return unavailablePage('入口页面 index.html 不在缓存中')
  // The page is only useful with its scripts, styles and WASM modules: check before starting it.
  const have = new Set((await cache.keys()).map((entry) => entry.url))
  const missing = [...ASSETS.keys()].filter((url) => !have.has(url))
  if (missing.length > 0) return unavailablePage(`缺少 ${missing.length} 个文件（例如 ${ASSETS.get(missing[0])}）`)
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
