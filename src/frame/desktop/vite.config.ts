import { createHash } from 'node:crypto'
import { readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs'
import https from 'node:https'
import { join, relative, resolve, sep } from 'node:path'
import { defineConfig, type Plugin } from 'vite'
import react from '@vitejs/plugin-react'

// Desktop service worker (application resources for a cold start without the network): after the
// bundle is written, list every built file and generate `sw.js` next to index.html from
// src/service-worker/sw.template.js. The version is a hash of all file contents, so any change of
// the build is a new cache. See src/serviceWorker.ts for the registration.
function desktopServiceWorker(): Plugin {
  let outDir = 'dist'
  return {
    name: 'desktop-service-worker',
    apply: 'build',
    configResolved(config) { outDir = resolve(config.root, config.build.outDir) },
    closeBundle() {
      const files: string[] = []
      const walk = (dir: string) => {
        for (const name of readdirSync(dir)) {
          const path = join(dir, name)
          if (statSync(path).isDirectory()) walk(path)
          else files.push(relative(outDir, path).split(sep).join('/'))
        }
      }
      try { walk(outDir) } catch { return } // the build failed before writing anything
      const precache = files.filter((file) => file !== 'sw.js' && !file.endsWith('.map')).sort()
      if (!precache.includes('index.html')) return
      const hash = createHash('sha256')
      for (const file of precache) hash.update(file).update('\0').update(readFileSync(join(outDir, file)))
      const template = readFileSync(resolve(import.meta.dirname, 'src/service-worker/sw.template.js'), 'utf8')
      writeFileSync(join(outDir, 'sw.js'), template.replace('__SW_VERSION__', hash.digest('hex').slice(0, 16)).replace('__SW_PRECACHE__', JSON.stringify(precache)))
    },
  }
}

// NFSP dev proxy: nfs-server has no CORS (in production the zone gateway
// forwards the same-origin root path /nfs/v1/* to it), so dev mirrors that
// shape by proxying. Point VITE_NFS_PROXY at a running nfs_server (e.g.
// http://127.0.0.1:3260 standalone, or http://127.0.0.1:4110 buckyos mode)
// and open the app with ?fbData=nfsp to run the File Browser on the real
// backend from `pnpm run dev`.
// Zone dev proxy: point VITE_ZONE_PROXY at the zone root origin (e.g.
// https://test.buckyos.io for the DV zone) and start the desktop with
// VITE_CP_USE_MOCK=false. Every kRPC / SSO / NDM call then stays same-origin
// on the dev server and is forwarded to the gateway with the zone's Host and
// TLS server name (the control panel only issues system session tokens to the
// zone root origin); cookies are re-scoped to the dev origin so the SSO
// refresh flow keeps working. VITE_ZONE_PROXY_IP pins the gateway address
// when the zone host does not resolve to it locally (DV zones use /etc/hosts).
// AI Workspace dev proxy: the aiworkspace service sends no CORS headers (in
// production the zone gateway serves /kapi/aiworkspace same-origin). Point
// AIWS_BACKEND at a standalone backend (e.g. http://127.0.0.1:4120) to forward
// /kapi/aiworkspace to it; unset, nothing changes. See src/app/aiworkspace/README.md.
export default defineConfig(() => {
  const nfsTarget = process.env.VITE_NFS_PROXY
  const zoneTarget = process.env.VITE_ZONE_PROXY
  const zoneIp = process.env.VITE_ZONE_PROXY_IP
  const aiwsTarget = process.env.AIWS_BACKEND
  const zoneAgent = zoneTarget
    ? new https.Agent({
        rejectUnauthorized: false,
        keepAlive: false,
        lookup: zoneIp
          ? ((_host, options, callback) => {
              const family = zoneIp.includes(':') ? 6 : 4
              if (typeof options === 'object' && options.all) callback(null, [{ address: zoneIp, family }])
              else callback(null, zoneIp, family)
            }) as https.AgentOptions['lookup']
          : undefined,
      })
    : undefined
  const zoneProxy = zoneTarget
    ? Object.fromEntries(['/kapi', '/sso_callback', '/sso_refresh', '/sso_logout', '/ndm', '/api'].map((path) => [path, {
        target: zoneTarget,
        changeOrigin: true,
        secure: false,
        agent: zoneAgent,
        cookieDomainRewrite: { '*': '' },
      }]))
    : {}
  return {
    base: './',
    plugins: [react(), desktopServiceWorker()],
    // The Replica Worker (src/app/aiworkspace/offline/replica.worker.ts) is a module worker.
    worker: { format: 'es' as const },
    // sqlite-wasm locates its .wasm relative to its own module; pre-bundling would move the module.
    optimizeDeps: { exclude: ['@sqlite.org/sqlite-wasm'] },
    resolve: {
      // loro-crdt's default browser entry imports its .wasm as an ES module, which Vite does not
      // support without an extra plugin. The `web` build is the same code with an explicit init()
      // (called in src/app/aiworkspace/richtext/loro.ts); loro-prosemirror must see the same instance.
      alias: [{ find: /^loro-crdt$/, replacement: 'loro-crdt/web' }],
    },
    server: {
      host: '0.0.0.0',
      port: 5174,
      proxy: {
        // listed first: more specific than the zone proxy's /kapi
        ...(aiwsTarget ? { '/kapi/aiworkspace': { target: aiwsTarget, changeOrigin: true } } : {}),
        ...(nfsTarget ? { '/nfs/v1': { target: nfsTarget, changeOrigin: true } } : {}),
        ...zoneProxy,
      },
    },
    // `vite preview` serves the production build (with its service worker); the AI Workspace e2e
    // suite uses it with the same backend forward as the dev server.
    preview: {
      proxy: {
        ...(aiwsTarget ? { '/kapi/aiworkspace': { target: aiwsTarget, changeOrigin: true } } : {}),
      },
    },
  }
})
