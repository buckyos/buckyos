/* Registration of the Desktop service worker (application resources for a cold start without the
 * network; see src/service-worker/sw.template.js). Production builds only: the Vite dev server has
 * no stable file list. `sw.js` sits next to index.html, so the scope is the Desktop's own directory. */

export const SW_UPDATE_EVENT = 'buckyos-desktop-sw-update'

/** Why application resources are not cached for offline start (the AI Workspace states it in its own language). */
export type ServiceWorkerUnavailable = 'not_registered' | 'dev_server' | 'insecure_context' | 'unsupported' | 'incomplete' | 'registration_failed'

export interface ServiceWorkerState {
  /** Why application resources are not cached for offline start, or null when a worker controls or is installing. */
  unavailable: ServiceWorkerUnavailable | null
  /** The browser's error, for `registration_failed`. */
  error?: string
  /** A newer build is installed and waiting for the old pages to close. */
  updateWaiting: boolean
}

const state: ServiceWorkerState = { unavailable: 'not_registered', updateWaiting: false }
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
  if (!import.meta.env.PROD) { state.unavailable = 'dev_server'; return }
  if (!window.isSecureContext) { state.unavailable = 'insecure_context'; return }
  if (!('serviceWorker' in navigator)) { state.unavailable = 'unsupported'; return }
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
        if (worker.state === 'redundant' && !navigator.serviceWorker.controller) { state.unavailable = 'incomplete'; announce() }
      })
    }
    if (result.waiting && navigator.serviceWorker.controller) state.updateWaiting = true
    watch(result.installing)
    result.addEventListener('updatefound', () => watch(result.installing))
    announce()
  }, (error: unknown) => {
    state.unavailable = 'registration_failed'
    state.error = error instanceof Error ? error.message : String(error)
    announce()
  })
}
