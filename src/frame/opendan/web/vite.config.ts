import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// Point VITE_OPENDAN_PROXY at a running opendan (e.g. http://127.0.0.1:4060)
// to run `pnpm dev` on the real kRPC service; without it dev uses mock data.
export default defineConfig(() => {
  const target = process.env.VITE_OPENDAN_PROXY
  return {
    base: './',
    plugins: [react()],
    server: {
      host: '0.0.0.0',
      port: 5184,
      proxy: target ? { '/kapi/opendan': { target, changeOrigin: true } } : undefined,
    },
  }
})
