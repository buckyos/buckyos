import { createHash } from 'node:crypto'
import { readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs'
import https from 'node:https'
import { join, relative, resolve, sep } from 'node:path'
import { defineConfig, searchForWorkspaceRoot, type Plugin } from 'vite'
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
      const precache = files.filter((file) => file !== 'sw.js' && !file.endsWith('.map') && !file.startsWith(PDFJS_PREFIX)).sort()
      if (!precache.includes('index.html')) return
      const hash = createHash('sha256')
      for (const file of precache) hash.update(file).update('\0').update(readFileSync(join(outDir, file)))
      const template = readFileSync(resolve(import.meta.dirname, 'src/service-worker/sw.template.js'), 'utf8')
      writeFileSync(join(outDir, 'sw.js'), template.replace('__SW_VERSION__', hash.digest('hex').slice(0, 16)).replace('__SW_PRECACHE__', JSON.stringify(precache)))
    },
  }
}

// pdf.js's CMaps (CJK text of older PDFs) and standard font data (PDFs that do not embed the 14 base fonts), served
// next to the app under aiworkspace/pdfjs/ and fetched only by pages that need them (PDF 导入方案 §2): never precached.
const PDFJS_ASSET_DIRS = ['cmaps', 'standard_fonts']
const PDFJS_PREFIX = 'aiworkspace/pdfjs/'
function pdfjsAssets(): Plugin {
  const root = resolve(import.meta.dirname, 'node_modules/pdfjs-dist')
  const types: Record<string, string> = { '.bcmap': 'application/octet-stream', '.pfb': 'application/octet-stream', '.ttf': 'font/ttf' }
  return {
    name: 'aiworkspace-pdfjs-assets',
    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        const path = (req.url ?? '').split('?')[0]
        const m = new RegExp(`^/${PDFJS_PREFIX}(${PDFJS_ASSET_DIRS.join('|')})/([\\w.-]+)$`).exec(path)
        if (!m) return next()
        try {
          const body = readFileSync(join(root, m[1], m[2]))
          res.setHeader('Content-Type', m[2].startsWith('LICENSE') ? 'text/plain' : types[m[2].slice(m[2].lastIndexOf('.'))] ?? 'application/octet-stream')
          res.end(body)
        } catch { res.statusCode = 404; res.end() }
      })
    },
    generateBundle() {
      for (const dir of PDFJS_ASSET_DIRS) {
        for (const name of readdirSync(join(root, dir))) {
          // the data files and their licences (Adobe's CMaps, Foxit's and Liberation fonts)
          if (!/\.(bcmap|pfb|ttf)$|^LICENSE/.test(name)) continue
          this.emitFile({ type: 'asset', fileName: `${PDFJS_PREFIX}${dir}/${name}`, source: readFileSync(join(root, dir, name)) })
        }
      }
    },
  }
}

// The AI Workspace page of a Workspace's own origin (`aiws-<id>.<zone>`, 内核架构改进 T4-8): the second entry of this
// build, `aiws.html`, which the aiworkspace service serves there. The dev and preview servers do the same for a
// request whose Host is such an origin (`aiws-<label>.localhost` resolves to loopback in browsers): a navigation
// gets aiws.html, files and /kapi are untouched.
const WORKSPACE_HOST = /^aiws-[a-z0-9]+\./
function aiwsHostPages(): Plugin {
  const rewrite = (req: { headers: { host?: string; accept?: string }; method?: string; url?: string }, _res: unknown, next: () => void) => {
    const path = (req.url ?? '').split('?')[0]
    if (req.method === 'GET' && WORKSPACE_HOST.test(req.headers.host ?? '') && /text\/html/.test(req.headers.accept ?? '') && !path.startsWith('/kapi/') && !/\.[a-z0-9]+$/i.test(path)) {
      req.url = '/aiws.html'
    }
    next()
  }
  return {
    name: 'aiworkspace-host-pages',
    configureServer(server) { server.middlewares.use(rewrite) },
    configurePreviewServer(server) { server.middlewares.use(rewrite) },
  }
}

const DRAWIO_DIR = resolve(import.meta.dirname, '../aiworkspace/import-drawio')
// The PDF importer shared with Deno and Agents (PDF 导入方案 §4): pdf.js page data → the draw.io importer's plans.
const PDF_DIR = resolve(import.meta.dirname, '../aiworkspace/import-pdf')
// The table importer shared with the Deno CLI (表格数据导入 TODO §2.1): pasted tables, CSV, xlsx, SQLite → TableSource plans.
const TABLE_DIR = resolve(import.meta.dirname, '../aiworkspace/import-table')
// The Block-controls shared with Deno and Agents (脑图 TODO M1): layout, measuring, outline conversion, catalogue.
const CONTROLS_DIR = resolve(import.meta.dirname, '../aiworkspace/controls')
// The wish program host (src/frame/aiworkspace/aiws/aiws.js) the local executor's Worker runs programs with: the file the service runs.
const AIWS_DIR = resolve(import.meta.dirname, '../aiworkspace/aiws')

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
// HomeStation dev proxy: HS_BACKEND (e.g. http://127.0.0.1:4131, the `devnet` example's `me`
// node) forwards /kapi/homestation and the zone-level protocol paths /home/* to it. See
// src/app/homestation/UI_DATAMODEL.md §11.
export default defineConfig(() => {
  const nfsTarget = process.env.VITE_NFS_PROXY
  const zoneTarget = process.env.VITE_ZONE_PROXY
  const zoneIp = process.env.VITE_ZONE_PROXY_IP
  const aiwsTarget = process.env.AIWS_BACKEND
  const hsTarget = process.env.HS_BACKEND
  const hsProxy = hsTarget
    ? Object.fromEntries(['/kapi/homestation', '^/home/'].map((path) => [path, { target: hsTarget, changeOrigin: true }]))
    : {}
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
    ? Object.fromEntries(['/kapi', '/sso_callback', '/sso_refresh', '/sso_logout', '/ndm', '/api', '/1.0/identifiers'].map((path) => [path, {
        target: zoneTarget,
        changeOrigin: true,
        secure: false,
        agent: zoneAgent,
        cookieDomainRewrite: { '*': '' },
      }]))
    : {}
  return {
    base: '/',
    plugins: [react(), pdfjsAssets(), aiwsHostPages(), desktopServiceWorker()],
    // two pages of one build: the Desktop (index.html) and a Workspace's own page (aiws.html), sharing their chunks
    build: { rollupOptions: { input: { index: resolve(import.meta.dirname, 'index.html'), aiws: resolve(import.meta.dirname, 'aiws.html') } } },
    // The Replica Worker (src/app/aiworkspace/offline/replica.worker.ts) is a module worker.
    worker: { format: 'es' as const },
    // sqlite-wasm locates its .wasm relative to its own module; pre-bundling would move the module.
    optimizeDeps: { exclude: ['@sqlite.org/sqlite-wasm'] },
    resolve: {
      // loro-crdt's default browser entry imports its .wasm as an ES module, which Vite does not
      // support without an extra plugin. The `web` build is the same code with an explicit init()
      // (called in src/app/aiworkspace/richtext/loro.ts); loro-prosemirror must see the same instance.
      // The draw.io importer and the canvas geometry it shares live outside the app (they also run in Deno):
      // src/frame/aiworkspace/import-drawio (draw.io 导入 TODO §3).
      alias: [{ find: /^loro-crdt$/, replacement: 'loro-crdt/web' }, { find: /^@drawio\//, replacement: `${DRAWIO_DIR}/` }, { find: /^@pdf\//, replacement: `${PDF_DIR}/` }, { find: /^@table\//, replacement: `${TABLE_DIR}/` }, { find: /^@controls\//, replacement: `${CONTROLS_DIR}/` }],
    },
    server: {
      fs: { allow: [searchForWorkspaceRoot(process.cwd()), DRAWIO_DIR, PDF_DIR, TABLE_DIR, CONTROLS_DIR, AIWS_DIR] },
      host: '0.0.0.0',
      port: 5174,
      proxy: {
        // listed first: more specific than the zone proxy's /kapi
        ...(aiwsTarget ? { '/kapi/aiworkspace': { target: aiwsTarget, changeOrigin: true } } : {}),
        ...hsProxy,
        ...(nfsTarget ? { '/nfs/v1': { target: nfsTarget, changeOrigin: true }, '/ndm': { target: nfsTarget, changeOrigin: true }, '/bucket/v1': { target: nfsTarget, changeOrigin: true } } : {}),
        ...zoneProxy,
      },
    },
    // `vite preview` serves the production build (with its service worker); the AI Workspace e2e
    // suite uses it with the same backend forward as the dev server.
    preview: {
      proxy: {
        ...(aiwsTarget ? { '/kapi/aiworkspace': { target: aiwsTarget, changeOrigin: true } } : {}),
        ...(nfsTarget ? { '/nfs/v1': { target: nfsTarget, changeOrigin: true }, '/ndm': { target: nfsTarget, changeOrigin: true }, '/bucket/v1': { target: nfsTarget, changeOrigin: true } } : {}),
        ...hsProxy,
      },
    },
  }
})
