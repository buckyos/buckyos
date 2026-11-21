import { resolve } from 'node:path'
import { defineConfig, searchForWorkspaceRoot } from 'vite'
import react from '@vitejs/plugin-react'
const target = process.env.VITE_NFSP_TARGET ?? 'http://127.0.0.1:3260'
export default defineConfig({ plugins: [react()], server: { fs: { allow: [searchForWorkspaceRoot(process.cwd()), resolve(import.meta.dirname, '../shared')] }, port: 5178, proxy: { '/nfs/v1': target, '/bucket/v1': target, '/kapi/nfs-server': target } } })
