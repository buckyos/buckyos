import https from 'node:https'
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

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
export default defineConfig(() => {
  const nfsTarget = process.env.VITE_NFS_PROXY
  const zoneTarget = process.env.VITE_ZONE_PROXY
  const zoneIp = process.env.VITE_ZONE_PROXY_IP
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
    plugins: [react()],
    server: {
      host: '0.0.0.0',
      port: 5174,
      proxy: {
        ...(nfsTarget ? { '/nfs/v1': { target: nfsTarget, changeOrigin: true } } : {}),
        ...zoneProxy,
      },
    },
  }
})
