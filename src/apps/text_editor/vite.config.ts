import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
export default defineConfig({ plugins: [react()], server: { port: 5178, proxy: { '/nfs/v1': process.env.VITE_NFSP_TARGET ?? 'http://127.0.0.1:3260' } } })
