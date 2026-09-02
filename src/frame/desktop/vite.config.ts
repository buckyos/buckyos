import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// NFSP dev proxy: nfs-server has no CORS (in production the zone gateway
// forwards the same-origin root path /nfs/v1/* to it), so dev mirrors that
// shape by proxying. Point VITE_NFS_PROXY at a running nfs_server (e.g.
// http://127.0.0.1:3260 standalone, or http://127.0.0.1:4110 buckyos mode)
// and open the app with ?fbData=nfsp to run the File Browser on the real
// backend from `pnpm run dev`.
export default defineConfig(({ mode: _mode }) => {
  const nfsTarget = process.env.VITE_NFS_PROXY
  return {
    base: './',
    plugins: [react()],
    server: {
      host: '0.0.0.0',
      port: 5174,
      proxy: nfsTarget
        ? {
            '/nfs/v1': {
              target: nfsTarget,
              changeOrigin: true,
            },
          }
        : undefined,
    },
  }
})
