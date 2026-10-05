/* Registration of the Desktop service worker (application resources for a cold start without the
 * network; see src/service-worker/sw.template.js). Production builds only: the Vite dev server has
 * no stable file list. `sw.js` sits next to index.html, so the scope is the Desktop's own directory. */

export const SW_UPDATE_EVENT = 'buckyos-desktop-sw-update'

export interface ServiceWorkerState {
  /** Why application resources are not cached for offline start, or null when a worker controls or is installing. */
  unavailable: string | null
  /** A newer build is installed and waiting for the old pages to close. */
  updateWaiting: boolean
}

const state: ServiceWorkerState = { unavailable: '尚未注册', updateWaiting: false }
let registration: ServiceWorkerRegistration | null = null

export function serviceWorkerState(): ServiceWorkerState {
  return { ...state }
}

function announce() {
  window.dispatchEvent(new CustomEvent(SW_UPDATE_EVENT))
}

/** Let the waiting worker take over now; the page reloads when it does. */
export function applyServiceWorkerUpdate() {
  registration?.waiting?.postMessage({ type: 'SKIP_WAITING' })
}

export function registerDesktopServiceWorker() {
  if (!import.meta.env.PROD) { state.unavailable = '开发服务器不缓存应用资源（仅生产构建注册 Service Worker）'; return }
  if (!window.isSecureContext) { state.unavailable = '当前页面不是安全上下文（需要 HTTPS 或 localhost）'; return }
  if (!('serviceWorker' in navigator)) { state.unavailable = '此浏览器不支持 Service Worker'; return }
  // The entry script is `<scope>/assets/<name>.js`; the worker is `<scope>/sw.js`. (Not written as
  // `new URL('../sw.js', import.meta.url)`: the bundler would take that for an asset reference.)
  const entry = new URL(import.meta.url)
  const script = new URL('../sw.js', entry.href)
  let reloading = false
  navigator.serviceWorker.addEventListener('controllerchange', () => {
    // only an explicit update replaces a controller; the first installation just starts controlling
    if (!state.updateWaiting || reloading) return
    reloading = true
    window.location.reload()
  })
  navigator.serviceWorker.register(script.href).then((result) => {
    registration = result
    state.unavailable = null
    const watch = (worker: ServiceWorker | null) => {
      if (!worker) return
      worker.addEventListener('statechange', () => {
        if (worker.state === 'installed' && navigator.serviceWorker.controller) { state.updateWaiting = true; announce() }
        if (worker.state === 'redundant' && !navigator.serviceWorker.controller) { state.unavailable = '应用资源没有完整下载，离线启动不可用（下次联网载入时会重试）'; announce() }
      })
    }
    if (result.waiting && navigator.serviceWorker.controller) state.updateWaiting = true
    watch(result.installing)
    result.addEventListener('updatefound', () => watch(result.installing))
    announce()
  }, (error: unknown) => {
    state.unavailable = `Service Worker 注册失败：${error instanceof Error ? error.message : String(error)}`
    announce()
  })
}
